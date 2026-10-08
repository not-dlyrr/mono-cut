# Release source and reproduction

The Windows release pairs the installer with three source archives and
`SHA256SUMS.txt`. Keep these together when redistributing it.
`BUILD-INFO.json` records the tagged application commit, tools and actual artifact
hashes using relative names. The Windows build remaps local Rust source paths;
media builds also remap their compiler paths.

| Archive | Contents |
| --- | --- |
| `mono-cut-source-0.1.0.zip` | Application, local source-built WebView2 loader patch, lockfiles, build scripts, documentation, licenses and CI |
| `mono-cut-media-source-0.1.0.zip` | Exact FFmpeg, x264, FreeType, HarfBuzz and zlib archives; Inter font release and preferred glyph source; verified input hashes, recipe, media notices and binary provenance |
| `mono-cut-source-dependencies-0.1.0.zip` | 424 locked registry Cargo crates, seven locked npm runtime packages, preferred upstream React/Tauri/Lucide source, NSIS/plugin/VSWhere source, relative Cargo vendor configuration and source audits |

Extract the application source first. Extract source dependencies into that
application directory: `.cargo/`, `cargo-vendor/`, `javascript/` and `installer/`
should sit beside `src-tauri/`. The application's local patched
`webview2-com-sys` source is in `src-tauri/vendor/`; it replaces the upstream
binary-only loader. Cargo can then resolve/build its locked dependencies
offline. The Node development toolchain still needs `npm ci` or a populated npm
cache, and platform compiler/runtime prerequisites remain external.

Extract the media source archive into a separate build directory. From the
application root run:

```powershell
pwsh -File scripts/prepare-media.ps1 -BuildDirectory /path/to/media-build -ForceBuild
npm ci
npm run check
npm run build
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/vendor/webview2-com-sys/Cargo.toml
npm run desktop:build
```

The media recipe verifies every cached source/tool input before compilation.
See [media components](media-components.md) for configuration and platform
details. Binary identity is recorded with SHA-256; source reproducibility does
not promise byte-identical compiler outputs across environments.

To recreate the auxiliary source archives from the application checkout:

```powershell
pwsh -File scripts/prepare-media.ps1 -SourcesOnly -SourceBundle work/media-source.zip
pwsh -File scripts/prepare-app-source.ps1 -Destination work/app-source -Archive work/source-dependencies.zip
```

`prepare-app-source.ps1` deliberately excludes upstream proprietary WebView2
loader assets and three unused compiled dependency test fixtures whose preferred
source is absent from their package. Fixture checksum entries are adjusted and
the exclusions are recorded. Tests of those dependencies themselves may need
their own fixture regeneration; Mono Cut's build does not use them.

The remaining native archive/object inputs are openly licensed Windows import
declarations/thunks and Wit-bindgen's source-covered WASM ABI runtime. They do not
contain implementations of Windows APIs. `native-source-input-inventory.json`
records every one with its hash, license, repository/revision and source origin.
The preferred upstream Tauri archive retains its MIT VSWhere build locator;
the exact 3.1.4 source is also supplied. This tool is not installed with Mono Cut.

Windows/WebView2, Linux/WebKitGTK and macOS/WKWebView are system prerequisites,
not redistributable Mono Cut resources. The Windows installer neither bundles
nor downloads the WebView2 runtime. The open loader's internal ABI limitation is
documented in [its source README](../src-tauri/vendor/webview2-com-sys/README.md).
