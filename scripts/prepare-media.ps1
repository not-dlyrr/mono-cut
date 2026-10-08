# SPDX-License-Identifier: GPL-3.0-or-later
[CmdletBinding()]
param(
    [string]$BuildDirectory,
    [int]$Jobs = 6,
    [switch]$ForceBuild,
    [switch]$SourcesOnly,
    [string]$SourceBundle
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $BuildDirectory) { $BuildDirectory = Join-Path $projectRoot 'work/media-build' }
$BuildDirectory = [System.IO.Path]::GetFullPath($BuildDirectory)
if ($Jobs -lt 1 -or $Jobs -gt 32) { throw 'Jobs must be between 1 and 32.' }
$spec = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'media-sources.json') -Raw | ConvertFrom-Json
$resourceDir = Join-Path $projectRoot 'src-tauri/resources/media'
New-Item -ItemType Directory -Path $BuildDirectory,$resourceDir -Force | Out-Null

function Fetch-Verified($entry) {
    $destination = Join-Path $BuildDirectory $entry.file
    if (-not (Test-Path -LiteralPath $destination)) {
        Write-Host "Downloading $($entry.name) $($entry.version)..."
        Invoke-WebRequest -Uri $entry.url -OutFile $destination
    }
    $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $entry.sha256) { throw "SHA-256 mismatch: $($entry.file). Delete this download and retry." }
    return $destination
}

foreach ($entry in $spec.sources) { Fetch-Verified $entry | Out-Null }
if ($SourceBundle) {
    $bundlePath = [System.IO.Path]::GetFullPath($SourceBundle)
    New-Item -ItemType Directory -Path (Split-Path -Parent $bundlePath) -Force | Out-Null
    $bundleInputs = @($spec.sources | ForEach-Object { Join-Path $BuildDirectory $_.file })
    $bundleInputs += Join-Path $PSScriptRoot 'build-media.sh'
    $bundleInputs += Join-Path $PSScriptRoot 'prepare-media.ps1'
    $bundleInputs += Join-Path $PSScriptRoot 'record-media-provenance.ps1'
    $bundleInputs += Join-Path $PSScriptRoot 'media-sources.json'
    $mediaNotices = @('FFmpeg-GPL-3.0.txt','FFmpeg-license.md','FreeType.txt','HarfBuzz.txt','Inter-OFL-1.1.txt','MinGW-w64-runtime.txt','x264-GPL-2.0.txt','zlib.txt','GCC-Runtime-Exception.txt','media-provenance.json')
    $bundleInputs += $mediaNotices | ForEach-Object { Join-Path $projectRoot "licenses/$_" } | Where-Object { Test-Path -LiteralPath $_ }
    $bundleReadme = Join-Path $BuildDirectory 'README-MEDIA-SOURCE.txt'
    @'
This archive carries the exact source inputs for the bundled media binaries.
It also contains the Inter font release and preferred glyph source, verified
input hashes, build recipes, license notices and recorded binary provenance.

First extract the Mono Cut application source archive. Extract this archive
into a directory you will use for media compilation, for example media-build.
From the application source root run:

pwsh -File scripts/prepare-media.ps1 -BuildDirectory /path/to/media-build -ForceBuild

The application scripts verify the cached archives before extracting/building.
The copies of scripts in this archive document the exact release recipe; use
the scripts/ folder in the application source to preserve relative layout.

The recipe targets Windows x64 and needs PowerShell 7, Git for Windows, and
7-Zip. Additional open compiler/assembler tools are fetched using the hashes
and upstream source URLs in media-sources.json. Build tools are not bundled
inside the editor. Windows operating-system libraries are platform components.
Linux/macOS use a separately configured and audited open FFmpeg build.

Provenance records include hashes of the release binaries, actual enabled
configuration, filters and encoders. Source reproducibility is provided; a
byte-identical binary rebuild is not promised. Retain corresponding source
beside any redistributed binary release.
'@ | Set-Content -LiteralPath $bundleReadme -Encoding utf8NoBOM
    $bundleInputs += $bundleReadme
    Compress-Archive -LiteralPath $bundleInputs -DestinationPath $bundlePath -Force
    Write-Host "Corresponding media source: $bundlePath"
}
if ($SourcesOnly) { return }

