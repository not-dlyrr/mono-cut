# Mono Cut 0.1.0 release record

This is an early editor foundation with Easy mode and a tooltip tutorial. It
imports actual local media, edits/saves/reopens projects and exports locally.
It does not claim Premiere Pro or Resolve Studio parity.

Source and release locations:

- [Public source](https://github.com/not-dlyrr/mono-cut)
- [0.1.0 release](https://github.com/not-dlyrr/mono-cut/releases/tag/v0.1.0)
- [Earlier native UI screenshots](screenshots.md)
- [Implemented capabilities and limits](features.md)

## Corrected Windows package

The Windows x64 installer was rebuilt on 2026-10-08 from the source containing
the persistence, source clock, cut/envelope, preview reuse and current-intent
corrections. Rust 1.95.0, Node 24 and the source-built software FFmpeg 8.1.1 were
used locally. Versioned artifacts include the installer, exact tagged app source,
corresponding media source, locked dependency source, BUILD-INFO.json and
SHA256SUMS.txt. The latter identifies the actual downloadable files.

The final installer SHA-256 is
`065fbc50f426bb8304e914345cd3e411da1132337bccbd592898c388c563c348`.
Its packaged executable SHA-256 is
`a0537fec0b6cdcf381870f77a61bc8965b1649d0cc85ca512f57949c3e0af0ae`.

Static extraction verified all 21 media/notice resource files against their
reviewed inputs. The application executable agrees with the optimized build
except Tauri's documented package marker: three bytes change UNK to NSS for the
NSIS bundle. No other byte differed. The installer contains the corrected full
dependency notices and their exact upstream source map. It contains no
proprietary WebView2 loader/runtime bootstrapper. The packaged application and
media executables contain no local private home path; the build scripts remap
compiler source paths.

This final package was inspected without executing the installer/editor. It was
not freshly installed or launched. Earlier native UI behavior and screenshots
are recorded in [validation](validation.md); later corrections were checked
through silent engine/interface tests. Speaker playback, renewed native transport
continuity and the final installer interaction are not newly verified here.

## Source, notices and checks

The current scheduling snapshot passed 47 Rust checks and 28 production-helper
interface tests, TypeScript checking and the production interface build. Seven
additional project persistence boundary tests were rerun for release packaging;
seven open-loader tests also passed without creating a browser view. The engine
checks use actual generated media and decoded pixels/audio samples, including
reopen/relink, frame/sample clocks, mixed rates, inherited effects, proxies,
cancellation, caches and stale requests.

The Windows distribution inventory has 253 Rust packages and seven runtime
JavaScript packages, with no UNKNOWN or metadata-only notice omission. Six crates
needed supplemental exact upstream texts; ten files and their commit/URL/hash
provenance are retained. Auxiliary source ZIP lockfiles match the checkout and
all pinned media/font inputs match their source/provenance records. Source scans
exclude credentials, signing material, user projects, personal footage and private
paths. Source packaging refuses dirty/tag-mismatched checkouts, hash mismatches,
unsafe ZIP members and output overwrite. Five rejection cases and two complete
packaging runs passed before the actual release packaging.

Build/test CI is defined for Windows, Linux and macOS. A separate manual workflow
builds and assembles the Windows installer, corresponding source and checksums,
then uploads reviewable workflow artifacts. That workflow does not publish a
release automatically. Remote CI outcomes are separate from the local results
recorded here; Linux/macOS installers have not been validated.

## Current limits

Actual timeline changes still render the whole sequence. The measured 60-second,
20-clip workload required about four minutes to refresh; cached reuse took 4–6 ms
through the engine API. The [benchmark record](preview-reuse-validation.md)
identifies its earlier frozen scheduling snapshot and measurement endpoints.
Incremental native frame/audio playback is the next substantial engine step.

The package is unsigned and requires an installed Windows WebView2 runtime.
The open loader's internal runtime ABI has a documented compatibility limit.
Rendering is CPU based, stereo and 8-bit YUV 4:2:0. Speed changes affect pitch.
Advanced color management, scopes, multicam, nested sequences, masks/tracking,
captions and plugins remain development work, with no fake controls exposed.
