# Split envelope validation

These checks exercise engine source with generated media, actual FFmpeg renders
and decoded frames/samples. They do not play audio, open native dialogs, validate
a new installer, or publish a release. The earlier packaged installer predates
these source corrections.

## Reproduced failures and corrections

A 60-frame red clip with a 30-frame fade-in changed brightness after splitting
at frame 15. The original fade restarted faster in the left fragment and was
removed from the right. The initial regression reproduced a maximum RGB change
of 128. Independent signed fade anchors now preserve both portions of the ramp.

Repeated cuts exposed decimal PTS/visibility rounding at frame boundaries.
Integer sequence ticks and frame-based visibility eliminate the missing black
frames. Position automation also exposed a one-pixel endpoint shift from mixing
the overlay counter with rounded decimal keyframe times. Actual timeline PTS
rounded to frame units now evaluates the same positions.

Animated scale followed by rotation used a canvas tied to a fragment's initial
scale. All pixel filters now precede animated scale, keeping their dimensions
stable. This also avoids invalid dynamic-dimension input to FFmpeg's geq filter.
Outgoing dissolve fragments retain the original compositing root, preventing
them from jumping above the incoming picture during overlap.

At 30000/1001 fps, source sample ceil bounds, rounded placement, and duration
trims previously disagreed. A sine fixture shifted one sample after a cut;
an independent audit measured 96,095 rather than 96,096 samples for a 60-frame
sequence. Stable source conversion, shared absolute ceil sample partitions,
and per-sample gain with phase correction remove the gap/overlap and gain shift.

## Decoded results

The first complete seven-test regression run passed in 9.96 seconds:

| Fixture | Measured result |
| --- | --- |
| Fade-in, fade-out, both, boundary/outside/repeated cuts, neutral control | 60 frames / 96,000 samples; maximum RGB and PCM differences zero |
| Linked video/audio with opacity, volume, scale, x/y keys, crop, color and rotation | Cuts 13/31/47 retain 8 clips in four links; maximum RGB and PCM differences zero |
| Independent opacity/rotation/scale/position groups | All decoded RGB and PCM differences zero |
| Outgoing, incoming and repeatedly split dissolve participants over a lower track | 90 frames / 144,000 samples; midpoint R127/B126; no black dip; RGB/PCM differences zero |
| 30000/1001 sequence with 30 fps patterned source and continuous sine/gain | 60 frames / 96,096 samples; maximum RGB and PCM differences zero |
| Trim in/out/both | Retained decoded spans agree with the original envelope |
| Explicit fragment fade edit | Fresh ramp agrees with an independent source-trim/fade reference |
| Undo/redo, save/reopen, hydrated recovery and original/proxy FFV1 renders | Original clip envelopes and decoded spans agree |
| Actual H.264/AAC program preview before/after split | Maximum RGB difference zero; original-media PCM maximum 0.000002146, RMS 0.000000106; proxy PCM difference zero |

Simple lossless fixture bounds are 1 RGB value and 0.000004 maximum PCM error,
0.000002 RMS. The known-duration standalone fade reference permits 5 RGB values
for RGB/YUV conversion and the same PCM bound. Actual lossy preview/export checks
permit 8 RGB values, 0.003 maximum PCM error and 0.0005 RMS, and inspect samples
immediately around cuts. These codec bounds are numerical amplitude differences,
not a claim that every AAC source decodes identically.

Malformed signed anchors and composition metadata are rejected without changing
the active project, explicit save or recovery bytes. Old version-1 clips omit
new fields and retain their normal edge defaults. The same bounded project IO
and store transactions apply to inherited metadata and exclusive-out keys.

The extended final suite passed **10 tests, 0 failures in 49.28 seconds**, with
the earlier matrix plus these checks:

- Cuts at 1/2/7/13/17/31/47 in a 30000/1001 sequence, including timeline start
  7 and source-in 5 frames at 1× and 2× speed: 67 decoded frames and 107,308
  samples; maximum RGB/PCM differences zero.
- Fractional in/out range 2 through 7: exactly 5 frames and 8,008 samples, with
  zero difference from the corresponding original absolute-grid interval.
- Negative render offset −3 after extending a trimmed source-backed clip:
  retained spans, repeated cuts, save/reopen and recovery all match exactly.
- Complete-group moves, independent fragment moves and subset duplication:
  composition lifecycle and decoded envelope samples match exactly.
- A still image held for 12 seconds with a 10-second source in-point:
  splits/trims with fades only, opacity only, and both retain all tested pixels.
- Both signed render-offset extremes and invalid source origins/anchors are
  rejected transactionally. Default fields remain omitted in old version-1 JSON.

The fractional tail regression also exposed early muxer termination: a global
video-frame cap could stop FLAC before its final 4,608-sample block. The graph
now caps video after output-fps conversion and audio at exact sample boundaries;
the time guard lies beyond both finite EOFs. The nonzero-start fixture now keeps
all 107,308 samples. The final output format remains stereo, 8-bit YUV 4:2:0.

A later focused dissolve check passed in 2.89 seconds: retaining only an incoming
tail after the transition window no longer suppresses the outgoing fade. The
tail and outgoing picture agree exactly with an independently assembled ordinary
fade/tail reference (zero RGB/PCM difference).

The final existing checks also passed: nine engine workflow tests (40.84 s), five
source timing tests (8.58 s), two historical relink tests (3.27 s), two metadata
unit tests, and seven persistence tests against the final model (334.58 s).
Together with the ten envelope regressions, that is **35 passed, 0 failed**.
Runs overlapped, so these timings are regression execution times rather than
performance benchmarks. Rust formatting, TypeScript and the production interface
build passed. The same CI test command discovers the new tests automatically.

## Limits and delivery

See [the envelope reference](split-envelopes.md) for trim extension semantics,
explicit fade editing, source conversion offsets and cache invalidation. Removed
automation knots are not reconstructed when extending a fragment. Varispeed
changes pitch. Professional audio buses and time stretching remain unimplemented.

The whole-sequence background preview architecture has not changed; these short
fixtures do not establish long-form playback or scrubbing performance. Inspector
bindings pass TypeScript checking; no new native UI interaction/visual sign-off
was performed during this silent engine stage. Installer rebuilding, public
repository creation, releases and their downloadable links remain separate work.
