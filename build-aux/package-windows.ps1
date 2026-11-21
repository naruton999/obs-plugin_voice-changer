[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string] $Configuration = 'Release',

    [string] $Target = 'x86_64-pc-windows-msvc',

    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$ModuleName = 'obs-plugin_voice-changer'
$StageRoot = Join-Path $Root 'dist/windows'
$StageDir = Join-Path $StageRoot $ModuleName
$ZipPath = Join-Path $Root 'dist/obs-plugin_voice-changer-windows-x64.zip'

$ProfileDir = if ($Configuration -eq 'Debug') { 'debug' } else { 'release' }
$CargoProfile = if ($Configuration -eq 'Debug') { 'dev' } else { 'release' }

if (-not $SkipBuild) {
    Push-Location $Root
    try {
        & cargo build -p obs-filter --profile $CargoProfile --target $Target
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$Dll = Join-Path $Root "target/$Target/$ProfileDir/obs_filter.dll"
if (-not (Test-Path $Dll)) {
    throw "DLL not found: $Dll"
}

# Rebuild the staging tree from scratch so no stale files or archives leak in.
if (Test-Path $StageRoot) {
    Remove-Item -Recurse -Force $StageRoot
}
New-Item -ItemType Directory -Force -Path (Join-Path $StageDir 'data/presets') | Out-Null

Copy-Item $Dll (Join-Path $StageDir "$ModuleName.dll")
Copy-Item (Join-Path $Root "presets/*.toml") (Join-Path $StageDir "data/presets/")

# Never append to or nest an existing archive; replace it.
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}

Compress-Archive -Path $StageDir -DestinationPath $ZipPath -CompressionLevel Optimal

Write-Host "created $ZipPath"
