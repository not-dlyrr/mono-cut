# SPDX-License-Identifier: GPL-3.0-or-later
# Packages a reviewed, committed build. It never uploads or publishes a release.
# Run with PowerShell 7; expected hashes come from the reviewed build/source inputs.
[CmdletBinding()]
param(
    [ValidateSet('0.1.0', '0.1.1')][string]$Version = '0.1.0',
    [string]$Tag = 'v0.1.0',
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$InstallerSha256,
    [Parameter(Mandatory)][string]$MediaSourcePath,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$MediaSourceSha256,
    [Parameter(Mandatory)][string]$DependencySourcePath,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$DependencySourceSha256,
    [string]$MediaProvenancePath,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'Release packaging requires PowerShell 7.' }
$projectRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
if (-not $MediaProvenancePath) { $MediaProvenancePath = Join-Path $projectRoot 'licenses/media-provenance.json' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot "work/release/$Tag" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if ($Tag -cne "v$Version") { throw 'The release tag must match the application version.' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Output directory already exists. Use a new directory; existing releases are never overwritten.' }

function Read-Command([string]$Command, [string[]]$Arguments) {
    $lines = & $Command @Arguments 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw "Required build tool failed: $Command" }
    return ($lines -join "`n").Trim()
}
function Hash-File([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Require-Input([string]$Path, [string]$Expected, [string]$Label) {
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or $item.Length -eq 0 -or $item.LinkType) { throw "$Label must be a nonempty regular file." }
    if ((Hash-File $item.FullName) -cne $Expected.ToLowerInvariant()) { throw "$Label SHA-256 mismatch." }
    return $item.FullName
}
function Open-SafeZip([string]$Path) {
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $expanded = 0L
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            if ($name.StartsWith('/') -or $name.Contains(':') -or $name -match '(^|/)\.\.(/|$)' -or $name.Contains([char]0) -or -not $seen.Add($name)) {
                throw 'Source ZIP contains an unsafe or duplicate member path.'
            }
            $expanded += $entry.Length
            if ($entry.Length -gt 512MB -or $expanded -gt 2GB -or $zip.Entries.Count -gt 100000) { throw 'Source ZIP exceeds packaging validation limits.' }
        }
        return ,$zip
    } catch { $zip.Dispose(); throw }
}
function Zip-Entry($Zip, [string]$Name) {
    $entry = $Zip.GetEntry($Name)
    if (-not $entry -or $entry.Length -eq 0) { throw "Corresponding source is missing: $Name" }
    return $entry
}
function Hash-Entry($Zip, [string]$Name) {
    $stream = (Zip-Entry $Zip $Name).Open()
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToHexString($sha.ComputeHash($stream)).ToLowerInvariant() }
    finally { $stream.Dispose(); $sha.Dispose() }
}
function Read-ZipJson($Zip, [string]$Name) {
    $entry = Zip-Entry $Zip $Name
    if ($entry.Length -gt 16MB) { throw 'Source inventory is too large.' }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { return $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable }
    finally { $reader.Dispose() }
}
function Require-ZipHash($Zip, [string]$Name, [string]$Expected) {
    if ((Hash-Entry $Zip $Name) -cne $Expected.ToLowerInvariant()) { throw "Corresponding source hash mismatch: $Name" }
}

