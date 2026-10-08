# Source timing validation — 2026-10-08

The shared source clock correction is verified in source on Windows with the
bundled source-built FFmpeg/FFprobe and Rust debug builds. All decoding and
rendering checks were silent. No native dialog checks or audible playback were
restarted. The earlier Windows installer has not been rebuilt with these fixes.
GitHub publication and release validation have not been performed in this stage.

## Supplied regression source

The supplied synthetic `offset.mkv` has video start 0, audio start 1/5 second,
and a decoded pulse at source time 0.700 seconds. A separate scratch harness
compiled the current engine modules and decoded the FFV1/FLAC exports to PCM.
These measurements were repeated after the final frame-grid correction.

| Edit | Decoded pulse at 48 kHz | Video frames | Export duration |
| --- | ---: | ---: | ---: |
| Full source, 1× | 33,600 exactly | 60 | 2.000 s |
| Source in 0.300 s | 19,200 exactly | 45 | 1.500 s |
| Same trim placed at 1.000 s | 67,200 exactly | 75 | 2.500 s |

The previous full export placed the pulse at sample 24,000, 200 ms early.
The corrected graph retains the source's relative stream delay.

## Generated regression suite

`src-tauri/tests/source_timing.rs`: **5 passed, 0 failed in 17.55 seconds**.
Fixtures are generated in temporary directories and contain no personal media
or committed local source paths.

- Audio 200 ms after video: sample 33,600 through original, prepared source,
  proxy, FFV1/FLAC re-export, and program previews with proxies on/off.
- Aligned control and video after audio: sample 24,000 through the same paths;
  delayed video has six leading black frames at 30 fps.
- Common 3 s epoch with audio at 3.2 s: sample 33,600 through every path, without
  applying the epoch or relative delay twice.
- Source trim 0.300 s: sample 19,200. Placement at 1.000 s and splitting at the
  pulse onset: sample 67,200. Split PCM RMS difference is 0.00000000; decoded
  video frame positions match.
- Existing 2× rate mapping: sample 57,600 through original, prepared source and
  proxy; frame barcodes match within the lossy cache bound, with exact lossless
  decoded audio length.
- A 24000/1001 video ending before its audio tail retains the pulse at sample
  96,480 through caches, re-exports and both preview paths. The 30 fps sequence
  exports 61 frames. This catches truncation from flooring cache coverage.
- Leading and trailing video gaps reveal the underlying PNG track through a
  proxy, matching the original composition across all 66 tested frames.
- Old version-1 documents without timing retain IDs, trims, markers and bins
  through reprobe, save/reopen, hydrated recovery, missing-media editing and
  relinking. Source-v3 and unmarked legacy proxies are not reused.

Lossless PCM cue tolerance is four samples (eight for sample-rate-converted 2×
audio). AAC transient tolerance is 256 samples, 5.33 ms, accounting for lossy
codec ringing after normal decoder priming handling. Full-range AAC cue samples
in these fixtures were exact; the tolerance cannot conceal the 9,600-sample
regression. Acceptance uses decoded cues, frame barcodes and sample/frame counts,
not packet start timestamps alone.

The stricter cached 2× barcode comparison initially found a one-frame mismatch
between coarse Matroska timestamps and the normalized MP4 grid. Normalizing
original footage to the same source frame-rate grid before trim/rate conversion
corrected the discrepancy. The strict comparison remains in the regression.

## Existing protections

- All nine existing actual-media engine tests passed in 18.71 seconds against
  the final rendering changes. They cover mixed rates, trimming, effects,
  geometry, linked movement, undo/redo, hydration failure, save/reopen/relink,
  proxy comparison, cancellation, and process shutdown.
- All seven persistence boundary tests passed in 339.12 seconds. Exactly
  67,108,864 bytes remains readable and writable; one more byte rejects safely.
  Oversized edits, escaped text, growing relative paths, compact recovery input,
  and nonfinite waveforms preserve previous readable state and history.
- Both new metadata unit tests passed: exact signed/tick timestamps and the
  distinction between Matroska absolute ends and MP4 elapsed durations.
- Rust formatting and TypeScript checks passed. Existing CI runs the new tests
  through its full test selection; no hosted CI result is claimed here.

## Follow-up: genuine historical duration and relinking

Independent review found that the first legacy test removed timing fields from
a freshly imported project, whose duration was already correct. A genuine old
import could instead retain `format.duration: 5.200000` for video at 3.000 s and
audio at 3.200 s, with a full 156-frame clip. Hydration kept those edits, but
relinking replaced its saved duration with 2.2 s and rejected the same source.
The new regression reproduced that rejection before the compatibility fix.

`src-tauri/tests/legacy_timing_relink.rs`: **2 passed, 0 failed in 4.42 seconds**.
The source and project are generated in disposable directories. Verified:

- Genuine old JSON without timing/compatibility fields retains the full 156-frame
  clip, effects, keyframes, source in, speed, IDs, bin and markers through same-file
  relink, missing-file relink, explicit save/reopen and hydrated recovery.
- Available originals gain a measured 2.2 s compatibility span and streamed
  SHA-256; those values survive persistence. Byte-identical replacement succeeds.
- A first-open missing old record can adopt the narrow documented historical
  metadata pattern, then persists identity and rejects a later different source.
- Lossless original, proxy and relinked exports each contain 156 frames and
  249,600 decoded samples, retaining the 5.2 s edit. The pulse remains at sample
  33,600. The legacy tail is neutral black/silence; source preparation and proxy
  caches cover the physical 2.2 s span.
- Shorter sources and different bytes with the same timing/geometry fail without
  changing the project, undo/redo, explicit saved file or recovery bytes.
- Earlier timing-only migrated documents without `container_duration` gain the
  same compatibility record. New provenance records cannot infer a legacy tail.
- Contradictory compatibility spans are rejected by validation and the reader;
  mathematically equivalent unreduced rational timestamps still relink correctly.

A separate scratch harness used a copy of the review's actual historical fixture:
same-source and missing-source relink succeeded, clip coordinates matched, and its
decoded export again had 156 frames, 249,600 samples and cue 33,600 at 5.2 s. The
review's originals were not changed.

After this correction, all five existing source timing tests passed in 13.00 s,
all nine media workflow tests in 21.87 s, and both timing metadata tests passed.
All seven persistence regressions passed again in 336.83 s, including the exact
64 MiB boundary and transactional rejection cases. The combined source checks
therefore cover 25 passing tests with no failures.
Rust formatting and TypeScript checks passed. No native UI or audible playback
checks were resumed; the shipping installer remains the earlier build.

Missing-first historical byte identity cannot be proven from an old document
that recorded no fingerprint. That first adoption uses metadata inference;
subsequent identified legacy replacement requires matching content. Reading full
legacy files for hashing adds worker time but uses bounded 64 KiB buffers.

## Limits

See [source timing](source-timing.md) for the documented additive format and
normalization policy. Variable-rate video is normalized to its declared source
frame-rate grid. Caches remain lossy H.264/AAC. Internal timestamp discontinuity
repair, native variable-frame cadence, unusual camera formats, and long-form
performance are outside this correction. Fade semantics, speed-edit UX and the
whole-sequence preview architecture have not been changed.

This report verifies engine source, not a newly packaged release. Installer,
public repository, release links and native final-package checks remain separate
delivery work.
