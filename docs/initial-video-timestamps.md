# Initial video timestamps: focused Stage 4B correction

This correction ships in [0.1.2](release-0.1.2.md). Independent review accepted
the tested Stage 4B prepared-region clock/readiness scope. Acceptance covers the
recorded source timing, exact picture/PCM coverage and validated file readiness;
the retained strict lossy-audio fidelity diagnostic still fails. The older
0.1.1 installer and assets remain intact. Evidence below retains its recorded
stage snapshot rather than a freshly installed 0.1.2 session. All
executed validation below used silent file decoding and headless engine helpers;
no editor, browser, player, installer or audio output was launched.

## Failure and input policy

An accepted H.264 long-GOP/B-frame Matroska source produces real picture through
the original full renderer, but optimized input seeking made regions `[78,79)`
and `[121,124)` entirely black. The untouched failure was reproduced before edits:
RGB RMS versus the original full render was 155.416 and 155.646 respectively.

The source has 288 selected video packets; 51 lack PTS at indices 0–49 and 52.
The generator attempted negative offsets, but the admitted source's observed
video/audio origin is **zero**. This is missing initial PTS behavior, not evidence
of a negative source origin. The project uses 160×90, 24000/1001 fps, 44100 Hz,
a clip starting at frame 37 for 180 frames, `source_in=123/997` and `speed=17/23`.

Before regional input seeking, `video_seek.rs` inspects at most 256 packets of
the selected physical video stream from its initial position. Missing packet
PTS selects an origin-prefix video decode with the original exact filter EOF.
Only that video's input seek is omitted. The shared composition clock, rational
coordinates, full-export branch, PCM audio seeking and encoder settings remain
unchanged. Normal B-frame PTS reordering and missing initial DTS are allowed.
Originals and normalized proxies have independent assessments.

The initial sample does **not** certify timestamps later in the file. Prefix
cost grows with source trim/playhead; this exception is not bounded region-only
decoding. `video_seek_checks` exposes sample scope, missing counts, cache hits,
identity and inspection cost. `video_prefix_fallbacks` exposes the replaced seek,
actual zero seek, nominal prefix span and filter EOF termination.

The assessment cache uses source, selected stream, FFprobe stamps and recipe
`video-initial-packet-clock-v1`. It retains at most 128 fixed-size records within
16 KiB. Probe output and retained error tail are each capped at 64 KiB, with a
ten-second deadline. Cancellation kills/reaps the managed child and joins its
readers. Failed, cancelled or changed-source results do not enter the cache;
classification writes no sidecar. Preview recipe
`program-preview-v6-regions-initial-clock` and the explicit assessment recipe/
FFprobe stamp invalidate old previews and queued admission.

## Actual media and coverage

Two focused media tests pass their stated scope: 21 lossless/native region
comparisons across the untouched counterexample and generated irregular/regular
sources, plus an actual normalized-proxy switch. The generated source is 24 fps
in the 24000/1001 project, with the same fractional trim/slowed speed. Its stereo
test audio uses moderate sine/pulse levels; it is not a loud-source codec test.

All seven untouched regions match the original full renderer's decoded RGB and
PCM **byte for byte**. The corrected full render also matches its original full
RGB/PCM; PCM contains 399,137 stereo sample frames. First/last contributing
frames, both previously failing ranges, a 120-frame (5.005-second) region and
two adjoining 60-frame regions pass. The join covers exactly 220,720 stereo
sample frames, with no omitted or duplicate coverage. Saved clip/export
coordinates are preserved.

Native 960×540 H.264 jobs verify actual CRF 20, GOP 12, effective minimum interval
7 and scene-cut 0. Maximum retained picture errors are grayscale RMS 1.486,
sampled RGB 2.845 and ROI 4.920, against unchanged limits 5/6/6. Generated native
audio RMS maxima are 0.001497/0.001691, below the unchanged 0.003 limit. A real
normalized proxy has usable initial PTS and retains seeking; disabling it returns
to the original's prefix policy.

