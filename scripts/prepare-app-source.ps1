# SPDX-License-Identifier: GPL-3.0-or-later
[CmdletBinding()]
param([string]$Destination, [string]$Archive)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Destination) { $Destination = Join-Path $projectRoot 'work/app-source' }
$Destination = [System.IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Path $Destination -Force | Out-Null
Push-Location $projectRoot
try {
    cargo vendor --locked --manifest-path src-tauri/Cargo.toml (Join-Path $Destination 'cargo-vendor')
    if ($LASTEXITCODE -ne 0) { throw 'Cargo source vendoring failed.' }
    # The application carries a source-built, open WebView2 loader patch in
    # src-tauri/vendor. Never redistribute the upstream proprietary loader
    # binaries that an older source bundle may have left in this directory.
    $obsoleteLoaderCrate = Join-Path $Destination 'cargo-vendor/webview2-com-sys'
    if (Test-Path -LiteralPath (Join-Path $projectRoot 'src-tauri/vendor/webview2-com-sys')) {
        $resolvedObsolete = [System.IO.Path]::GetFullPath($obsoleteLoaderCrate)
        $resolvedVendor = [System.IO.Path]::GetFullPath((Join-Path $Destination 'cargo-vendor')) + [System.IO.Path]::DirectorySeparatorChar
        if (-not $resolvedObsolete.StartsWith($resolvedVendor,[StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid obsolete vendor source path.' }
        if (Test-Path -LiteralPath $resolvedObsolete) { Remove-Item -LiteralPath $resolvedObsolete -Recurse -Force }
    }
    if (@(Get-ChildItem -LiteralPath $Destination -Recurse -File -Filter 'WebView2Loader*' | Where-Object Extension -In '.dll','.lib').Count) {
        throw 'Source bundle contains a proprietary prebuilt WebView2 loader. Apply the local open loader patch before publishing.'
    }
    # These are upstream dependency test-only binaries whose source is not in
    # the published crate. They are unnecessary for building Mono Cut. Keep the
    # vendored-source checksum manifest consistent with the documented pruning.
    $excludedFixtures = @(
        @{ crate = 'libloading'; file = 'tests/nagisa32.dll' },
        @{ crate = 'libloading'; file = 'tests/nagisa64.dll' },
        @{ crate = 'system-deps'; file = 'src/tests/lib/libteststatic.a' }
    )
    foreach ($fixture in $excludedFixtures) {
        $crateDir = Join-Path $Destination "cargo-vendor/$($fixture.crate)"
        $file = [System.IO.Path]::GetFullPath((Join-Path $crateDir $fixture.file))
        $cratePrefix = [System.IO.Path]::GetFullPath($crateDir) + [System.IO.Path]::DirectorySeparatorChar
        if (-not $file.StartsWith($cratePrefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid fixture path.' }
        if (Test-Path -LiteralPath $file) {
            Remove-Item -LiteralPath $file -Force
            $checksumsPath = Join-Path $crateDir '.cargo-checksum.json'
            $checksums = Get-Content -LiteralPath $checksumsPath -Raw | ConvertFrom-Json -AsHashtable
            $checksums.files.Remove($fixture.file) | Out-Null
            $checksums | ConvertTo-Json -Depth 10 -Compress | Set-Content -LiteralPath $checksumsPath -Encoding utf8NoBOM
        }
    }
    $excludedFixtures | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Destination 'excluded-upstream-test-fixtures.json') -Encoding utf8NoBOM
    node scripts/audit-source-inputs.mjs $Destination
    if ($LASTEXITCODE -ne 0) { throw 'Native source input audit failed.' }
    node scripts/prepare-js-source.mjs (Join-Path $Destination 'javascript')
    if ($LASTEXITCODE -ne 0) { throw 'JavaScript source preparation failed.' }
    $extra = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'extra-source-inputs.json') -Raw | ConvertFrom-Json
    $installerSource = Join-Path $Destination 'installer'
    New-Item -ItemType Directory -Path $installerSource -Force | Out-Null
    foreach ($source in $extra.installer) {
        $file = Join-Path $installerSource $source.file
        if (-not (Test-Path -LiteralPath $file)) { Invoke-WebRequest -Uri $source.url -OutFile $file }
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $source.sha256) { throw "Installer source hash mismatch: $($source.name)" }
        & tar tf $file | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Installer source archive is invalid: $($source.name)" }
    }
    $extra.installer | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $installerSource 'source-inputs.json') -Encoding utf8NoBOM
    New-Item -ItemType Directory -Path (Join-Path $Destination '.cargo') -Force | Out-Null
    @'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "cargo-vendor"
'@ | Set-Content -LiteralPath (Join-Path $Destination '.cargo/config.toml') -Encoding utf8NoBOM
    @'
This archive carries exact Cargo.lock crate source and the locked npm runtime
package archives (including their licenses). Unpack it in the application source
root to use the included .cargo/config.toml and cargo-vendor directory.
Cargo can then build with --locked --offline. Node development tools still need
npm ci or a populated npm cache. The JavaScript archives include upstream
repositories and git revisions in javascript/source-inputs.json.
Preferred upstream React, Tauri and Lucide source archives are also included.
NSIS and its Tauri plugin source archives are in installer/. The application
uses the NSIS zlib compressor. Build tools and platform runtimes remain external
prerequisites; these archives do not promise an entirely offline npm toolchain.
Upstream proprietary WebView2 loader binaries are excluded; the application
source includes its replacement patch in src-tauri/vendor/webview2-com-sys.
Three unused prebuilt dependency test fixtures are excluded as recorded in
excluded-upstream-test-fixtures.json. Their checksum entries are pruned; building
Mono Cut is unaffected, but tests of those dependencies may need fresh fixtures.
Remaining Windows import libraries are openly licensed generated declarations
and thunks, not implementations of Windows. Wit-bindgen's packaged WASM runtime
objects have their matching C/Rust source in cargo-vendor/wit-bindgen/src/rt.
Every remaining native import/runtime object is identified and SHA-256 hashed
in native-source-input-inventory.json with its license and source origin.
Upstream Tauri source retains the MIT VSWhere 3.1.4 build locator executable;
its exact preferred source archive is also supplied under installer/. This
supporting tool is not bundled in the installed Mono Cut application.
Media source and fonts are supplied separately in mono-cut-media-source.
The application source, lockfiles and build scripts are the main source archive.
'@ | Set-Content -LiteralPath (Join-Path $Destination 'README-SOURCE.txt') -Encoding utf8NoBOM
    Copy-Item -LiteralPath (Join-Path $projectRoot 'src-tauri/Cargo.lock'),(Join-Path $projectRoot 'package-lock.json') -Destination $Destination
    if ($Archive) {
        $archivePath = [System.IO.Path]::GetFullPath($Archive)
        New-Item -ItemType Directory -Path (Split-Path -Parent $archivePath) -Force | Out-Null
        $sevenZip = Get-Command 7z -ErrorAction Stop
        Push-Location $Destination
        $temporaryArchive = $archivePath + '.tmp-' + [guid]::NewGuid().ToString('N')
        try {
            & $sevenZip.Source a -tzip $temporaryArchive '.\*' -mx=5
            if ($LASTEXITCODE -ne 0) { throw 'Source archive failed.' }
            Move-Item -LiteralPath $temporaryArchive -Destination $archivePath -Force
        }
        finally { Pop-Location }
    }
} finally { Pop-Location }
