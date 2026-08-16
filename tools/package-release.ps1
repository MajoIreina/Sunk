param(
    [Parameter(Mandatory = $true)]
    [string]$DxcRoot,

    [string]$CargoAboutPath = "cargo-about",

    [string]$Version = "0.0.1",

    [string]$OutputDirectory = "artifacts"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot "Cargo.toml"
$manifest = Get-Content -LiteralPath $manifestPath -Raw
$manifestMatch = [regex]::Match($manifest, '(?m)^version = "([^"]+)"$')
if (-not $manifestMatch.Success -or $manifestMatch.Groups[1].Value -ne $Version) {
    throw "Cargo.toml version does not match requested release version $Version."
}

$gitStatus = (& git -C $repositoryRoot status --porcelain=v1 --untracked-files=all) -join "`n"
if ($LASTEXITCODE -ne 0) {
    throw "Unable to inspect the release worktree."
}
if ($gitStatus.Length -ne 0) {
    throw "Release packaging requires a clean Git worktree."
}
$sourceCommit = ((& git -C $repositoryRoot rev-parse HEAD) -join "").Trim()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40}$') {
    throw "Unable to resolve the release source commit."
}

$cargoAboutCommand = (Get-Command -Name $CargoAboutPath -CommandType Application -ErrorAction Stop).Source
$cargoAboutVersion = ((& $cargoAboutCommand --version) -join "").Trim()
if ($LASTEXITCODE -ne 0 -or $cargoAboutVersion -ne "cargo-about 0.9.1") {
    throw "cargo-about 0.9.1 is required to generate release license notices."
}

$binaryPath = Join-Path $repositoryRoot "target\release\sunk.exe"
$compilerPath = Join-Path $DxcRoot "bin\x64\dxcompiler.dll"
$dxilPath = Join-Path $DxcRoot "bin\x64\dxil.dll"
$aboutConfigPath = Join-Path $repositoryRoot "about.toml"
$aboutTemplatePath = Join-Path $repositoryRoot "about.hbs"
$requiredFiles = @(
    $compilerPath,
    $dxilPath,
    $aboutConfigPath,
    $aboutTemplatePath,
    (Join-Path $repositoryRoot "README.md"),
    (Join-Path $repositoryRoot "CHANGELOG.md"),
    (Join-Path $repositoryRoot "LICENSE"),
    (Join-Path $repositoryRoot "THIRD_PARTY_NOTICES.md"),
    (Join-Path $DxcRoot "LICENCE-MIT.txt"),
    (Join-Path $DxcRoot "LICENSE-LLVM.txt"),
    (Join-Path $DxcRoot "LICENSE-MS.txt")
)
foreach ($path in $requiredFiles) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required release file is missing: $path"
    }
}

$expectedDxcHashes = @{
    $compilerPath = "9a5100511e127c6a2fc78edf984f95074a76d35b90c90c4d342430a5ae160e9b"
    $dxilPath = "feb57253eff0a622561e29b44cedbe86b89fc9a5bc8dc00fa2f98fafd712c2d8"
}
foreach ($entry in $expectedDxcHashes.GetEnumerator()) {
    $actualHash = (Get-FileHash -LiteralPath $entry.Key -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $entry.Value) {
        throw "Pinned DXC runtime hash mismatch: $($entry.Key)"
    }
}

$outputRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    [IO.Path]::GetFullPath($OutputDirectory)
} else {
    [IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputDirectory))
}
$bundleName = "Sunk-v$Version-windows-x64"
$stageDirectory = Join-Path $outputRoot $bundleName
$archivePath = Join-Path $outputRoot "$bundleName.zip"
$archiveHashPath = "$archivePath.sha256"
$temporaryLicensePath = Join-Path $outputRoot "$bundleName.third-party-licenses.tmp.html"

foreach ($path in @($stageDirectory, $archivePath, $archiveHashPath, $temporaryLicensePath)) {
    if (Test-Path -LiteralPath $path) {
        throw "Release output already exists: $path"
    }
}

& cargo build --release --locked --manifest-path $manifestPath
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "Locked Release build failed."
}
& (Join-Path $PSScriptRoot "check-binary-size.ps1") -Path $binaryPath

& $cargoAboutCommand generate `
    --config $aboutConfigPath `
    --locked `
    --fail `
    --target x86_64-pc-windows-msvc `
    --manifest-path $manifestPath `
    $aboutTemplatePath `
    -o $temporaryLicensePath
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $temporaryLicensePath -PathType Leaf)) {
    throw "Third-party license generation failed."
}

New-Item -ItemType Directory -Path $stageDirectory -Force | Out-Null
$dxcLicenseDirectory = Join-Path $stageDirectory "licenses\DirectXShaderCompiler"
New-Item -ItemType Directory -Path $dxcLicenseDirectory -Force | Out-Null

Copy-Item -LiteralPath $binaryPath -Destination $stageDirectory
Copy-Item -LiteralPath $compilerPath, $dxilPath -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "README.md") -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "CHANGELOG.md") -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "LICENSE") -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $repositoryRoot "THIRD_PARTY_NOTICES.md") -Destination $stageDirectory
Copy-Item -LiteralPath (Join-Path $DxcRoot "LICENCE-MIT.txt") -Destination $dxcLicenseDirectory
Copy-Item -LiteralPath (Join-Path $DxcRoot "LICENSE-LLVM.txt") -Destination $dxcLicenseDirectory
Copy-Item -LiteralPath (Join-Path $DxcRoot "LICENSE-MS.txt") -Destination $dxcLicenseDirectory
Move-Item -LiteralPath $temporaryLicensePath -Destination (Join-Path $stageDirectory "THIRD-PARTY-LICENSES.html")

$buildInformation = @(
    "Sunk version: $Version",
    "Git commit: $sourceCommit",
    "Target: x86_64-pc-windows-msvc",
    "Rust compiler: $((& rustc --version) -join '')",
    "License generator: $cargoAboutVersion",
    "DXC release: Microsoft DirectXShaderCompiler v1.9.2607"
)
[IO.File]::WriteAllLines((Join-Path $stageDirectory "BUILD-INFO.txt"), $buildInformation)

$manifestLines = Get-ChildItem -LiteralPath $stageDirectory -Recurse -File |
    Sort-Object FullName |
    ForEach-Object {
        $relativePath = [IO.Path]::GetRelativePath($stageDirectory, $_.FullName).Replace('\', '/')
        $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $relativePath"
    }
[IO.File]::WriteAllLines((Join-Path $stageDirectory "SHA256SUMS.txt"), $manifestLines)

Compress-Archive -LiteralPath $stageDirectory -DestinationPath $archivePath -CompressionLevel Optimal
$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($archiveHashPath, "$archiveHash  $bundleName.zip`n")

Get-Item -LiteralPath $archivePath, $archiveHashPath |
    Select-Object FullName, Length
