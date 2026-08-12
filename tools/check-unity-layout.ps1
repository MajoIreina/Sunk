[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Fail {
    param([Parameter(Mandatory = $true)][string] $Message)
    throw "Unity repository layout check failed: $Message"
}

function Assert-Condition {
    param(
        [Parameter(Mandatory = $true)][bool] $Condition,
        [Parameter(Mandatory = $true)][string] $Message
    )

    if (-not $Condition) {
        Fail $Message
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$gitCandidates = @()
$gitCommand = Get-Command git -ErrorAction SilentlyContinue
if ($null -ne $gitCommand) {
    $gitCandidates += $gitCommand.Source
}
$programFiles = [Environment]::GetEnvironmentVariable('ProgramFiles')
if ($programFiles) {
    $gitCandidates += Join-Path $programFiles 'Git/cmd/git.exe'
}
$git = $gitCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
    Select-Object -First 1
if (-not $git) {
    Fail 'Git executable was not found.'
}

$repositoryFiles = @(
    & $git -C $repoRoot -c core.quotePath=false ls-files --cached --others --exclude-standard
)
if ($LASTEXITCODE -ne 0) {
    Fail 'Git could not enumerate repository files.'
}
$repositoryFiles = @($repositoryFiles | ForEach-Object { $_.Replace('\', '/') } | Sort-Object -Unique)

$allowedRoots = @(
    '.gitattributes',
    '.github',
    '.gitignore',
    'docs',
    'LICENSE',
    'native',
    'README.md',
    'tools',
    'unity'
)
foreach ($path in $repositoryFiles) {
    $topLevel = $path.Split('/')[0]
    Assert-Condition ($allowedRoots -contains $topLevel) "Unexpected root entry: $path"
}

$requiredFiles = @(
    '.gitattributes',
    '.github/workflows/unity-layout.yml',
    '.gitignore',
    'docs/development/BRANCHING.md',
    'docs/unity/Sunk_Desktop_Unity_Development_Document_v0.2.md',
    'LICENSE',
    'native/macos/README.md',
    'native/windows/README.md',
    'README.md',
    'tools/check-unity-layout.ps1',
    'unity/Sunk/Packages/manifest.json',
    'unity/Sunk/Packages/packages-lock.json',
    'unity/Sunk/ProjectSettings/EditorSettings.asset',
    'unity/Sunk/ProjectSettings/ProjectSettings.asset',
    'unity/Sunk/ProjectSettings/ProjectVersion.txt'
)
foreach ($requiredFile in $requiredFiles) {
    Assert-Condition ($repositoryFiles -contains $requiredFile) "Required file is missing: $requiredFile"
}

$projectVersionFiles = @(
    $repositoryFiles | Where-Object { $_ -match '(^|/)ProjectSettings/ProjectVersion\.txt$' }
)
Assert-Condition ($projectVersionFiles.Count -eq 1) 'Exactly one Unity project must exist.'
Assert-Condition ($projectVersionFiles[0] -eq 'unity/Sunk/ProjectSettings/ProjectVersion.txt') `
    'The only Unity project root must be unity/Sunk.'

foreach ($forbiddenRoot in @('Assets', 'Packages', 'ProjectSettings')) {
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $repoRoot $forbiddenRoot))) `
        "A root-level $forbiddenRoot directory is not allowed."
}

$allowedAssetRoots = @(
    'InputSystem_Actions.inputactions',
    'InputSystem_Actions.inputactions.meta',
    'Scenes',
    'Scenes.meta',
    'Settings',
    'Settings.meta',
    'Sunk',
    'Sunk.meta'
)
$assetPaths = @($repositoryFiles | Where-Object { $_.StartsWith('unity/Sunk/Assets/') })
foreach ($path in $assetPaths) {
    $assetRoot = $path.Substring('unity/Sunk/Assets/'.Length).Split('/')[0]
    Assert-Condition ($allowedAssetRoots -contains $assetRoot) `
        "Product assets must be placed under unity/Sunk/Assets/Sunk: $path"
}

$forbiddenPatterns = @(
    '(^|/)Cargo\.toml$',
    '^Cargo\.lock$',
    '^crates/',
    '^rust-toolchain\.toml$',
    '^rustfmt\.toml$',
    '\.wgsl$',
    '^unity/Sunk/(Library|Temp|Obj|Build|Builds|Logs|UserSettings|MemoryCaptures|Recordings)(/|$)',
    '^artifacts/',
    '^native/(windows|macos)/build/',
    '(^|/)\.(vs|idea|vscode)/',
    '\.(csproj|sln|suo|user|userprefs|pidb|booproj|opendb)$',
    '(^|/)\.git(/|$)'
)
foreach ($path in $repositoryFiles) {
    foreach ($pattern in $forbiddenPatterns) {
        Assert-Condition (-not ($path -match $pattern)) "Forbidden file is present: $path"
    }
}

$projectRoot = Join-Path $repoRoot 'unity/Sunk'
$projectVersionPath = Join-Path $projectRoot 'ProjectSettings/ProjectVersion.txt'
$projectVersion = Get-Content -LiteralPath $projectVersionPath -Raw
Assert-Condition ($projectVersion -match '(?m)^m_EditorVersion: 6000\.0\.30f1\r?$') `
    'ProjectVersion.txt must pin Unity 6000.0.30f1.'
Assert-Condition ($projectVersion -match '(?m)^m_EditorVersionWithRevision: 6000\.0\.30f1 \(62b05ba0686a\)\r?$') `
    'ProjectVersion.txt must pin the expected Unity revision.'

$projectSettings = Get-Content -LiteralPath (Join-Path $projectRoot 'ProjectSettings/ProjectSettings.asset') -Raw
Assert-Condition ($projectSettings -match '(?m)^  productName: Sunk\r?$') `
    'Unity Product Name must be Sunk.'
Assert-Condition (-not ($projectSettings -match 'com\.unity\.template|Unity Technologies')) `
    'Unity template naming remains in ProjectSettings.asset.'

$editorSettings = Get-Content -LiteralPath (Join-Path $projectRoot 'ProjectSettings/EditorSettings.asset') -Raw
Assert-Condition ($editorSettings -match '(?m)^  m_ProjectGenerationRootNamespace: Sunk\r?$') `
    'The C# root namespace must be Sunk.'
Assert-Condition ($editorSettings -match '(?m)^  m_ExternalVersionControlSupport: Visible Meta Files\r?$') `
    'Unity must use visible meta files.'
Assert-Condition ($editorSettings -match '(?m)^  m_SerializationMode: 2\r?$') `
    'Unity assets must use Force Text serialization.'

$manifestPath = Join-Path $projectRoot 'Packages/manifest.json'
$lockPath = Join-Path $projectRoot 'Packages/packages-lock.json'
try {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $packageLock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
} catch {
    Fail "Unity package JSON is invalid: $($_.Exception.Message)"
}
$manifestDependencies = @($manifest.dependencies.PSObject.Properties)
foreach ($dependency in $manifestDependencies) {
    $lockedDependency = $packageLock.dependencies.PSObject.Properties[$dependency.Name]
    Assert-Condition ($null -ne $lockedDependency) `
        "Direct package is missing from packages-lock.json: $($dependency.Name)"
    Assert-Condition ($lockedDependency.Value.version -eq $dependency.Value) `
        "Direct package version differs between manifest and lock: $($dependency.Name)"
    Assert-Condition ($lockedDependency.Value.depth -eq 0) `
        "Direct package must have lock depth 0: $($dependency.Name)"
}
$urpProperty = $manifest.dependencies.PSObject.Properties['com.unity.render-pipelines.universal']
Assert-Condition ($null -ne $urpProperty) 'The URP package is missing from manifest.json.'
Assert-Condition ($urpProperty.Value -eq '17.0.1') 'URP must be pinned to version 17.0.1.'
$lockText = Get-Content -LiteralPath $lockPath -Raw
Assert-Condition (-not ($lockText -match 'com\.unity\.template')) `
    'Template package metadata must not remain in packages-lock.json.'

$assetsRoot = Join-Path $projectRoot 'Assets'
Assert-Condition (Test-Path -LiteralPath $assetsRoot -PathType Container) 'Unity Assets directory is missing.'
$assetItems = @(Get-ChildItem -LiteralPath $assetsRoot -Force -Recurse)
foreach ($item in $assetItems) {
    if ($item.Name.EndsWith('.meta', [System.StringComparison]::OrdinalIgnoreCase)) {
        $assetPath = $item.FullName.Substring(0, $item.FullName.Length - 5)
        Assert-Condition (Test-Path -LiteralPath $assetPath) "Orphan meta file: $($item.FullName)"
    } else {
        Assert-Condition (Test-Path -LiteralPath ($item.FullName + '.meta') -PathType Leaf) `
            "Missing meta file for: $($item.FullName)"
    }
}

$guidOwners = @{}
$metaFiles = @(Get-ChildItem -LiteralPath $assetsRoot -Force -Recurse -Filter '*.meta' -File)
foreach ($metaFile in $metaFiles) {
    $metaText = Get-Content -LiteralPath $metaFile.FullName -Raw
    $guidMatch = [regex]::Match($metaText, '(?m)^guid: ([0-9a-fA-F]{32})\r?$')
    Assert-Condition $guidMatch.Success "Invalid or missing GUID in: $($metaFile.FullName)"
    $guid = $guidMatch.Groups[1].Value.ToLowerInvariant()
    Assert-Condition (-not $guidOwners.ContainsKey($guid)) `
        "Duplicate Unity GUID $guid in $($guidOwners[$guid]) and $($metaFile.FullName)"
    $guidOwners[$guid] = $metaFile.FullName
}

function Test-GitIgnored {
    param([Parameter(Mandatory = $true)][string] $Path)
    & $git -C $repoRoot check-ignore --quiet --no-index -- $Path
    return $LASTEXITCODE -eq 0
}

$ignoredSentinels = @(
    'unity/Sunk/Library/check.bin',
    'unity/Sunk/Temp/check.bin',
    'unity/Sunk/Obj/check.bin',
    'unity/Sunk/Build/check.bin',
    'unity/Sunk/Builds/check.bin',
    'unity/Sunk/Logs/check.log',
    'unity/Sunk/UserSettings/check.asset',
    'unity/Sunk/MemoryCaptures/check.snap',
    'unity/Sunk/Recordings/check.bin',
    'artifacts/check.bin',
    'native/windows/build/check.bin',
    'native/macos/build/check.bin',
    'unity/Sunk/.vs/check.bin',
    'unity/Sunk/.idea/check.bin',
    'unity/Sunk/.vscode/check.bin'
)
foreach ($path in $ignoredSentinels) {
    Assert-Condition (Test-GitIgnored $path) "Generated path is not ignored: $path"
}

$trackedTypeSentinels = @(
    'unity/Sunk/Assets/Sunk/sentinel.meta',
    'unity/Sunk/Assets/Sunk/Plugins/Windows/sentinel.dll',
    'unity/Sunk/Assets/Sunk/Plugins/macOS/sentinel.bundle'
)
foreach ($path in $trackedTypeSentinels) {
    Assert-Condition (-not (Test-GitIgnored $path)) "Required asset type is ignored: $path"
}

Write-Host "Unity repository layout is valid ($($repositoryFiles.Count) files, $($metaFiles.Count) meta files)."
exit 0