$provenancePath = Join-Path $projectRoot 'licenses/media-provenance.json'
if (-not $ForceBuild -and (Test-Path -LiteralPath $provenancePath)) {
    $provenance = Get-Content -LiteralPath $provenancePath -Raw | ConvertFrom-Json
    $ready = $true
    foreach ($file in $provenance.files) {
        $candidate = Join-Path $resourceDir $file.name
        if (-not (Test-Path -LiteralPath $candidate) -or (Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash.ToLowerInvariant() -ne $file.sha256) { $ready = $false }
    }
    if ($ready) { Write-Host 'Media resources match the recorded SHA-256 checksums.'; return }
}
if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { throw 'The pinned media recipe targets Windows x64. See docs/media-components.md for Unix development.' }
$gitCommand = Get-Command git -ErrorAction Stop
$gitBin = Join-Path (Split-Path -Parent (Split-Path -Parent $gitCommand.Source)) 'bin'
if (-not (Test-Path -LiteralPath (Join-Path $gitBin 'bash.exe'))) { throw 'Install Git for Windows with Git Bash before building x264.' }
$sevenZip = Get-Command 7z -ErrorAction Stop
foreach ($entry in $spec.buildTools) { Fetch-Verified $entry | Out-Null }
if (-not (Test-Path -LiteralPath (Join-Path $BuildDirectory 'w64devkit/bin/gcc.exe'))) {
    & $sevenZip.Source x (Join-Path $BuildDirectory 'w64devkit.7z.exe') "-o$BuildDirectory" -y
    if ($LASTEXITCODE -ne 0) { throw 'w64devkit extraction failed.' }
}
foreach ($archive in @('nasm-2.16.03.zip','Inter-4.1.zip')) {
    $destination = if ($archive -eq 'Inter-4.1.zip') { Join-Path $BuildDirectory 'inter' } else { $BuildDirectory }
    & $sevenZip.Source x (Join-Path $BuildDirectory $archive) "-o$destination" -y
    if ($LASTEXITCODE -ne 0) { throw "Extraction failed: $archive" }
}
$originalPath = $env:PATH
try {
    $env:PATH = (Join-Path $BuildDirectory 'w64devkit/bin') + ';' + (Join-Path $BuildDirectory 'nasm-2.16.03') + ';' + $gitBin + ';' + $originalPath
    foreach ($entry in $spec.sources | Where-Object { $_.file -match '\.tar\.' }) {
        & tar xf (Join-Path $BuildDirectory $entry.file) -C $BuildDirectory '--exclude=*/test/*'
        if ($LASTEXITCODE -ne 0) { throw "Source extraction failed: $($entry.file)" }
    }
    $env:MEDIA_BUILD_ROOT = $BuildDirectory.Replace('\','/')
    $env:MEDIA_BUILD_JOBS = "$Jobs"
    $env:MEDIA_BUILD_SHELL = (Join-Path $BuildDirectory 'w64devkit/bin/sh.exe').Replace('\','/')
    & (Join-Path $BuildDirectory 'w64devkit/bin/sh.exe') (Join-Path $PSScriptRoot 'build-media.sh').Replace('\','/')
    if ($LASTEXITCODE -ne 0) { throw 'Media compilation failed.' }
    foreach ($binary in @('ffmpeg.exe','ffprobe.exe')) {
        Copy-Item -LiteralPath (Join-Path $BuildDirectory "prefix/bin/$binary") -Destination (Join-Path $resourceDir $binary)
    }
    Copy-Item -LiteralPath (Join-Path $BuildDirectory 'inter/extras/ttf/Inter-Regular.ttf') -Destination (Join-Path $resourceDir 'Inter.ttf')
    & (Join-Path $PSScriptRoot 'record-media-provenance.ps1')
    Write-Host 'Pinned software media engine compiled and installed.'
} finally {
    $env:PATH = $originalPath
    Remove-Item Env:MEDIA_BUILD_ROOT,Env:MEDIA_BUILD_JOBS,Env:MEDIA_BUILD_SHELL -ErrorAction SilentlyContinue
}