The untouched sharp-pulse source **does not pass** the native AAC-versus-FLAC
0.003 limit: all seven regions measure 0.011435–0.014726; the adjoining comparison
is 0.013531. Lossless PCM is exact. Later independent controls classify the
formerly unexplained residual using the production filter's unquantized float
PCM. For `[121,124)`, `[78,198)`, `[78,138)` and `[138,198)`, those region samples
equal the corresponding full-timeline float samples exactly: RMS/max difference
0, sample lag 0 and gain 1. Separately encoding the same float samples at AAC
160 kbit/s produces every decoded native sample exactly in all four cases.

The independently quantized FLAC input differs from float PCM by about 6.9e-8
RMS, but encoding it produces different AAC samples: native-versus-FLAC-encoded
AAC RMS is 0.007542, 0.006910, 0.007973 and 0.013031 respectively. Matching
controls with and without the mux time guard give the same result. The root
review independently decoded and rehashed all four controls and confirmed both
the float-clock and matching-encoder equalities. This identifies downstream
lossy encoding/reference-format effects for this fixture, rather than changed
timeline samples or a region normalization/seek offset. It does not turn the
failed strict AAC-versus-FLAC diagnostic into a pass or establish audible quality.
No audio processing, bitrate, numeric tolerance or source bytes were changed.

A preliminary 120p native comparison also exceeded the unchanged RGB/ROI limit
(6.378/6.294 versus 6). Its failed output is preserved. The 540p results above do
not establish that quality bound at every preview resolution. A preliminary
generated regular-source AAC tail also exceeded 0.003; its higher-amplitude
fixture and failure remain retained. The final generated fixture's lower audio
levels and complete source hashes are recorded explicitly.

## Prefix cost and ordinary readiness

The initial counterexample assessment took 63.84 ms; its cold compile took
211.80 ms including assessment and PCM preparation. The following direct native
job times start after that preparation; they are file-ready correctness timings,
not controller latency or monitor display measurements.

| Global region | Nominal prefix seconds | Observed input frames decoded | Native file-ready seconds |
| --- | ---: | ---: | ---: |
| [37,38) | 1.024 | 7 | 0.201 |
| [78,79) | 2.288 | 39 | 0.210 |
| [121,124) | 3.675 | 122 | 0.232 |
| [78,198) | 5.957 | 177 | 0.636 |
| [216,217) | 6.172 | 190 | 0.252 |
| [78,138) | 4.107 | 133 | 0.449 |
| [138,198) | 5.957 | 177 | 0.494 |

Instrumented decoding observes a reconstructed timestamp regression in this
source. Input frame counts report work that final PTS alone would understate.
Nominal spans and these small-fixture times are not universal decoding caps.

The original/proxy ordinary 60-second fixtures were measured again with the
current production controller/store/jobs, the actual 100 ms edit debounce,
one cold preparation and ten real cumulative edits per cohort. Native files
verify CRF 20/GOP 15/effective minimum 8/scene-cut 0.

| Workload | Cold decoded frame | Cold 5s file ready | Edit frame p95 | Edit 5s file-ready p95 |
| --- | ---: | ---: | ---: | ---: |
| originals | 1.018 s | 2.674 s | 0.921 s | 3.654 s |
| proxies | 0.762 s | 2.228 s | 0.723 s | 3.106 s |

p95 uses nearest rank and is the maximum of ten edits. Initial application caches
are empty; OS file cache is not forcibly cleared. Classification cost is included
in these endpoints. Plans collected afterward can report assessment cache hits;
their inspection times must not be mistaken for the earlier cold worker cost.

All 22 current continuous MP4s and all 22 first-frame MP4s/RGB hashes are identical
to retained v5 assets already decoded against each own edited full reference.
The linkage verifies actual files, retained full MKVs, source/proxy/tool hashes,
actual profiles, interface/helper files, hydrated render descriptors, clip state
and export ranges. Disposable title-sidecar filenames are normalized only for
diagnostic graph comparison. This is byte-equivalence linkage to those prior
references, not 22 new full-reference renders. Both runs end with zero managed
children and partial files; final caches are 23.39/21.87 MB.

