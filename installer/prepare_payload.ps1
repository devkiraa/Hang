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

Add-Type -AssemblyName System.Net.Http

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

function Invoke-FileDownload {
    param(
        [string]$Url,
        [string]$Destination,
        [string]$Label
    )

    $tempPath = "$Destination.partial"
    if (Test-Path $tempPath) {
        Remove-Item $tempPath -Force
    }
    if (Test-Path $Destination) {
        Remove-Item $Destination -Force
    }

    $client = [System.Net.Http.HttpClient]::new()
    try {
        $response = $client.GetAsync($Url, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).Result
        if (-not $response.IsSuccessStatusCode) {
            throw "Failed to download ${Label}: $($response.StatusCode)"
        }
        $total = $response.Content.Headers.ContentLength
        $input = $response.Content.ReadAsStreamAsync().Result
        $output = [System.IO.File]::Open($tempPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
        try {
            $buffer = New-Object byte[] (256 * 1024)
            $totalRead = 0L
            while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $output.Write($buffer, 0, $read)
                $totalRead += $read
                if ($total) {
                    $percent = [int](([double]$totalRead / [double]$total) * 100)
                    $status = "{0:N1} MB / {1:N1} MB" -f ($totalRead / 1MB), ($total / 1MB)
                } else {
                    $percent = 0
                    $status = "{0:N1} MB downloaded" -f ($totalRead / 1MB)
                }
                Write-Progress -Activity "Downloading $Label" -Status $status -PercentComplete $percent
            }
        }
        finally {
            $output.Dispose()
            $input.Dispose()
        }
    }
    finally {
        $client.Dispose()
    }

    Move-Item -Path $tempPath -Destination $Destination -Force
    Write-Progress -Activity "Downloading $Label" -Completed
}

function Get-VlcArchive {
    param(
        [string]$Url,
        [string]$Destination
    )

    Invoke-FileDownload -Url $Url -Destination $Destination -Label "VLC runtime $VlcVersion"
}

if (-not (Test-Path $zipPath)) {
    Get-VlcArchive -Url $vlcUrl -Destination $zipPath
}

$extractDir = Join-Path $cacheDir "vlc-$VlcVersion"
if (Test-Path $extractDir) {
    Remove-Item $extractDir -Recurse -Force
}

try {
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -ErrorAction Stop
}
catch {
    Write-Warning "Cached VLC archive appears to be corrupt. Re-downloading..."
    Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
    Get-VlcArchive -Url $vlcUrl -Destination $zipPath
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -ErrorAction Stop
}

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
