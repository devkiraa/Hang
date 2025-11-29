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
    $build = if ($parsed.Build -lt 0) { 0 } else { $parsed.Build }
    $rev = if ($parsed.Revision -lt 0) { 0 } else { $parsed.Revision }
    return "{0}.{1}.{2}.{3}" -f $parsed.Major, $parsed.Minor, $build, $rev
}

function EscapeXml {
    param([string]$Value)
    return [System.Security.SecurityElement]::Escape($Value)
}

$normalizedVersion = NormalizeVersion -Value $Version
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exePath = Join-Path $root 'target\release\hang-client.exe'
if (-not (Test-Path $exePath)) {
    throw "Client binary not found. Build it with 'cargo build --release -p hang-client' before packaging."
}

$wxsTemplate = Join-Path $PSScriptRoot 'hang-client.wxs'
$wxsGenerated = Join-Path $PSScriptRoot 'hang-client.generated.wxs'
$wixobj = Join-Path $PSScriptRoot 'hang-client.wixobj'
if (Test-Path $wixobj) {
    Remove-Item $wixobj -Force
}
if (Test-Path $wxsGenerated) {
    Remove-Item $wxsGenerated -Force
}

$template = Get-Content $wxsTemplate -Raw
$rendered = $template.Replace('__PRODUCT_VERSION__', $normalizedVersion)
$rendered = $rendered.Replace('__CLIENT_EXE__', (EscapeXml -Value $exePath))
Set-Content -Path $wxsGenerated -Value $rendered -Encoding UTF8

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

try {
    & $candle -arch x64 -o $wixobj $wxsGenerated
    & $light $wixobj -o $outputPath
}
finally {
    if (Test-Path $wxsGenerated) {
        Remove-Item $wxsGenerated -Force
    }
}

Write-Host "Generated MSI: $outputPath"