## Regression record and evidence

61 relevant engine tests pass, including the new capped LRU, source/stream/tool
identity, failure/retry and active cancellation/reaping checks. Existing mixed
frame rates, nonzero origins/offsets, split/envelope phase, long-GOP filter EOF,
source recovery/relink, cache retention and cached-Undo supersession gates pass.
23 engine source/build/test hashes are unchanged throughout the run. The two
scoped actual-media tests and 62 interface tests pass. Interface source matches
the previously captured type-check/build; no new UI behavior is claimed.

The [path-free evidence](evidence/initial-video-timestamps.json) contains all
regions, strict failures, source/tool/helper/asset hashes, timing samples and
reference linkage. SHA-256:
`96d825b774ef83a883b39c37d255e9c1af85b72e1d2a126f2aafb83d1383514e`.

Important source identities:

- Untouched source: `66219b1d7b14c28054641b8f04fb98e70c46b52a7ab141770a70f8afe2e64ee0`
- Untouched project: `9c9d4a0e64910d08bcb34d5a10f22588a08ae3cfef79b23c798bfeb3b65f8406`
- Renderer: `dcc79ada2ccdce49e266d900b4c25453dfb93ff781bd3f58b800af742c9213e5`
- Assessment: `dcdc6f0f3cb728317ffd952ead859c844401ba02e90d3a7a44d1d600d435415b`
- Preview: `1502c26c5090e086a5f75fcefd1b1b867983ff29e6f167cc9101f9ab8556405e`

## Independent acceptance and limits

The independent engine review reproduced all seven untouched regions against the
preserved original full render, with exact lossless RGB/PCM and exact adjoining
220,720-stereo-sample coverage. Actual 540p JobManager output passed retained
picture RMS 6, with maximum per-frame RGB RMS 2.886003. It also reproduced the
retained strict AAC failures, executed the matching float-PCM encoder controls,
passed four policy tests and the capped-LRU test, and verified two actual
optimized-seek regions with positive source origin 23.373 s and audio start
23.472 s. Its small-source file-ready times are not monitor latency or an
ordinary-workload benchmark. The normalized proxy switch remains builder-tested
and source-reviewed; that reviewer did not independently rerun it.

A separate criteria audit rehashed report/artifact linkage and recomputed the
ordinary timing percentiles above. It distinguished retained builder timings
and reference linkage from the independent engine/root decoding. The original
curated JSON is preserved unchanged as historical evidence; its formerly
unclassified-residual wording predates these later controls. Independent review
artifacts remain outside the source checkout. Their immutable identities are:

- Engine `stage-4b-timestamp-engine-review/result.md`:
  `a29a632d43af44cac840ab4fb8a9cddb5a2429488351fba48f2f6498bc098b6d`
- Root `stage-4b-timestamp-root-check.py`:
  `423c42f0bc8ade3af2d553b222d55ef88f0066b212c8bc9f201dc7bae3e9f0a8`
- Root `stage-4b-timestamp-root-check.json`:
  `712712514b8f3b46d1c4b8d074c1dc2c20810c008cdb5de589a7b1f5f4980aac`
- Criteria `stage-4b-timestamp-criteria-review/result.md`:
  `f5ae1044c2ff6946dadd6ffdab58c2936b7afa1b00981599905120140d6d1795`

Raw generated/retained media, failed outputs and private logs remain outside the
source checkout. The earlier [v5 record](preview-regions.md) remains historical
known-fixture evidence. Later timestamp anomalies, genuine negative origins,
every resolution/codec, native display/audio behavior, seamless region-boundary
playback and packaged release validation remain outside this acceptance. Prefix
cost continues to grow with source trim/playhead. The retained native codec-limit
failures remain unresolved fidelity diagnostics. Acceptance is limited to the
tested clock/readiness scope and does not assert professional feature parity.
