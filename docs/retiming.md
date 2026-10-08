# Speed and retained source intervals

The Speed control shipped in [0.1.2](release-0.1.2.md) uses one `retime_clip` transaction in both Easy
and Advanced workspaces. A request supplies a clip ID and a positive rational
speed. Titles and still images have no changing source clock; their Speed
control is unavailable. Trim their duration instead. Audio uses varispeed,
including a pitch change. Reverse and pitch-preserving stretching remain on
the roadmap.

## Timeline policy

The first retime retains the exact selected source span:
`source_span = duration / sequence_fps * old_speed`. Each accepted request sets
`duration = floor(source_span / new_speed * sequence_fps)`. Start and source-in
stay fixed. A 480-frame, 30 fps clip becomes 240 frames at 2× or 960 at 0.5×.
There is no source clamp or automatic ripple. A result shorter than one frame,
outside model limits, or beyond available source media is rejected. The
fractional remainder is retained in the project rather than discarded: a later
speed change or return to 1× computes from the same exact span.

Linked members are planned and committed together. They must already share
their timeline start and speed; different source-in values remain different.
Offset or differently sped linked groups require alignment or unlinking before
retiming. An unavailable source or a locked member rejects the whole operation.
Unrelated clips remain fixed. A new same-track overlap, or enlargement of an
existing overlap, rejects the request. Existing intentional overlaps may shrink.
Undo and redo cover the entire transaction, and saving/reopening retains its
accepted model. A rejected operation must preserve history and recovery too.
An explicit in/out range that becomes invalid is also rejected; change the
range first rather than letting a speed request silently move it.

The committed text field accepts finite decimal speeds from 0.05× through 32×.
Enter or leaving the field commits a complete valid value; Escape restores it.
Partial, out-of-range or excessive-precision values produce guidance without an
edit. Native rational bounds also apply. A generic `update_clip` containing only
`speed` uses the same operation; mixed speed/property patches are rejected so a
caller cannot bypass the transaction.

## Source-relative envelopes and discrete rendering

Optional version-1 `clip.retime` stores `source_span`, `envelope`,
`render_source_origin`, and optional `composition_source_offset`. Envelope keys
use `{property, time: Rational, value}` in source-progress seconds relative to
the clip's source-in. Fade durations and anchors use the same units. Existing
integer frame fields are editor projections, not the renderer's canonical
values after retiming. Coincident projected controls may collapse in that view;
the exact controls remain in the canonical envelope. Editing one envelope
property changes that property without rounding other properties through the
view. Cuts and trims rebase the canonical envelope and preserve continuation
anchors. A split's right member retains the fractional tail.

