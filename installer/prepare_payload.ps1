param(
    [Parameter(Mandatory = $true)]
    [string]$PayloadDir,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$VlcVersion = "3.0.20",

    [switch]$MinimalRuntime
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exeSource = Join-Path $root 'target\release\hang-client.exe'
if (-not (Test-Path $exeSource)) {
    throw "Client binary not found at $exeSource. Run 'cargo build --release -p hang-client' first."
}

if (Test-Path $PayloadDir) {
    Remove-Item $PayloadDir -Recurse -Force
}
New-Item -ItemType Directory -Path $PayloadDir | Out-Null

$payloadExe = Join-Path $PayloadDir 'Hang.exe'
Copy-Item -Path $exeSource -Destination $payloadExe -Force

$cacheDir = Join-Path $root 'export\cache'
if (-not (Test-Path $cacheDir)) {
    New-Item -ItemType Directory -Path $cacheDir | Out-Null
}

$zipName = "vlc-$VlcVersion-win64.zip"
$zipPath = Join-Path $cacheDir $zipName
$vlcUrl = "https://downloads.videolan.org/pub/videolan/vlc/$VlcVersion/win64/$zipName"
if (-not (Test-Path $zipPath)) {
    Write-Host "Downloading VLC runtime $VlcVersion..."
    Invoke-WebRequest -Uri $vlcUrl -OutFile $zipPath -UseBasicParsing
}

$extractDir = Join-Path $cacheDir "vlc-$VlcVersion"
if (Test-Path $extractDir) {
    Remove-Item $extractDir -Recurse -Force
}
Expand-Archive -Path $zipPath -DestinationPath $extractDir

$vlcRoot = Get-ChildItem -Path $extractDir | Where-Object { $_.PSIsContainer } | Select-Object -First 1
if (-not $vlcRoot) {
    throw "Failed to locate extracted VLC root in $extractDir"
}

$runtimeDir = Join-Path $PayloadDir 'runtime'
New-Item -ItemType Directory -Path $runtimeDir | Out-Null

$itemsToCopy = @('libvlc.dll', 'libvlccore.dll', 'plugins', 'locale', 'resources', 'lua')
foreach ($item in $itemsToCopy) {
    $sourcePath = Join-Path $vlcRoot.FullName $item
    if (Test-Path $sourcePath) {
        Copy-Item -Path $sourcePath -Destination (Join-Path $runtimeDir $item) -Recurse -Force
    }
}

$licenseFiles = @('COPYING.txt', 'COPYING.LIB')
foreach ($license in $licenseFiles) {
    $licensePath = Join-Path $vlcRoot.FullName $license
    if (Test-Path $licensePath) {
        Copy-Item -Path $licensePath -Destination (Join-Path $runtimeDir $license) -Force
    }
}

if ($MinimalRuntime) {
    Write-Host "Pruning optional VLC assets for smaller payload..."

    $localeDir = Join-Path $runtimeDir 'locale'
    if (Test-Path $localeDir) {
        Remove-Item -Path $localeDir -Recurse -Force
    }

    $pluginDir = Join-Path $runtimeDir 'plugins'
    if (Test-Path $pluginDir) {
        $prunePluginDirs = @(
            'access_output',
            'control',
            'gui',
            'logger',
            'misc',
            'services_discovery',
            'stream_extractor',
            'stream_filter',
            'stream_out',
            'video_splitter',
            'visualization'
        )

        foreach ($dir in $prunePluginDirs) {
            $target = Join-Path $pluginDir $dir
            if (Test-Path $target) {
                Remove-Item -Path $target -Recurse -Force
            }
        }
    }
}

Write-Host "Payload prepared in $PayloadDir"
