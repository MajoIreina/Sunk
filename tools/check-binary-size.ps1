param(
    [Parameter(Mandatory = $true)]
    [string]$Path,

    [int64]$LimitBytes = 50MB
)

$binary = Get-Item -LiteralPath $Path -ErrorAction Stop
$sizeMiB = [Math]::Round($binary.Length / 1MB, 2)
Write-Host "$($binary.FullName): $sizeMiB MiB (limit: $([Math]::Round($LimitBytes / 1MB, 2)) MiB)"

if ($binary.Length -gt $LimitBytes) {
    throw "Release binary exceeds the configured size limit."
}
