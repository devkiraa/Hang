param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$Output,

    [Parameter(Mandatory = $true)]
    [string]$PayloadDir
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
$payloadRoot = Resolve-Path $PayloadDir
$exePath = Join-Path $payloadRoot 'Hang.exe'
if (-not (Test-Path $exePath)) {
    throw "Payload executable not found at $exePath"
}

$runtimeDir = Join-Path $payloadRoot 'runtime'
if (-not (Test-Path $runtimeDir)) {
    throw "Runtime directory not found at $runtimeDir"
}

$wxsTemplate = Join-Path $PSScriptRoot 'hang-client.wxs'
$wxsGenerated = Join-Path $PSScriptRoot 'hang-client.generated.wxs'
$heatFragment = Join-Path $PSScriptRoot 'vlc-runtime.generated.wxs'
$mainObj = Join-Path $PSScriptRoot 'hang-client.wixobj'
$runtimeObj = Join-Path $PSScriptRoot 'vlc-runtime.wixobj'
if (Test-Path $mainObj) {
    Remove-Item $mainObj -Force
}
if (Test-Path $runtimeObj) {
    Remove-Item $runtimeObj -Force
}
if (Test-Path $wxsGenerated) {
    Remove-Item $wxsGenerated -Force
}
if (Test-Path $heatFragment) {
    Remove-Item $heatFragment -Force
}

$template = Get-Content $wxsTemplate -Raw
$rendered = $template.Replace('__PRODUCT_VERSION__', $normalizedVersion)
$rendered = $rendered.Replace('__CLIENT_EXE__', (EscapeXml -Value $exePath))
$rendered = $rendered.Replace('__PAYLOAD_DIR__', (EscapeXml -Value $payloadRoot))
$licensePath = (Resolve-Path (Join-Path $PSScriptRoot 'license.rtf')).ProviderPath
$rendered = $rendered.Replace('__LICENSE_RTF__', (EscapeXml -Value $licensePath))
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
$heat = (Get-Command heat.exe).Path

& $heat dir $runtimeDir -cg VlcRuntimeGroup -dr VlcInstallDir -srd -gg -var var.RuntimeSourceDir -out $heatFragment

try {
    $runtimeDefine = "-dRuntimeSourceDir=$runtimeDir"
    & $candle -arch x64 $runtimeDefine -out $mainObj $wxsGenerated
    & $candle -arch x64 $runtimeDefine -out $runtimeObj $heatFragment
    & $light $runtimeDefine $mainObj $runtimeObj -o $outputPath
}
finally {
    if (Test-Path $wxsGenerated) {
        Remove-Item $wxsGenerated -Force
    }
    if (Test-Path $heatFragment) {
        Remove-Item $heatFragment -Force
    }
    if (Test-Path $mainObj) {
        Remove-Item $mainObj -Force
    }
    if (Test-Path $runtimeObj) {
        Remove-Item $runtimeObj -Force
    }
}

Write-Host "Generated MSI: $outputPath"
