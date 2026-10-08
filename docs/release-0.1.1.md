# Mono Cut 0.1.1 release record

Version 0.1.1 retains the real import, multitrack edit, save/reopen and export
workflow, with Easy mode and its tooltip tutorial. It corrects a portrait image
edge loss in the shared preview/export renderer. The initial fitted canvas now
uses even pixels before RGBA conversion, preserving its earlier rounding policy.
Preview recipe v2 prevents reuse of stale v1 results. The project format is
unchanged. See [the display-fit evidence](display-fit-validation.md).

## Windows package and corresponding source

The Windows x64 installer was rebuilt and statically extracted on 2026-10-08.
Its SHA-256 is
`9b68ec666e650ce6861e60fa83a98a5995c3c78128cecf3cd612baf5094e89f7`.
The packaged executable SHA-256 is
`74d72868c648682176a19f871512a8afcb489b5744fb139a198ae675e1038ed3`.
All 21 media/notice resources byte-match their source inputs. The executable
differs from the optimized build only by Tauri's documented three-byte UNK-to-NSS
bundle marker. No private home path or proprietary loader/runtime bootstrapper
was found. The installer/editor was not executed during this verification.

The release pairs the installer with exact tagged application source, updated
locked dependency source, corresponding media source, BUILD-INFO.json and
SHA256SUMS.txt. Third-party versions and media binaries are unchanged from 0.1.0;
only the inventory's application version changes. All 253 Rust inventory
entries, seven JavaScript packages, 259 external notice sections and ten retained
upstream texts were checked again, with no missing notice or unresolved license.
The existing 0.1.0 release remains intact.

Release packaging now supports both reviewed versions. Isolated full packaging
fixtures passed for 0.1.0 and 0.1.1; six packaging rejection cases and six workflow
manifest/tamper rejection cases passed. The dispatch-only release workflow checks
exact tag/version/source and six payload files before uploading artifacts. It
does not publish automatically. Main/PR CI builds and tests source; explicit
release preparation handles tags without duplicate push builds.

## Validation and limits

Eight actual-media source/proxy geometry cases passed optimized and scalar
rendering, including exact fractional fit. Related Windows renderer/cache suites
passed 43 tests, followed by a focused final rounding regression and four library
lifecycle/time tests. The corrected engine passed all 54 active Rust tests on
Linux and macOS ARM64. TypeScript checking and the production Windows desktop
build passed after version metadata was updated. The earlier tagged Windows CI
passed its full 54 engine tests, seven open-loader tests, interface checks,
installer build and static integrity check. These are distinct results and
snapshots; see [publication validation](publication-validation.md).

Native UI screenshots and workflow checks are from the earlier 0.1.0 validation.
The 0.1.1 installer was not freshly installed or launched, and speaker playback
was not repeated. Linux/macOS native GUI installers have not been validated.
The Windows package is unsigned and requires an existing WebView2 runtime.

Whole-sequence preview refresh still scales poorly for long edits. The earlier
60-second benchmark took about four minutes to refresh; this release does not
claim a new playback performance measurement. Rendering remains CPU based,
stereo and 8-bit YUV 4:2:0. HDR/color management, multicam, nested sequences,
tracking, advanced audio, captions and plugins remain development work. Full
capability limits are in [the feature matrix](features.md).
