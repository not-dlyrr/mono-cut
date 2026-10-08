# Bundled media build

Mono Cut distributes its own software Windows x64 FFmpeg 8.1.1/FFprobe build,
rather than a broad third-party binary of uncertain corresponding-source scope.
Inputs are pinned in `scripts/media-sources.json` and verified before extraction.

The only external linked media libraries are x264 at commit
`c24e06c2e184345ceb33eb20a15d1024d9fd3497`, FreeType 2.14.1, HarfBuzz 14.6.0
and zlib 1.3.2. zlib supports PNG thumbnail encoding/decoding.
FreeType is built with zlib, bzip2, PNG, HarfBuzz and Brotli dependencies disabled.
HarfBuzz has FreeType integration enabled and optional tools/subset/raster/vector
and GPU components disabled. x264 is static with CLI/OpenCL disabled. NASM
assembles CPU optimizations; it does not add a runtime dependency.

The configure recipe is:

```sh
sh configure --prefix=/mono-cut-media --target-os=mingw32 --arch=x86_64 \
  --enable-gpl --enable-version3 --enable-static --disable-shared \
  --disable-autodetect --disable-network --disable-hwaccels --disable-doc \
  --disable-debug --disable-ffplay --enable-libx264 --enable-libfreetype \
  --enable-libharfbuzz --enable-zlib --pkg-config-flags=--static \
  --extra-cflags="-O2 -ffile-prefix-map=BUILD_ROOT=media-source" \
  --extra-ldflags="-static -static-libgcc -static-libstdc++"
```

`BUILD_ROOT` is replaced at build time, then the embedded descriptive
`FFMPEG_CONFIGURATION` string maps that machine path to `/build/mono-cut-media`.
This is a generated-configuration privacy change, not a change to FFmpeg code.
The complete executable recipe is `scripts/build-media.sh`. There are no custom
media-source patches. Compiler file-prefix mapping avoids embedding private
build paths in assertions or file metadata.

## Rebuild and source archive

Run from the repository root with PowerShell 7, Git for Windows and 7-Zip:

```powershell
pwsh -File scripts/prepare-media.ps1 -ForceBuild -Jobs 6
pwsh -File scripts/prepare-media.ps1 -SourcesOnly `
  -SourceBundle work/mono-cut-media-source-0.1.0.zip
```

The source archive contains the exact FFmpeg, x264, FreeType, HarfBuzz and zlib source
archives, official Inter 4.1 font release and glyph source archives, media build scripts, input
hashes and license texts. Build tools have pinned binary hashes and upstream
source archive URLs; they are not shipped in the application. w64devkit's static
MinGW runtime notices are retained. GCC runtime linkage uses the upstream GCC
runtime library exception. The app release source/lockfiles and dependency
inventory cover the Rust/UI application separately.

`licenses/media-provenance.json` records hashes of installed FFmpeg, FFprobe and
Inter resources plus actual version/configuration/filters/encoders. A rebuilt
binary may differ byte-for-byte because timestamps, environment and compiler
inputs affect reproducibility. The recipe is source reproducible, not a claimed
bit-identical reproducible-build guarantee. New binaries need new provenance
and release checksums, even if feature configuration is unchanged.

## Distribution and portability

This FFmpeg configuration is GPL-3.0-or-later. x264 permits GPL v2 or later;
FreeType's FTL, HarfBuzz's MIT and zlib's Zlib terms are compatible with this distribution.
The Inter font remains separately OFL-1.1 licensed. Retain all notices and place
the full corresponding media source beside every downloadable binary release.
Upstream reference: [FFmpeg licensing](https://ffmpeg.org/legal.html).

The pinned Windows binary has no vendor hardware acceleration interfaces.
Capability detection reports the installed configuration; CPU software encoding
is the supported baseline. Linux/macOS development and CI build native FFmpeg
8.1.1 through `scripts/prepare-media-unix.sh`, using the same seven hash-pinned
source inputs. The Unix recipe retains the GPL/version3, software-only and
disabled-autodetection/network configuration, with native CPU detection and the
host C++ runtime in place of the Windows target/linker options. Static media
libraries use position-independent code; optional HarfBuzz CoreText, GLib and ICU
integrations are disabled. See the README for platform build prerequisites.

The Unix recipe records configuration and binary/font hashes in its build
directory's `build-provenance.json`. A warm cache is accepted only when recipe,
platform, architecture and output hashes match. System builds from older/newer
major versions are not validated substitutes; the first Unix CI runs exposed
different frame/filter behavior. New platform binaries still need their own
configuration, licenses, corresponding source and package/runtime validation
before being bundled in a release. Adding this recipe does not establish a
verified Linux/macOS installer release.
