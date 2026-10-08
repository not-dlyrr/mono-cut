# SPDX-License-Identifier: GPL-3.0-or-later
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$resourceDir = Join-Path $projectRoot 'src-tauri/resources/media'
$ffmpeg = Join-Path $resourceDir 'ffmpeg.exe'
$ffprobe = Join-Path $resourceDir 'ffprobe.exe'
$spec = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'media-sources.json') -Raw | ConvertFrom-Json
foreach ($name in @('ffmpeg.exe','ffprobe.exe','Inter.ttf')) {
    if (-not (Test-Path -LiteralPath (Join-Path $resourceDir $name))) { throw "Media resource missing: $name" }
}
function Read-Tool([string]$tool, [string[]]$Arguments) {
    $lines = & $tool @Arguments 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'Bundled media inspection failed.' }
    return ($lines -join "`n").Trim()
}
$version = Read-Tool $ffmpeg @('-version')
$probeVersion = Read-Tool $ffprobe @('-version')
if ($version -match '--enable-nonfree|C:[/\\]Users[/\\]|/home/|/Users/') { throw 'Binary configuration contains nonfree flags or a private build path.' }
foreach ($required in @('--enable-gpl','--enable-version3','--disable-autodetect','--disable-network','--disable-hwaccels','--enable-libx264','--enable-libfreetype','--enable-libharfbuzz','--enable-zlib')) {
    if (-not $version.Contains($required)) { throw "Bundled FFmpeg configuration lacks $required" }
}
$filters = Read-Tool $ffmpeg @('-hide_banner','-filters')
$encoders = Read-Tool $ffmpeg @('-hide_banner','-encoders')
foreach ($required in @('drawtext','overlay','amix','afade','fade','scale','eq','crop','trim','atrim','volume','aresample','setpts','asetpts','fps')) {
    if ($filters -notmatch "(?m)^\s*[TSC.]+\s+$required\s") { throw "Bundled FFmpeg lacks filter $required" }
}
foreach ($required in @('libx264','aac','ffv1','flac','png','pcm_s16le')) {
    if ($encoders -notmatch "(?m)^\s*[VAS.FSXBD]+\s+$required\s") { throw "Bundled FFmpeg lacks encoder $required" }
}
$files = @('ffmpeg.exe','ffprobe.exe','Inter.ttf') | ForEach-Object {
    $item = Get-Item -LiteralPath (Join-Path $resourceDir $_)
    @{ name = $_; bytes = $item.Length; sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$record = [ordered]@{
    schemaVersion = 1
    target = $spec.target
    license = 'GPL-3.0-or-later'
    externalLibraries = @('x264','FreeType','HarfBuzz','zlib')
    sourceInputs = $spec.sources
    files = $files
    ffmpegVersion = $version
    ffprobeVersion = $probeVersion
    filters = $filters
    encoders = $encoders
    hardwareAcceleration = Read-Tool $ffmpeg @('-hide_banner','-hwaccels')
    notes = 'Windows x64 static software build. Build paths in the descriptive configuration map to /build/mono-cut-media. Byte-identical reproducibility is not claimed.'
}
$path = Join-Path $projectRoot 'licenses/media-provenance.json'
$record | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path -Encoding utf8NoBOM
Copy-Item -LiteralPath $path -Destination (Join-Path $resourceDir 'provenance.json')
Write-Host 'Bundled media features and hashes recorded.'
