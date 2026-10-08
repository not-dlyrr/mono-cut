# Mono Cut 0.1.2 release record

Published as a public prerelease on 2026-10-08. It packages the accepted stages
4B–7 while retaining the original failed and accepted evidence. All six assets
were downloaded without authentication and rehashed; their sizes and SHA-256
digests match the packaged inputs and GitHub metadata. Repository, tagged source,
release and download URLs returned HTTP 200. The previous releases remain intact.

- [Windows x64 installer](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/mono-cut-0.1.2-windows-x64-setup.exe)
- [Public release and all six assets](https://github.com/not-dlyrr/mono-cut/releases/tag/v0.1.2)
- [Exact tagged source](https://github.com/not-dlyrr/mono-cut/tree/v0.1.2)
- [Checksums](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/SHA256SUMS.txt) and [build information](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/BUILD-INFO.json)
- [Machine-readable verification](evidence/release-0.1.2.json) and [packaged source freeze](evidence/release-0.1.2-source-freeze.json)

Exact source commit: `6da0bd2300295c12fcb25b725983bcaf9d2a52c8`.
Source tree: `ca1030d1958ead2f93752e37bc3daf3c57c5fc93`.
The tag and application source ZIP identify this commit, including the license,
notices, setup/build scripts and stage evidence. This publication report is a
subsequent documentation-only update; it does not change the tag or release assets.

## Release assets

| Asset | Bytes | SHA-256 |
| --- | ---: | --- |
| [BUILD-INFO.json](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/BUILD-INFO.json) | 8,841 | `09abde1a3ca1d850594306256de42aaef7be30727ce65044f415d229f5d9d36f` |
| [mono-cut-0.1.2-windows-x64-setup.exe](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/mono-cut-0.1.2-windows-x64-setup.exe) | 34,691,557 | `0878859e0868429c6d2a9be0ee143b4162a8835a02f09e4598058999e911dd3c` |
| [mono-cut-media-source-0.1.2.zip](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/mono-cut-media-source-0.1.2.zip) | 79,579,716 | `c3ca8309a686a92426c0449a06ab0adedd73776f2ad04b01d1e3e409aced502d` |
| [mono-cut-source-0.1.2.zip](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/mono-cut-source-0.1.2.zip) | 2,526,558 | `eaeab907d18be9a37b2475611a82a51f357499f2c5a819df4ca7d00125090e2a` |
| [mono-cut-source-dependencies-0.1.2.zip](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/mono-cut-source-dependencies-0.1.2.zip) | 131,053,057 | `addcac78c5905ab0af7e076a70dff9cd796fbd4fe7478c929d6599d2f8adcf87` |
| [SHA256SUMS.txt](https://github.com/not-dlyrr/mono-cut/releases/download/v0.1.2/SHA256SUMS.txt) | 480 | `c3a4e3081340089e11a2ed8bcaebd78ec5e0c493b3ea5032f8c8a5675950be1a` |

SHA256SUMS lists the five payloads; its own digest is recorded above and in the
verification JSON. Keep the installer together with the three corresponding-source
ZIPs, build information and checksum manifest. A clean exact-tag archive supplies
202 application source files; all were byte-compared after public download.
The locked dependency ZIP contains 24,026 nonempty members. The software media
archive includes hash-pinned upstream sources, recipes and recorded build configuration.
Source preparation excludes the proprietary WebView2 loader/bootstrapper and
unneeded closed upstream test binaries. The open loader's code/tests and notices
are distributed. See [source distribution](source-distribution.md),
[media configuration](media-components.md), [license inventory](../licenses/dependency-inventory.json)
and [third-party notices](../THIRD_PARTY_NOTICES.md).

## Static package and exact-source CI

Local Windows packaging completed successfully. Static NSIS extraction verifies
all 21 bundled media/font/notice resources byte-for-byte, application version
0.1.2, and no embedded private home path. The installer is unsigned (NotSigned).
Built executable SHA-256: `5d80bbe4dbaa38196c346f8a9e59859e47bc00bc97b20e18ee2db56a68f90e99`.
Packaged executable SHA-256: `30e72ef977222481ebe0af8f98e5972488dbcaadc256c0d8d30d0883a5165914`.
Only Tauri's expected three-byte UNK-to-NSS installer marker distinguishes those
executables. Neither executable nor installer was launched. All 32 frozen native
files, 21 owned frontend files, 21 resources and six historical stage JSON files
were checked; native metadata changed only from application version 0.1.1 to 0.1.2.

[Exact-commit CI](https://github.com/not-dlyrr/mono-cut/actions/runs/37840359908) passes on Windows, Ubuntu 24.04 and macOS ARM64.
Each platform passes 142 interface checks and 89 active engine/media tests,
with zero failures. Eight explicitly ignored diagnostics/benchmarks per platform
retain their separate historical evidence. Seven Windows open-loader tests also
pass. The Windows job builds and statically verifies its installer and resources.

[Exact-tag Windows release preparation](https://github.com/not-dlyrr/mono-cut/actions/runs/37840368151) also passes all tests,
installer extraction, source preparation, checksum/manifest verification and
artifact upload. CI artifacts were downloaded and checked separately. Their
installer SHA-256 is `a24052a141c1cbc8b1194cad1c958e061fc03db775d119729dd0e325d18bbf49`; it is a different build and
does not replace the public installer checksum in the table. Compiler/tool versions,
source/media input hashes and configuration are retained in BUILD-INFO. Independent
builds are not promised byte-identical.

Engine/media gates exercise actual generated files for edits, save/reopen/relink,
export cancellation, mixed rational rates, source clocks, source-span retiming,
bounded region/reference comparisons, audio samples and caches. They establish
their strict tested file/model behavior, not native display or speaker output.
The local release-stage interface suite also passes all 142 checks. Older native
UI tests and screenshots retain their own 0.1.0 date/version rather than being
relabelled as evidence for this package.

## Included implementation

Easy mode and the seven-step tutorial remain the defaults. Actual native import,
multitrack edits, project save/reopen, relinking, proxies and exports remain on
the Rust/Tauri/FFmpeg foundation. This release adds requested-frame and
five-second background-rendered regions with one adjoining successor, retained
transport intent and ordered native asset pins. It does not implement a direct
GPU compositor. The CPU fallback requires no particular GPU vendor.

- [Prepared regions](preview-regions.md), [initial timestamp correction](initial-video-timestamps.md)
  and [successor continuity](preview-continuity.md) retain measured source/file
  evidence and failures. Preview recipe v8 uses an integer post-trim audio sample
  clock. Active media pins can exceed the 32-record/512 MiB retention target.
- [Speed](retiming.md) preserves rational selected source spans, resizes linked
  A/V atomically and rejects overlaps. Arbitrary ratios round to whole-Hz audio
  resampling rates and change pitch. No reverse or pitch-preserving stretch is
  claimed. Version-1 project compatibility and rounding are documented.
- [Timeline waveforms](timeline-waveforms.md) follow source-in, speed and proposed
  trim/slip ranges. They aggregate all intersecting pre-effect mono peak bins,
  with bounded visible-window rendering. They are coarse source curves, rather
  than post-effects loudness meters or sample-accurate peak positions.
- [Monitor keyboard ownership](monitor-keyboard.md) routes Source/Program
  transport, steps and range marks to the activated monitor in either mode,
  protects widget keys and exposes a restrained keyboard/focus cue.

## Evidence context and limitations

Earlier evidence JSON remains byte-identical and refers to its recorded source
snapshot, not a newly installed 0.1.2 session. The timed continuity fixture had
no inherited split-envelope fields. Separate `split_envelopes`, retime and
actual-media regression cases supply split-envelope checks; the timed fixture
must not be cited as verifying them.

Keep the retained strict sharp-pulse AAC-versus-FLAC RMS failure, the separate
older fractional split's tiny float differences, and the 256-packet initial-PTS
inspection/prefix-decoding cost limits visible. Neither coarse waveforms nor
matching rendered regions establish universal frame, sample or codec fidelity.
Native media-element readiness, focus, pointer, display and audible successor
handoff have not been observed for these new stages.

All new release-stage validation is silent and file/source based. No installer,
native editor, browser or player is launched. Fresh installation, native
import/edit/save/reopen/export UI and Linux/macOS GUI behavior remain unverified
for this package. [Screenshots](screenshots.md) are retained 0.1.0 captures from
2026-10-08 and cannot prove this release's runtime behavior.

The Windows x64 package is unsigned and requires Windows 10/11 with an existing
WebView2 runtime. No proprietary loader/runtime bootstrapper is distributed.
GPL application source, exact software FFmpeg build configuration, Inter and
icon notices, locked dependency source and build information accompany it.
Retain the installer together with the three matching source ZIPs, BUILD-INFO
and SHA256SUMS. Byte-identical independent rebuilds are not promised.

Nested sequences, multicam, scopes, LUTs, masks, tracking, advanced audio,
captions, HDR/color management and an open plugin API remain roadmap work.
Playback remains prepared-file based, stereo, CPU rendered and 8-bit YUV 4:2:0.
This release is a working early editor, without professional feature parity.
