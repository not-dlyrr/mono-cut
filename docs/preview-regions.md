# Bounded program previews: Stage 4B

An independent accepted-media test found
that input seeking loses real picture in a Matroska source with missing initial
packet PTS, despite a correct full render. The known-fixture results below are
preserved historical evidence; they do not establish seek safety for this media
class. The [focused source-clock correction](initial-video-timestamps.md) was
independently accepted within its tested temporal/readiness scope. The retained
sharp-pulse AAC comparison remains outside its strict quality limit.
This document preserves Stage 4B measurements and its boundary-pause behavior;
[Stage 4C continuity](preview-continuity.md) describes the subsequent successor queue.

This source-only stage replaces complete-sequence preparation in the editing
workspace with requested regions. The published 0.1.1 installer and its release
assets remain unchanged. Validation uses generated media, silent file decoding
and production scheduling code. Native display/resume latency and audible output
remain unverified for this stage and are outside the timing endpoints below.

## Preparation and sequence coordinates

`ProgramPreviewController`, used by App and the silent measurement harness,
checks native identity, publishes the current intent and waits the actual
100 ms edit debounce. It prepares one frame at the requested playhead first,
then a continuous region of at least five seconds when the sequence is that
long. The second stage has no additional debounce. A request near the sequence
end shifts the five-second region backwards to retain its full duration.

Coverage is an exact, half-open global frame interval, with the project rational
frame rate. Local media time zero corresponds to `start_frame`. The monitor
converts local media time to a frame before adding the integer global origin.
Stepping, scrubbing, markers, clip coordinates and export in/out retain sequence
coordinates. The editing cursor can still reach the exact sequence end while
the requested displayed frame clamps to its last valid frame.

An edit or seek outside validated coverage supersedes previous intent. The
workspace hides unavailable footage; it retains a correct new still while its
playback region prepares. Metadata and Save preserve valid playback. Stamped
program identity is separate from region identity so a legitimate coverage
change cannot conceal a source/proxy replacement. Session/revision, key, exact
coverage, output manifest, recipe and file stamps reject obsolete results and
wrong-region reuse. Cached Undo still publishes intent before taking a shortcut.

At each prepared-region boundary playback pauses, prepares the adjoining region,
and resumes once current coverage is available. Reverse shuttle keeps its signed
transport intent across coverage preparation and uses a five-second look-behind
region. Manual seeks, Stop, source or render-model changes clear pending resume intent.
Reverse stepping remains silent. This stage does not claim
seamless long playback, native display/resume latency, or a GPU engine.

## Shared rendering and bounded work

The same compiler implements full export and region rendering. Its working
canvas follows output dimensions; transform offsets scale from project geometry.
Region composition filters out noncontributing clips and creates only the
requested working interval, with 0.5 seconds of bounded history/lookahead for
audio resampling and limiting. Contributing video inputs seek near the required
source time. Exact filter trim EOF stops decoding at the contributing end;
codec keyframe preroll can decode earlier packets than the nominal seek. A short
input duration combined with inexact seeking can expire from that earlier
keyframe and omit the requested picture, so video inputs use filter termination.
Titles generate only contributing frames.
Looped still images also generate only contributing frames, with one guard frame;
large source offsets do not generate a repeated still prefix. Imported PNG/JPEG/
WebP/TIFF images are treated as stills, rather than animated image sequences.
This is not a complete render followed by final output trimming.

Source-stream origins, relative audio/video offsets, converted-frame cadence,
split conversion/composition origins, inherited envelope and automation phase,
speed mapping and dissolve neighbors remain global. Initial video fitting
retains the strict even-pixel geometry correction. `RenderPlan` reports the
working region, per-input nominal source spans, initial packet-clock inspections
and explicit audio/video-prefix exceptions.
Those spans exclude unavoidable codec preroll; they are not packet-read limits.
Each input reports `filter_eof` or `input_duration` termination explicitly.

Some containers quantize packet timestamps, so a packet seek alone cannot
recover the full-render sample-count phase. A stamp-keyed float PCM clock is
prepared once for eligible audio sources. Cold preparation can decode that
whole source's audio; warm edits seek this precise clock and preserve the
varispeed resampler cadence. This cost is reported separately from warm editing.
An individual precise clock is capped at 256 MiB. Larger sources use a slower,
audio-only origin-to-required-end decode without a PCM allocation. Such
audio-prefix work is explicit in plan diagnostics
and is a limitation of this stage, rather than an unavailable-media error.