Push-Location $projectRoot
$staging = $null
try {
    $commit = Read-Command git @('rev-parse', '--verify', 'HEAD^{commit}')
    if ($commit -notmatch '^[a-f0-9]{40,64}$') { throw 'A committed source checkout is required.' }
    if (Read-Command git @('status', '--porcelain', '--untracked-files=all')) { throw 'Commit all application changes before packaging its corresponding source.' }
    $tagCommit = Read-Command git @('rev-parse', '--verify', "refs/tags/$Tag`^{commit}")
    if ($tagCommit -cne $commit) { throw 'The version tag must identify the exact committed checkout used for this release.' }
    $package = Get-Content -LiteralPath 'package.json' -Raw | ConvertFrom-Json
    $tauri = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json
    $cargo = Get-Content -LiteralPath 'src-tauri/Cargo.toml' -Raw
    $cargoPackage = [regex]::Match($cargo, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
    $cargoVersions = [regex]::Matches($cargoPackage.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if ($package.version -cne $Version -or $tauri.version -cne $Version -or -not $cargoPackage.Success -or $cargoVersions.Count -ne 1 -or $cargoVersions[0].Groups[1].Value -cne $Version) {
        throw 'Application, desktop shell and native engine versions must match the release.'
    }
    $installer = Require-Input $InstallerPath $InstallerSha256 'Windows installer'
    $mediaSource = Require-Input $MediaSourcePath $MediaSourceSha256 'Media source ZIP'
    $dependencySource = Require-Input $DependencySourcePath $DependencySourceSha256 'Dependency source ZIP'
    $reader = [IO.File]::OpenRead($installer)
    try {
        if ([IO.Path]::GetExtension($installer) -cne '.exe' -or $reader.Length -lt 8192 -or $reader.ReadByte() -ne 77 -or $reader.ReadByte() -ne 90) { throw 'Installer must be a Windows executable.' }
    } finally { $reader.Dispose() }

    # Verify actual bundled binaries against the reviewed open media provenance.
    $provenance = Get-Content -LiteralPath $MediaProvenancePath -Raw | ConvertFrom-Json -AsHashtable
    $mediaSpec = Get-Content -LiteralPath 'scripts/media-sources.json' -Raw | ConvertFrom-Json -AsHashtable
    $extra = Get-Content -LiteralPath 'scripts/extra-source-inputs.json' -Raw | ConvertFrom-Json -AsHashtable
    foreach ($name in @('ffmpeg.exe', 'ffprobe.exe', 'Inter.ttf')) {
        $record = @($provenance.files | Where-Object name -CEQ $name)
        if ($record.Count -ne 1) { throw 'Media provenance must identify every bundled component exactly once.' }
        Require-Input (Join-Path 'src-tauri/resources/media' $name) $record[0].sha256 $name | Out-Null
    }
    $mediaZip = Open-SafeZip $mediaSource
    try {
        foreach ($source in $mediaSpec.sources) { Require-ZipHash $mediaZip $source.file $source.sha256 }
        foreach ($name in @('build-media.sh', 'prepare-media.ps1', 'record-media-provenance.ps1', 'media-sources.json')) {
            Require-ZipHash $mediaZip $name (Hash-File (Join-Path 'scripts' $name))
        }
        Require-ZipHash $mediaZip 'media-provenance.json' (Hash-File $MediaProvenancePath)
        Zip-Entry $mediaZip 'README-MEDIA-SOURCE.txt' | Out-Null
    } finally { $mediaZip.Dispose() }
    $dependencyZip = Open-SafeZip $dependencySource
    try {
        Require-ZipHash $dependencyZip 'Cargo.lock' (Hash-File 'src-tauri/Cargo.lock')
        Require-ZipHash $dependencyZip 'package-lock.json' (Hash-File 'package-lock.json')
        Zip-Entry $dependencyZip '.cargo/config.toml' | Out-Null
        Zip-Entry $dependencyZip 'README-SOURCE.txt' | Out-Null
        foreach ($source in $extra.javascriptUpstreams) { Require-ZipHash $dependencyZip "javascript/$($source.file)" $source.sha256 }
        foreach ($source in $extra.installer) { Require-ZipHash $dependencyZip "installer/$($source.file)" $source.sha256 }
        $javascript = @(Read-ZipJson $dependencyZip 'javascript/source-inputs.json')
        $lock = Get-Content -LiteralPath 'package-lock.json' -Raw | ConvertFrom-Json -AsHashtable
        $runtimeCount = 0
        foreach ($location in $lock.packages.Keys) {
            $locked = $lock.packages[$location]
            if (-not $location -or ($locked.ContainsKey('dev') -and $locked.dev)) { continue }
            $runtimeCount++
            $name = ($location -split 'node_modules/')[-1]
            $record = @($javascript | Where-Object { $_.name -ceq $name -and $_.version -ceq $locked.version -and $_.integrity -ceq $locked.integrity })
            if ($record.Count -ne 1) { throw 'JavaScript source inventory differs from the committed runtime lockfile.' }
            Require-ZipHash $dependencyZip "javascript/$($record[0].file)" $record[0].sha256
        }
        if ($javascript.Count -ne $runtimeCount) { throw 'JavaScript source inventory contains unexpected runtime packages.' }
        foreach ($record in @(Read-ZipJson $dependencyZip 'native-source-input-inventory.json')) { Require-ZipHash $dependencyZip $record.file $record.sha256 }
        foreach ($entry in $dependencyZip.Entries) {
            if ($entry.FullName -match '(?i)(^|/)WebView2Loader[^/]*\.(dll|lib)$') { throw 'Dependency source contains the proprietary prebuilt WebView2 loader.' }
        }
    } finally { $dependencyZip.Dispose() }

    # Output names never retain input paths or personal build directories.
    $parent = Split-Path -Parent $OutputDirectory
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $staging = Join-Path $parent ('.mono-cut-release-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $staging | Out-Null
    $sourceName = "mono-cut-source-$Version.zip"
    Read-Command git @('archive', '--format=zip', "--prefix=mono-cut-$Version/", "--output=$(Join-Path $staging $sourceName)", $commit) | Out-Null
    $artifacts = [Collections.Generic.List[object]]::new()
    foreach ($input in @(
        @{ file = "mono-cut-$Version-windows-x64-setup.exe"; path = $installer; hash = $InstallerSha256 },
        @{ file = "mono-cut-media-source-$Version.zip"; path = $mediaSource; hash = $MediaSourceSha256 },
        @{ file = "mono-cut-source-dependencies-$Version.zip"; path = $dependencySource; hash = $DependencySourceSha256 }
    )) {
        $target = Join-Path $staging $input.file
        Copy-Item -LiteralPath $input.path -Destination $target
        if ((Hash-File $target) -cne $input.hash.ToLowerInvariant()) { throw 'An input changed while release artifacts were copied.' }
    }
    foreach ($item in @(Get-ChildItem -LiteralPath $staging -File | Sort-Object Name)) {
        $artifacts.Add([ordered]@{ file = $item.Name; bytes = $item.Length; sha256 = Hash-File $item.FullName })
    }
    $tools = [ordered]@{
        powershell = $PSVersionTable.PSVersion.ToString()
        git = Read-Command git @('--version')
        rustc = Read-Command rustc @('--version')
        cargo = Read-Command cargo @('--version')
        node = Read-Command node @('--version')
        npm = Read-Command npm @('--version')
        ffmpeg = ($provenance.ffmpegVersion -split '\r?\n')[0]
        ffprobe = ($provenance.ffprobeVersion -split '\r?\n')[0]
    }
    $info = [ordered]@{
        schemaVersion = 1
        application = 'Mono Cut'
        version = $Version
        tag = $Tag
        commit = $commit
        target = 'x86_64-pc-windows-msvc'
        license = 'GPL-3.0-or-later'
        sourceArchiveMethod = 'git archive of the exact tagged commit; no working-tree files'
        packagedUtc = [DateTime]::UtcNow.ToString('o')
        tools = $tools
        bundledMedia = [ordered]@{
            provenanceArchiveMember = 'media-provenance.json'
            provenanceSha256 = Hash-File $MediaProvenancePath
            committedReferenceSha256 = Hash-File 'licenses/media-provenance.json'
            target = $provenance.target
            sourceInputs = $provenance.sourceInputs
            files = $provenance.files
            ffmpegVersion = $provenance.ffmpegVersion
            ffprobeVersion = $provenance.ffprobeVersion
            hardwareAcceleration = $provenance.hardwareAcceleration
            notes = $provenance.notes
        }
        artifacts = $artifacts.ToArray()
        notes = 'Checksums identify these reviewed inputs and packaged outputs. Source/build recipes are supplied; byte-identical rebuilds and a signed installer are not claimed.'
    }
    $json = $info | ConvertTo-Json -Depth 12
    if ($json -match '(?i)(?:^|["\s])[A-Za-z]:[/\\]|/Users/|/home/|\\\\[^\\]') { throw 'Build information contains an absolute/private path.' }
    [IO.File]::WriteAllText((Join-Path $staging 'BUILD-INFO.json'), $json + "`n", [Text.UTF8Encoding]::new($false))
    $checksums = @(Get-ChildItem -LiteralPath $staging -File | Sort-Object Name | ForEach-Object { "$(Hash-File $_.FullName)  $($_.Name)" })
    [IO.File]::WriteAllText((Join-Path $staging 'SHA256SUMS.txt'), ($checksums -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    foreach ($item in Get-ChildItem -LiteralPath $staging -File) {
        if ($item.Name -ne 'SHA256SUMS.txt' -and $checksums -cnotcontains "$(Hash-File $item.FullName)  $($item.Name)") { throw 'Final release checksum verification failed.' }
    }
    # Directory.Move fails if another packager created the destination meanwhile;
    # Move-Item would instead nest our staging directory inside that release.
    [IO.Directory]::Move($staging, $OutputDirectory)
    $staging = $null
    Write-Host "Packaged Mono Cut $Version from commit $commit. Six verified release files are ready; nothing was published."
} finally {
    Pop-Location
    if ($staging -and (Test-Path -LiteralPath $staging)) {
        $resolvedStage = [IO.Path]::GetFullPath($staging)
        $resolvedParent = [IO.Path]::GetFullPath((Split-Path -Parent $OutputDirectory)) + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedStage.StartsWith($resolvedParent, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path -Leaf $resolvedStage) -notlike '.mono-cut-release-*') { throw 'Refusing cleanup outside the release staging directory.' }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
}
