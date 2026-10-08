# Mono Cut 0.1.2 release candidate

This candidate packages the accepted source stages 4B–7. Publication and package
verification are pending; the public 0.1.1 installer remains unchanged until a
new release is verified. The final public release record will link the exact tag,
commit, checksums, source archives, static package inspection and CI results.

## Included implementation

Easy mode and the seven-step tutorial remain the defaults. Actual native import,
multitrack edits, project save/reopen, relinking, proxies and exports remain on
the Rust/Tauri/FFmpeg foundation. This candidate adds requested-frame and
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
