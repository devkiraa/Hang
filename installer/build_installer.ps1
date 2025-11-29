param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$Output
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function NormalizeVersion {
    param([string]$Value)
    $trimmed = $Value.Trim()
    if ($trimmed.StartsWith('v', 'OrdinalIgnoreCase')) {
        $trimmed = $trimmed.Substring(1)
    }
    $parsed = $null
    if (-not [System.Version]::TryParse($trimmed, [ref]$parsed)) {
        throw "Version '$Value' must be numeric (e.g. 1.0.0)"
    }
    return $parsed.ToString()
}

$normalizedVersion = NormalizeVersion -Value $Version
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exePath = Join-Path $root 'target\release\hang-client.exe'
if (-not (Test-Path $exePath)) {
    throw "Client binary not found. Build it with 'cargo build --release -p hang-client' before packaging."
}

$wxs = Join-Path $PSScriptRoot 'hang-client.wxs'
$wixobj = Join-Path $PSScriptRoot 'hang-client.wixobj'
if (Test-Path $wixobj) {
    Remove-Item $wixobj -Force
}

if ([System.IO.Path]::IsPathRooted($Output)) {
    $outputPath = $Output
} else {
    $outputPath = Join-Path $root $Output
}
$outputDirectory = Split-Path $outputPath -Parent
if (-not (Test-Path $outputDirectory)) {
    New-Item -ItemType Directory -Path $outputDirectory | Out-Null
}

$candle = (Get-Command candle.exe).Path
$light = (Get-Command light.exe).Path

& $candle -arch x64 -dProductVersion=$normalizedVersion -dClientExePath=$exePath -o $wixobj $wxs
& $light $wixobj -o $outputPath

Write-Host "Generated MSI: $outputPath"