The shared preview/export compiler converts canonical envelope times by the
current speed. It retains the inherited exact source conversion origin and
timeline composition anchor. `composition_source_offset` is rescaled with the
speed to keep that ordering anchor fixed, so retiming a cut cannot move it above
an overlapping layer. Where FFmpeg's timebase and numeric limits permit, the
retimed video branch uses integer ticks for the source frame rate, conversion
origin and rational speed. Otherwise it uses the previous microsecond clock.
At the final discrete video grid, the inherited phase is
rounded to the nearest sequence frame, with half ties away from zero. Decoded
frame selection therefore has normal frame-rate conversion quantization;
retiming does not promise a frame at every fractional source timestamp. Video
uses the existing [FFmpeg FPS conversion](https://ffmpeg.org/ffmpeg-filters.html#fps-1)
timestamp grid, including its drop/duplicate decisions at rounded timestamps.
Audio sample boundaries round upward as in the existing shared renderer. The basic
audio converter rounds `sample_rate * speed` to a whole sample rate; arbitrary
high-precision speeds can therefore have a small cadence difference from the
exact video multiplier. Neither audio pitch preservation nor arbitrary-ratio
sample accuracy is claimed.

Clips without `retime` retain their legacy editing model and envelope semantics.
The shared sample-clock correction described below also applies to them. The metadata participates in
preview identity, so accepted retimes invalidate current and successor assets;
stale readiness events cannot restore stopped playback. Old saved projects need
no migration until a speed change is requested.

## Validation scope

An actual inherited-fade test exposed a separate preexisting v7 clock residual:
2,046 float values differed between its bounded and full graphs (1,023 stereo
sample frames; maximum 14 ULP, maximum absolute difference about `9.686e-8`).
Old and new retime graphs produced the same residual. A causal control changed
only the post-trim reset from `asetpts=N/SR/TB` to `asetpts=N` under the already
explicit `1/48000` timebase; the compared 480,000 float values then matched
exactly. V8 adopts that reset for the shared renderer and invalidates v7 caches.
It changes neither source levels, gain curves, media, ranges nor codec profile.
Previously exported automated samples can round differently. This counterexample
is distinct from the retained older two-sample split diagnostic and sharp-pulse
AAC fidelity failure; those historical captures remain unchanged.

Validation is silent: media is encoded and decoded to files, with no player,
native app, speaker output or installer launch. The preceding Speed defect was
reproduced through the actual `ProjectStore` before the replacement. The historical
stage measurements below retain their original source snapshots. This correction
now ships in 0.1.2; native audiovisual continuity remains unverified.

The final media matrix passes all 18 cases: each scenario below runs with
originals and with actual generated proxies, using the same frozen renderer.
Each case checks the exact retained interval, duration, fixed start/source-in,
linked members, actual undo/redo, saved/reopened clips and repeated retiming.
All 88 relevant native engine/persistence tests pass with zero failures or
ignored tests, and all 113 interface checks plus the interface build pass.
The eight new interface checks exercise the actual compiled Inspector callbacks
and controller/bridge with injected native calls. They do not establish native
focus or keyboard presentation. The native suite uses locked dependencies and
verifies source/tool hashes before and after execution. Its focused retime,
grid, inherited-clock and cache/profile checks are included in these 88 tests.
The sanitized source hashes, results, prior failures and reproduction provenance
are retained in [the machine-readable verification record](evidence/retiming.json).

| Scenario | Speed |
| --- | --- |
| Full selected source | 2× |
| Full selected source | 0.5× |
| Trimmed source interval | 2× |
| Trimmed source interval | 0.5× |
| Fractional source-in of 7/13 seconds | 7/6× |
| 30000/1001 source in a 30 fps sequence | 7/6× |
| 30000/1001 sequence | 7/6× |
| Inherited trim, fades, volume keys and subsequent split | 7/6× |
| Return to normal speed with negative inherited fade anchors | 1× |

Across the requested regions, lossless decoded RGB matches the full references
byte for byte, all 8,384,960 compared float PCM values match exactly, and matching
AAC encodes decode identically. The worst all-frame grayscale RMS is 1.6991,
worst individual grayscale-frame RMS 2.0011, selected RGB/marker RMS 4.8873 and
ordinary native-AAC versus full-FLAC RMS 0.00008449. The original limits remain
5, 5, 6 and 0.003 respectively. Native preview retains CRF 20, GOP 15,
effective minimum key interval 8 and disabled scene cuts. Independent baked
source markers and source-only audio controls verify frame selection, pulse
bounds and zero sample lag. These results cover the captured regions and
synthetic sources, not arbitrary codecs, sources or long editing sessions.
The independent Python gain calculation is diagnostic: its returned-1×
envelope differs from FFmpeg by at most `9.3133e-10` for originals and
`3.7253e-9` for proxies. These values are retained, without an acceptance
epsilon or normalization; the production bounded/full audio still matches
exactly in both cases, as do the independent pulse boundaries and zero lag.

Originals use helper v4; proxies use v5. The original audio-oracle function is
byte-identical between them. The first nine v4 proxy attempts failed their
additional oracle because it decoded original PCM while the renderer used
proxy AAC. Their production picture, float, matching-AAC and 0.003 checks passed,
but those attempts remain failed evidence. V5 probes and decodes the actual
proxy input independently; it preserves original-versus-proxy compression
differences as diagnostics. AAC ringing changes some threshold pulse bounds,
so original/proxy samples are not claimed to match exactly. No lag fitting,
gain normalization, source-level change or tolerance increase is used. The
proxy source cache has no limiter; the existing program limiter is accounted
for in both independent controls. Both final runs close their driver with zero
managed children and no partial files.

The retained sharp-pulse AAC fidelity failure remains documented in
[continuity validation](preview-continuity.md). No new playback or scrubbing
performance benchmark, native keyboard/focus test, WebView presentation or
audible handoff observation is claimed by this matrix.

## Reproducing the silent checks on Windows

First follow the README's locked dependency and media-build setup. These optional
verification tools require Python 3 and NumPy (the recorded runs use Python
3.14.5 and NumPy 2.4.6). They do not run the desktop app. Run the following from
the checkout root, using a fresh directory outside the checkout for synthetic
media, local project paths and raw captures:

```powershell
$taskRepo = (Get-Location).Path
$taskData = Join-Path (Split-Path $taskRepo -Parent) ('mono-cut-retime-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))
$env:MONO_CUT_FFMPEG = Join-Path $taskRepo 'src-tauri\resources\media\ffmpeg.exe'
$env:MONO_CUT_FFPROBE = Join-Path $taskRepo 'src-tauri\resources\media\ffprobe.exe'
$env:MONO_CUT_FONT = Join-Path $taskRepo 'src-tauri\resources\media\Inter.ttf'
$env:MONO_CUT_REGION_BENCH_DIR = Join-Path $taskData 'base'

py -3 -m pip install numpy==2.4.6
cargo test --locked --manifest-path src-tauri/Cargo.toml --test region_benchmark prepare_reproducible_ordinary_fixture_and_preserve_stress_evidence -- --ignored --exact --test-threads=1
cargo build --locked --manifest-path src-tauri/Cargo.toml --example preview_region_driver

$taskProject = Join-Path $taskData 'base\ordinary-60s.monocut'
$taskOffsetDir = Join-Path $taskData 'offset'
$taskDriver = Join-Path $taskRepo 'src-tauri\target\debug\examples\preview_region_driver.exe'
py -3 scripts/generate-retime-offset-fixture.py --project $taskProject --output $taskOffsetDir
$taskOffset = Join-Path $taskOffsetDir 'source-1080p30-offset.mkv'
py -3 scripts/verify-retiming.py --repo $taskRepo --driver $taskDriver --project $taskProject --offset-source $taskOffset --output (Join-Path $taskData 'originals') --mode originals
py -3 scripts/verify-retiming.py --repo $taskRepo --driver $taskDriver --project $taskProject --offset-source $taskOffset --output (Join-Path $taskData 'proxies') --mode proxies
```

Each verifier invocation runs nine cases. `--cases` accepts a comma-separated
subset for diagnosis. `--ffmpeg-source` optionally records the exact FFmpeg
source tree's FPS, timebase and timestamp implementation hashes. The verifier
checks actual native edits, saves and reopens, renders previews and full
references, then decodes their contents. Proxy mode generates its own proxies.
Neither the verifier nor fixture generator opens an audio output device.

The ordinary fixture has baked frame/time markers at 30 and 30000/1001 fps,
16-second source spans and low-level gated stereo tones. The offset generator
remuxes the original with video shifted by 3 seconds and audio by 3.2 seconds;
it verifies every encoded packet payload and all decoded source audio samples
remain unchanged. These fixtures provide repeatable alignment checks rather
than a loud-source codec stress test.

The public verifier is the exact tested helper. The offset generator exposes
the tested remux and packet/PCM checks through a public CLI; its CLI was checked
with `--help`, while the retained fixture was generated by the earlier private
entry point. No additional media run is claimed for that CLI wrapper.