Before a regional video input seeks, FFprobe reads at most 256 packets of the
selected physical video stream from its initial position. This is an initial
clock check, not a certification of every timestamp later in the source. Missing
packet PTS triggers a source-specific origin-to-contributing-end video decode,
with the unchanged exact filter EOF. The main video input omits its input seek;
the independent precise audio clock keeps its existing seek and sample cadence.
Normal B-frame PTS reordering and missing initial DTS do not trigger fallback.
Normalized proxies are assessed separately from their originals.

`video_prefix_fallbacks` records the optimized seek that was replaced, actual
zero seek and nominal prefix span. That span grows with source trim/playhead and
can exceed the requested preview duration. Actual decoding also includes codec
read-ahead. This exception must not be described as bounded region-only work.
The full-export path is unchanged.

## Processes, cache and cancellation

Stage 4B preview records used recipe `program-preview-v6-regions-initial-clock`;
Stage 4C now uses v7 for common mixing precision, as recorded separately. Old complete-sequence
and interim region records cannot satisfy a request. Records include exact coverage and
frame/sample settings, with generation-specific output paths. Preview retention
targets 32 records/512 MiB. Precise audio clocks have independent 64-record/
512 MiB retention and a 256 MiB per-file preparation limit. Current source,
playback, preparation and manifest assets are pinned. Protected assets can
temporarily exceed retention targets; the existing global media budget also
applies after work releases its pins.

The initial packet assessment uses source/selected-stream/FFprobe file stamps
and its own recipe. Only completed unchanged-source assessments enter an
in-memory LRU capped at 128 records and 16 KiB. Probe stdout and the retained
diagnostic tail are each capped at 64 KiB, with a ten-second deadline. Cancellation
uses the preview worker's flag, kills and reaps the managed child, joins readers
and writes no classification sidecar. Failed, cancelled or changed-source
inspections cannot become reusable results. FFprobe changes also invalidate
preview identity and admission. Cold assessment cost and cache hits are explicit
in `video_seek_checks`.

The interface retains one timer and one displayed asset; its event history is
capped at 64 jobs. File identity checks admit one active native RPC and one
replaceable pending request; superseded pending callers resolve promptly.
Identical metadata checks share the useful in-flight result. A stale `play()`
rejection cannot pause a replacement region or restarted transport. Native
preview admission coalesces equal requests, supersedes
obsolete workers, rechecks source identity after waiting for admission, and
serializes preview admission. Cancellation drains children and removes partial output
and sidecars. Normalized audio preparation uses the same cancellation signal;
cancelled preparation reports cancellation rather than an encoding failure.

The monitor also starts its frame clock when current media becomes playable,
even if its initial `play` event preceded metadata. The tutorial preserves its
step while Export is open, and focused buttons retain native keyboard activation.

## Reproduction and evidence

The explicit fixture contains 1,800 frames at 1920x1080p30, 21 clips on two video
and two audio tracks, an on-canvas inset, a title, fades/dissolves, linked audio,
automation and speed mapping. Sources have deterministic frame/time markers,
30 and 30000/1001 cadence, and 48 kHz pulse audio. All generation and decoding are
silent. The ordinary preview settings are 960x540, H.264 CRF 20/AAC 160 kbit/s,
with a half-second GOP and disabled scene-cut insertion. One shared preview
compiler supplies both native jobs and diagnostic plans. Every measured first-frame
and continuous file must report matching effective x264 quality/GOP settings in
its encoder SEI; a plan's intended settings alone are insufficient evidence.
Proxies are prepared separately at 1280x720, H.264 CRF 23.

`scripts/measure-preview-regions.mjs` loads the actual compiled production
controller, bridge, scheduler, coordinate helpers and playback asset queue.
Its headless Rust host calls the same ProjectStore and JobManager used by Tauri.
Timing starts before each real edit RPC and includes identity, intent, queue,
debounce, preparation and decoded-frame confirmation. Continuous readiness
includes exact frame count, cadence, duration, audio-stream and actual encoded-profile checks; decoded
full-reference correctness is a separate mandatory gate. It is not inferred
from cache equivalence or metadata alone.

