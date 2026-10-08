# Third-party notices

Mono Cut application code is licensed under GPL-3.0-or-later. The application
license does not replace the separate licenses of the components below.
Complete retained license texts are in `licenses/` and are installed with the
application under `resources/notices/`.

## Bundled media components

| Component | Pinned source | License | Purpose |
| --- | --- | --- | --- |
| [FFmpeg](https://ffmpeg.org/) | 8.1.1 | GPL-3.0-or-later for this configuration | Probe, decode, filter, proxy and encode |
| [x264](https://www.videolan.org/developers/x264.html) | c24e06c2e184345ceb33eb20a15d1024d9fd3497 | GPL-2.0-or-later, combined under GPL v3 | Software H.264 encoding |
| [FreeType](https://freetype.org/) | 2.14.1 | FreeType License (FTL) | Title glyph rasterization |
| [HarfBuzz](https://harfbuzz.github.io/) | 14.6.0 | MIT | Title text shaping |
| [zlib](https://zlib.net/) | 1.3.2 | Zlib | PNG thumbnails and built-in codec compression |
| [Inter](https://rsms.me/inter/) | 4.1 title font; fontsource UI version in npm lock | SIL OFL 1.1 | Interface and title typography |
| [MinGW-w64](https://www.mingw-w64.org/) | w64devkit 2.10.0 runtime | Permissive component licenses | Windows C runtime support |

The pinned media build uses only FFmpeg built-in codecs/filters plus x264,
FreeType, HarfBuzz and zlib. It enables GPL and version 3, disables automatic external
library discovery, disables networking and hardware acceleration, and does
**not** enable `nonfree`, FDK AAC, NVIDIA/AMD/Intel SDKs, DeckLink, or closed
third-party plugins. AAC and FFV1 are FFmpeg's built-in implementations. Software
encoding works independently of GPU vendor.

Portions of the media software are copyright (c) the FreeType Project
(www.freetype.org). All rights reserved. See `licenses/FreeType.txt` for the
required attribution and full terms.

Source URLs and SHA-256 checksums for every media input and build tool are in
`scripts/media-sources.json`. The `mono-cut-media-source` release archive contains
the actual source archives, license texts and build recipe for the distributed
media binaries; it is not just a list of links. The recorded binary configuration
and checksums are in `licenses/media-provenance.json`. See
[`docs/media-components.md`](docs/media-components.md) for reproduction details.

## Interface and desktop dependencies

React and React DOM are MIT-licensed. Tauri, its JavaScript API and dialog plugin
are available under MIT or Apache-2.0. Lucide icons use ISC. Fontsource's package
code uses MIT and the included Inter font uses OFL-1.1. The complete resolved
Rust and JavaScript dependency inventory, including versions, licenses and
upstream URLs, is retained in `licenses/dependency-inventory.json` and
`licenses/DEPENDENCY_LICENSES.txt`.

Some published crates omit repository-level notice files. Exact texts from their
package VCS commits are retained under `licenses/upstream/`, with URLs, revisions
and hashes in `licenses/upstream-notice-sources.json`. The notice generator checks
those pins and includes the complete texts in the installed notices.

Tauri's JavaScript API includes Microsoft's permissively licensed `tslib`
helper module. Its copyright and license header are retained in
`licenses/Tauri-vendored-tslib.txt`.

The Windows installer uses NSIS 3.11 with its zlib compressor and the unchanged
`nsis_tauri_utils` 0.5.3 plugin (MIT or Apache-2.0). Their notices are retained in
`licenses/NSIS-3.11.txt` and `licenses/NSIS-Tauri-Utils.txt`. Corresponding
upstream source archives are included with the release source dependencies.
NSIS's complete notice also documents optional compressors that are not used
by this installer.

Rust's standard runtime and GCC runtime components have their applicable
permissive licenses or runtime-linking exceptions. Build tools are prerequisites,
not bundled executables. Windows, its SDK and its WebView2 runtime are platform
components; Mono Cut does not distribute a proprietary WebView2 installer or
bootstrapper. An existing Windows WebView2 runtime is required by the Tauri shell.
Linux uses the system WebKitGTK runtime, and macOS uses the system WKWebView.

The Windows shell replaces the SDK's binary-only WebView2 loader with a
source-built Rust implementation. The generated COM bindings retain MIT terms;
runtime discovery uses ISC/MIT open implementations. The exact sources and
limits are documented in
[`src-tauri/vendor/webview2-com-sys/README.md`](src-tauri/vendor/webview2-com-sys/README.md)
and retained notices are in `licenses/Open-WebView2-Loader.txt`. The loader calls
the installed runtime's internal creation entry point; that ABI is not a
documented stable Microsoft API and requires revalidation after runtime changes.

When redistributing a changed binary, regenerate the inventory, retain the
notices and provide its exact corresponding source and build configuration.
