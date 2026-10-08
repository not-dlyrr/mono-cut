# Mono Cut

Mono Cut is a GPL-3.0-or-later desktop video editor built with Rust, Tauri 2,
React and TypeScript. It imports local media, edits a multitrack sequence, saves
and reopens projects, and exports through a locally bundled software FFmpeg
engine. No accounts, subscriptions or rendering services are required.

Version 0.1 is a working foundation. The program monitor currently plays a
background-rendered preview file; direct low-latency native playback and advanced
professional tools remain development work. See the
[feature matrix](docs/features.md) and [validation record](docs/validation.md)
for exact capabilities and verified limits.

[Source repository](https://github.com/not-dlyrr/mono-cut) ·
[Versioned releases](https://github.com/not-dlyrr/mono-cut/releases) ·
[Native screenshots](docs/screenshots.md)

The [publication record](docs/publication-validation.md) records verified public
links, checksums and CI results. The [0.1.1 release record](docs/release-0.1.1.md)
describes the portable display-fit correction and its package validation.

## Windows installation

The Windows x64 release assets pair the installer with application, media and
dependency source archives, `BUILD-INFO.json` and `SHA256SUMS.txt`. Verify the
installer's SHA-256 against that manifest. The installer includes the
software FFmpeg/FFprobe tools, Inter title font and retained license notices.
Windows 10/11 and an existing Microsoft Edge WebView2 runtime are required by the
Tauri desktop shell. The installer does not bundle a proprietary runtime
bootstrapper. The release is unsigned until a project signing process is added.

The media engine runs on CPU and supports different GPU vendors; no vendor SDK
or hardware encoder is required. Disk space for imported media, proxies, preview
renders and exports is separate from the installation size.

## Usage

Easy mode is on by default. It uses a CapCut-style media/tool rail, one tabbed
Source/Program viewer, a contextual inspector and a large timeline. The first
project opens a seven-step tooltip tutorial. Skip it any time, or reopen it from
**Help → Restart tutorial**. Controls also show tooltips and shortcut hints.
Switch the top **Editor mode** selector to **Advanced** for separate monitors,
the full inspector and keyframes. Both modes edit the same project format;
switching modes preserves the timeline and work.

1. Create a project with a name, dimensions and rational frame rate.
2. Import local video, audio or images. Use bins to organize them. Select a media
   item to inspect it in the Source monitor and set source in/out points.
3. Add media to the timeline. Select clips to trim, move, split, duplicate or
   ripple delete. The inspector controls transforms, crop, speed, color, audio,
   fades and basic keyframes. Add titles through the timeline controls.
4. Wait for the program preview to refresh, then play or scrub. Frame stepping
   and J/K/L operate the monitors. Generate video proxies from the media item's
   menu and choose preview resolution/proxy use in program controls.
5. Save a `.monocut` project, reopen it later, and relink missing files through
   the media item's menu. The project references footage; it does not copy it.
6. Export MP4 H.264/AAC or MKV FFV1/FLAC. Choose dimensions, fps, quality and audio
   settings. Export progress and cancellation operate the actual encoder job.

Use the Appearance menu for dark/light theme, liquid glass and reduced motion.
Workspace offers panel layout, media engine information and keyboard remapping.
Dragging and trimming are direct; keyboard nudge/edit controls provide
alternatives. Saved shortcut mappings and appearance settings are local UI
preferences, separate from the project.

## Development setup

Install Node.js 22+, Rust stable with the Windows MSVC target, Visual Studio C++
build tools, Git for Windows (including Git Bash), PowerShell 7 and 7-Zip on PATH.
These are build prerequisites; no build tools are bundled in the editor.

```powershell
npm ci
pwsh -File scripts/prepare-media.ps1
npm run desktop
```

The media preparation script downloads version-pinned source/tool archives,
checks every SHA-256, and compiles a minimal software FFmpeg with x264, FreeType,
HarfBuzz and zlib. A fresh build can take several minutes. Use `-Jobs 4` on smaller
machines. Existing media resources are accepted only when they match recorded
binary checksums. `-ForceBuild` always recompiles. Generated files stay in
`work/media-build/`, excluded from version control.

```powershell
npm run check
npm run test:preview
npm run build
npm run test:engine
cargo test --manifest-path src-tauri/Cargo.toml --test engine_workflow -- --nocapture
npm run desktop:build
```

The Windows installer is generated under `src-tauri/target/release/bundle/nsis/`.
The desktop build script remaps local source paths in Rust diagnostics before
compilation, preserving privacy without embedding a machine's home directory.
Do not distribute a rebuild without its corresponding source, notices,
configuration and checksums. See [media reproduction](docs/media-components.md).

## Linux and macOS

The model, edit commands, renderer and React interface are portable. Install the
[Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/), and use
the FFmpeg 8.1.1 baseline built with x264, FreeType and HarfBuzz. Arbitrary older
or newer system FFmpeg major versions are not validated substitutes. Linux also
needs `build-essential`, `cmake`, `ninja-build`, `pkg-config`, `nasm`, `curl` and
`unzip`. On macOS install the Xcode command line tools and Homebrew packages
`cmake`, `ninja`, `pkg-config` and `nasm`. Node.js 22+ is required on both.

```sh
npm ci
bash scripts/prepare-media-unix.sh
export MONO_CUT_FFMPEG="$PWD/src-tauri/resources/media/ffmpeg"
export MONO_CUT_FFPROBE="$PWD/src-tauri/resources/media/ffprobe"
export MONO_CUT_FONT="$PWD/src-tauri/resources/media/Inter.ttf"
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run desktop
```

The Unix recipe compiles the same hash-pinned upstream versions for the native
host. An optional build directory and job count can be passed as its first and
second arguments. Provenance and output hashes are stored in
`work/media-unix/build-provenance.json` for the default directory. Linux/macOS
compile/unit/media test jobs are defined in CI; a platform is not a verified
installer release until its own runtime/package results are recorded.

Engine tests accept `MONO_CUT_FFMPEG`, `MONO_CUT_FFPROBE` and `MONO_CUT_FONT` for
explicit tool/font paths. The desktop shell resolves bundled resources; native
packaging on additional platforms needs appropriately named platform binaries.

## Source, licenses and contributions

Application source is GPL-3.0-or-later. Each binary release should carry the app
source archive, locked Rust/JavaScript source dependencies, corresponding media
source archive, checksums and notices. Dependency versions are pinned by
npm/Cargo lockfiles and media input hashes.
Read [third-party notices](THIRD_PARTY_NOTICES.md),
[source reproduction](docs/source-distribution.md),
[architecture](docs/architecture.md), [project format](docs/project-format.md)
and [contribution guidelines](CONTRIBUTING.md).

User footage, credentials, signing keys, caches, recovery files and private paths
are not release inputs. No proprietary rendering services or paid feature gates
are included.