Ten brightness edits cover the beginning, middle and end. p95 uses nearest rank;
with ten samples it is the maximum. First preparation starts with empty app
preview/audio caches; OS file caches are not forcibly cleared. Fixture/proxy
generation, startup hydration, native job intervals, encoded-asset availability,
decoded-frame availability, sampled working sets, process counts, cache bytes,
source/build/helper/input hashes and every sample are retained separately.

The shared filter compiler uses at most four available CPU filter threads,
falling back to one. A controlled one/two/four-thread comparison of the slowest
ordinary region produced byte-identical MP4 files and identical decoded RGB/PCM;
codec, quality, preset and GOP settings were held fixed. This adjustment improves
software rendering without a GPU requirement.

The preserved earlier four-video stress fixture has 20 clips, some footage
partly offcanvas and no source audio. Its original 240.198 s cold, 241.949 s edit
and 248.902 s proxy timings are historical builder measurements, not new native
display observations. Original source/output hashes are preserved and checked.

## Previously verified v5 known-fixture timings

These measurements use the corrected long-GOP source and pass the decoded
full-reference, regression and terminal file-hash gates below. The earlier
timings remain superseded evidence: their stress reference caught black and
truncated footage despite successful timing and metadata checks.

Measurements use the Ryzen 7 3700X (8 cores/16 logical processors), 16 GB RAM,
Windows 11 10.0.26200 and Node 24.15.0. All 33 preparations verify actual
CRF 20/GOP 15/scene-cut 0 files; x264 reports an effective minimum key interval
of 8 for the requested interval of 15. Before/after hashes confirm every measured
source, compiled helper, tool and fixture input remained unchanged during its run.

Times are seconds. First preparation has empty application preview/audio caches;
OS cache is not flushed. Warm p95 is the largest of ten actual edit samples.
Targets are 2 seconds to a silently decoded frame and 5 seconds to a completed,
inspected five-second asset. These are not monitor display or speaker endpoints.

| Workload | Initial frame | Initial 5s ready | Edit frame p95 | Edit 5s ready p95 |
| --- | ---: | ---: | ---: | ---: |
| originals | 0.834 | 2.557 | 0.890 | 3.241 |
| proxies | 0.685 | 2.147 | 0.676 | 2.830 |
| stress | 0.485 | 2.993 | 1.227 | 4.495 |

| Edit | Global frame | Originals: frame | Originals: 5s ready | Proxies: frame | Proxies: 5s ready |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 30 | 0.890 | 2.825 | 0.560 | 2.096 |
| 2 | 180 | 0.722 | 2.466 | 0.610 | 2.195 |
| 3 | 340 | 0.725 | 2.670 | 0.583 | 2.252 |
| 4 | 370 | 0.843 | 2.830 | 0.644 | 2.317 |
| 5 | 720 | 0.888 | 3.241 | 0.676 | 2.767 |
| 6 | 850 | 0.760 | 3.024 | 0.637 | 2.830 |
| 7 | 1010 | 0.718 | 2.799 | 0.635 | 2.431 |
| 8 | 1300 | 0.628 | 2.519 | 0.571 | 2.260 |
| 9 | 1610 | 0.677 | 2.476 | 0.610 | 2.246 |
| 10 | 1770 | 0.712 | 2.513 | 0.560 | 2.157 |

| Stress edit | Global frame | Decoded frame | 5s ready |
| --- | ---: | ---: | ---: |
| 1 | 30 | 0.700 | 3.406 |
| 2 | 180 | 1.141 | 4.264 |
| 3 | 340 | 0.909 | 3.933 |
| 4 | 370 | 1.029 | 4.098 |
| 5 | 720 | 0.981 | 3.966 |
| 6 | 850 | 0.990 | 3.914 |
| 7 | 1010 | 0.739 | 3.565 |
| 8 | 1300 | 1.227 | 4.495 |
| 9 | 1610 | 1.106 | 4.344 |
| 10 | 1770 | 0.826 | 3.941 |

Synthetic original media preparation was observed at 6.302 seconds in the initial
successful tool run. Its original manifest was replaced by later fixture lookups;
this is an earlier observation, not the current lookup manifest. Separate normalized
proxy preparation, retained in its own report, took 13.608 seconds. Startup hydration took
0.680 seconds for originals,
0.612 for proxies and
0.171 for stress; these costs
are outside the edit timer. Precise audio preparation is included in initial
preparation, rather than hidden in warm-edit figures.

