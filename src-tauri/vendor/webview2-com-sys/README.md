# Source-built WebView2 loader

Mono Cut patches `webview2-com-sys` 0.39.1 to keep its generated MIT-licensed
COM bindings while replacing its binary-only Microsoft SDK loader with Rust
source. This directory contains no `.dll`, `.lib`, NuGet package, proprietary
SDK header, or generated loader binary. Cargo compiles `src/loader.rs` directly;
no separate C++ compiler or WebView2 SDK installation is needed.

## Upstream sources and licenses

- Generated bindings: [wravery/webview2-rs](https://github.com/wravery/webview2-rs/tree/edc2caf886175ccaebe86078c9cfe1ae2a187328/crates/bindings),
  crate version 0.39.1, upstream commit
  `edc2caf886175ccaebe86078c9cfe1ae2a187328`. Original MIT notice: `LICENSE-MIT`.
- Runtime discovery and entry-point ABI were adapted from
  [jchv/OpenWebView2Loader](https://github.com/jchv/OpenWebView2Loader/blob/fb3a1687029b986b235dc64d98b395d0cd581494/Source/WebView2Loader.cpp),
  commit `fb3a1687029b986b235dc64d98b395d0cd581494`. ISC notice: `LICENSE-ISC`.
- The same ABI and registry-based runtime discovery are implemented in
  [webview/webview](https://github.com/webview/webview/blob/cbbdee44afff22867de9fd88a9fc8350d9bdd399/core/include/webview/detail/platform/windows/webview2/loader.hh),
  commit `cbbdee44afff22867de9fd88a9fc8350d9bdd399`. MIT notice:
  `LICENSE-webview-MIT`.
- Mono Cut modifications and Rust loader source: copyright 2026 Mono Cut
  contributors, MIT licensed under the terms in `LICENSE-MIT`.

## Runtime behavior and limits

The loader supports the installed Evergreen stable WebView2 runtime in the
current-user or local-machine EdgeUpdate registry, both registry views, and
absolute explicit runtime folders. `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` and
`WEBVIEW2_USER_DATA_FOLDER` overrides are respected. Architectures map to
`EBWebView/x64`, `EBWebView/x86`, or `EBWebView/arm64`.

The loader implements the creation and version APIs used by Wry, version
comparison, and the parameterless creation wrapper. Returned version strings
are UTF-16, NUL terminated, and allocated with `CoTaskMemAlloc`, as required by
the public API. Runtime DLLs stay resident for the lifetime of their COM objects
and asynchronous callbacks. Dependencies load from the explicit DLL directory
and standard trusted Windows locations.

This is a deliberately smaller implementation than Microsoft's SDK loader.
Edge Beta/Dev/Canary selection, packaged runtime discovery, registry/group
policy overrides, creation retries, and channel-selection options on
`GetAvailableCoreWebView2BrowserVersionStringWithOptions` are unsupported. The
latter returns `E_NOTIMPL` for non-null options instead of silently ignoring
them. Standard environment options passed to creation are forwarded to the
runtime. The internal `CreateWebViewEnvironmentWithOptionsInternal` export is
the ABI used by both open implementations above; it is not a documented stable
Microsoft API and future runtime updates require revalidation. Missing runtime
files, unsupported versions, missing exports, and DLL-load failures return
errors rather than selecting a bundled proprietary fallback.

The system WebView2 runtime is a proprietary Windows prerequisite supplied by
the operating system/user. It is neither included in Mono Cut's installer nor
automatically downloaded by it. This patch removes the proprietary **distributed
loader**, and does not make the system runtime open source. Linux/macOS use
Tauri's corresponding system webviews.

## Tests

From the application directory:

```powershell
cargo test --manifest-path src-tauri/vendor/webview2-com-sys/Cargo.toml
```

Tests cover numeric versions, channel suffixes, bad inputs, null pointers,
missing explicit runtimes, and installed runtime discovery/loading when the
system prerequisite exists. An application launch must additionally verify COM
environment creation and the actual WebView rendering after any loader change.