Sampled process working sets and final application-cache bytes are decimal MB:

| Workload | Encoder peak | Rust host peak | Silent decoder peak | Final cache |
| --- | ---: | ---: | ---: | ---: |
| originals | 371.5 | 19.3 | 18.8 | 23.4 |
| proxies | 316.1 | 19.3 | 19.2 | 21.9 |
| stress | 494.8 | 19.0 | 17.6 | 4.5 |

A 20 ms sampler observed at most one encoder plus one silent frame decoder
at a time in each measured run. Every run ended with zero managed children and
zero partial files. These figures exclude the interface/Node process and FFprobe;
sampling can miss shorter peaks. Cache counts include clocks, thumbnails and
sidecars, separately from preview-record retention limits.

The identical-production-plan thread trial took 4.430/2.883/2.335 seconds for
one/two/four filter threads respectively. These are one warm encoder-only trial
per configuration. All three files were byte-identical to the retained native
production asset, with every decoded RGB frame and PCM sample identical.
The corrected final source produced the same MP4 bytes. Quality, preset,
GOP and codec were held fixed; source preparation and native display were outside
this comparison.

## Validation record and remaining limits

The previously captured v5 suite passed 63 active engine tests and 62 interface
tests. Four full-reference
gates cover 14 ordinary boundary/adjoining regions and 11 cumulative edited
states each with originals, proxies and long-GOP stress: 47 continuous regions,
33 actual timed first frames, 7,050 decoded grayscale frames and 11,280,000
stereo sample frames. Every continuous region has exactly 150 video frames and
240,000 stereo sample frames. Joined adjoining audio has no omitted or duplicate
coverage. Each edited state preserves the saved clip coordinates and export
range while preview rendering ignores that export range in a separate clone.

Maximum observed errors are grayscale RMS 1.874 (limit 5), sampled RGB 3.663
(limit 6), source-marker RGB 5.246 (limit 6), and audio 0.0001035 (limit 0.003).
The stress fixture is silent and its offcanvas footage lacks the ordinary source
marker; its checks establish visual correctness, while the ordinary fixtures and
engine clock tests establish audio and frame-phase behavior. No limit was relaxed.

The retained v5 terminal audit verified its source, binaries, generated helpers,
saved projects and original/proxy inputs against every before/after report.
All managed children drain and no partial files remain. The interface source and
build were unchanged from the captured 62-test, type-check and production-build
results. These v5 checks covered the listed known fixtures and were insufficient
for acceptance before the independent irregular-PTS counterexample received the
focused source-clock correction. Its later temporal/readiness acceptance is
scoped separately and retains the sharp-pulse codec-quality failure.
Native monitor and audible playback behavior remain unobserved.

The [machine-readable evidence](evidence/preview-regions.json) includes every
sample, reference metric, source/tool hash, retained failure and final audit.
It omits private filesystem paths and is tied to that exact v5 known-fixture
cohort. Raw media, project snapshots and diagnostic logs remain outside the
source checkout. Its SHA-256 is
`99c5260972af0067b8edcf2b192506a210f8a7a470a4a2894fff9ac219880d68`.

The stress reference caught missing media at global frame 180: first-frame RGB
RMS was 130.818 and continuous RGB RMS was 116.828. Retained controls changed
only the input seek-duration policy. Extending the input duration, accurate
seeking, and removing the video input duration all restored the picture; their
six lossless first/continuous controls matched the full reference exactly.
Accurate seeking failed the separate nonzero-origin clock regression. The
correction retains inexact seeking and the original timestamps, removes the
main-video input duration, and uses exact filter EOF to terminate decoding.
Looped still durations and precise PCM audio seeking retain their own bounds.
The new preview recipe also invalidates cached black or truncated interim files.
The active long-GOP regression checks a requested frame and 150-frame region at
both zero and nonzero stream origins, mixed cadence, split envelopes and speed
conversion. Lossless RGB matches the full reference byte for byte; native H.264
checks every frame, and silent PCM retains exact sample coverage and phase.
Upstream frame diagnostics verify decoding stops before the contributing source
end rather than consuming the entire source, including the required GOP preroll.

The earlier one-filter-thread candidates missed the 5-second target:
original/proxy edit readiness p95 was 5.855/5.461 seconds. The CRF 22 picture
check also failed one source-marker sample at RMS 6.015 against the original
limit of 6. Lossless controls matched all 150 RGB frames and 240,000 stereo
sample frames exactly, identifying compression error. Preview CRF changed to 20
while the numeric limits remained fixed.

Interim candidates labelled quality20 actually encoded CRF 22 through a duplicated
native settings path. Inspection of all 44 retained first/continuous files
confirmed the mismatch. Those candidates are preserved and excluded. Native
jobs, diagnostic plans and reference previews now use one shared profile compiler;
the new recipe prevents reuse of interim caches. A first GOP metadata correction
was also superseded after actual native MP4 inspection confirmed the older
JobManager already appended GOP 15. The final comparison uses the actual shared
profile without adding an override.

This stage remains a CPU-rendered region architecture. Playback pauses while
adjoining coverage prepares; seamless long playback, native display/resume timing,
audible output and direct GPU composition are unverified or unfinished. Cold
whole-source audio normalization and the slower oversized-audio prefix fallback
can remain expensive on long footage. Long GOPs add decoding before the requested
window; the measured synthetic workloads do not establish latency for every
source codec, GOP length or machine. No professional feature parity is claimed.


## Run the silent checks

Build the pinned media tools as described in the README, then run these commands
from the source root in a Windows developer terminal. Keep generated footage and
raw reports in the separate sibling directory. None of these commands plays sound.
Use an empty output directory for each timing run, and avoid concurrent encoders
or builds while measuring. Correctness rendering happens afterwards.

```powershell
$bench = Join-Path (Split-Path $PWD -Parent) 'mono-cut-preview-check'
$env:MONO_CUT_REGION_BENCH_DIR = $bench
$env:MONO_CUT_FFMPEG = Join-Path $PWD 'src-tauri/resources/media/ffmpeg.exe'
$env:MONO_CUT_FFPROBE = Join-Path $PWD 'src-tauri/resources/media/ffprobe.exe'
$env:MONO_CUT_FONT = Join-Path $PWD 'src-tauri/resources/media/Inter.ttf'
npm run test:preview
npm run check
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --tests -j 1 -- --test-threads=1
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_benchmark prepare_reproducible_ordinary_fixture_and_preserve_stress_evidence -- --ignored
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_benchmark prepare_normalized_proxies_for_ordinary_fixture -- --ignored
cargo build --locked --manifest-path src-tauri/Cargo.toml --example preview_region_driver -j 1
node scripts/measure-preview-regions.mjs --project "$bench/ordinary-60s.monocut" --output "$bench/originals" --mode originals
node scripts/measure-preview-regions.mjs --project "$bench/ordinary-60s-proxy.monocut" --output "$bench/proxies" --mode proxies
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_reference ordinary_regions_match_shared_full_reference_with_exact_adjoining_coverage -- --ignored --nocapture
$env:MONO_CUT_REGION_PRODUCTION_REPORT = "$bench/originals/controller-results.json"
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_reference timed_production_edits_decode_to_their_own_shared_full_reference -- --ignored --nocapture
$env:MONO_CUT_REGION_PRODUCTION_REPORT = "$bench/proxies/controller-results.json"
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_reference timed_production_edits_decode_to_their_own_shared_full_reference -- --ignored --nocapture
```

On Linux/macOS, set the corresponding media executable paths without `.exe`.
Recorded timings belong to the Windows machine below; they are not cross-platform
performance claims. Historical stress artifacts are optional and are not distributed
with the source. The ordinary fixture is generated entirely by the retained test.

The exact Windows-only thread comparison tool is retained as
[`scripts/compare-preview-filter-threads.py`](../scripts/compare-preview-filter-threads.py).
With Python 3, run it from the source root after ordinary timing and before
starting other encoders:

```powershell
py -3 scripts/compare-preview-filter-threads.py "$bench/originals/controller-results.json" "$bench/filter-threads"
```

It uses the recorded ordinary 30 fps/CRF 20 plan and its existing cache inputs;
only filter-thread count and output destination change. Keep its output outside
the source checkout. Its MP4/RGB/PCM equality check is separate from the
full-render accuracy gates.
