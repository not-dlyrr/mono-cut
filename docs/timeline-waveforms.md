# Timeline waveforms

Easy and Advanced now draw the audio contained in the edited clip, including its
source-in and speed. Moving a clip preserves that source interval. Trimming or
slipping previews the proposed interval immediately; cancelling the pointer
gesture restores the original curve. Linked partners follow the same proposed
source change as the native editing command. Unrelated selected clips do not
follow a trim or slip. The Slip tool is available in Advanced; Easy's inspector
slip controls update the same project model.

This implementation ships in [0.1.2](release-0.1.2.md). The stage measurements
below retain their original source snapshots; native gestures were not observed
for this package. The older 0.1.1 release remains intact.

## Source clock and resolution

Native import already generates 1,024 maximum-amplitude bins from a mono,
4,000 Hz decode on the common source clock. It inserts leading silence when
audio starts after video. The interface consumes those bins without decoding
PCM or applying the stream offset a second time.

For source duration `D`, let `E = ceil(D * 4000)` and `N` be the bin count.
Bin `j` owns samples `[ceil(j*E/N), ceil((j+1)*E/N))`. A clip-local sequence
time `t` maps to `source_in + t * speed`. The drawing uses explicit half-open
source intervals, exact rational project timing and decimal pixel geometry.
It limits coverage to the visible clip duration, including a retime's rounded
frame endpoint; retained fractional source tails do not extend the drawing.
Coverage outside the media duration is silent.

Each display bucket takes the maximum of **every** intersecting source bin.
This preserves brief peaks that the former every-tenth-bin sampling discarded.
The source bins cannot locate a peak within a bin. A trim cutting through that
bin may still show a nearby excluded peak; precision is bounded by the source
bin width divided by speed plus one display bucket. This is a source waveform,
before volume, fades, mute or mixing, rather than a loudness meter. Mono
downmixing can cancel opposite-phase channels.

## Bounded rendering

The timeline continues to render only visible clips, with 100 pixels of overscan
at each side. Waveform SVGs cover that visible window rather than the entire
length of a large clip. Buckets target two pixels and are capped at 512 per
visible clip. Source bin storage is capped at 1,024; malformed oversized arrays
are suppressed. Work is `O(N + P log N)` and storage `O(N + P)` per visible clip,
where `P <= 512`. This is a bound on work, not a measured native playback rate.

Curves are memoized against project, drag, zoom and scroll changes. Playhead-only
updates reuse the curve. Direct timeline movement has no decorative easing.
Existing colors, focus styles, themes and preview ownership remain unchanged.
No media cache recipe, renderer, audio levels or project-format field changed.

## Silent validation

[Recorded evidence](evidence/timeline-waveforms.json) links source, helper,
tools, fixtures and correctness results by SHA-256. The interface suite passes
125 tests, including 12 waveform/helper and actual compiled Timeline component
checks. They cover live trim-in/out, Advanced slip, cancellation, linked
membership, half-open boundaries, excluded peaks and viewport bounds. These
checks execute the component with controlled hooks and events without a DOM.

The file-only media verifier exercises actual native import hydration,
ProjectStore commands, preview plans and file rendering in 25 cases. A synthetic
lossless source has two brief tone pulses, a 30000/1001 video stream, a common
origin of three seconds and audio starting 0.2 seconds later. The first pulse
falls between bins skipped by the old algorithm. All 1,024 imported peaks match
independently decoded float bin maxima exactly.

Both sequence rates (30 and 30000/1001) cover full clips, selected tail trims,
trim-out, slips, split fragments, 2x/0.5x/7:6 speeds, fractional source-in and
leading padding. Predicted curve boundaries agree with decoded audio from the
actual native render within each case's declared source-bin/display-bucket
resolution. Proxy rendering uses a generated native H.264/AAC proxy; switching
to it preserves the source waveform exactly. Actual undo/redo and fresh-process
save/reopen preserve the mapped component curves. No gain normalization or
relaxed codec-quality gate is used.

The focused native retime, source-timing and bounded-region suites also pass
25 tests. All 32 accepted native source/test/build hashes remain unchanged.
Earlier preview/retiming evidence is preserved. The first verifier attempt
requested an unsupported 90-pixel preview height and failed before rendering;
that capture is retained. The corrected verifier requests the documented
minimum 120 pixels. An intermediate 13-case pass preceded the final 25-case
matrix; it is retained separately.

No native window, browser, player, installer or audio device was opened.
Native pointer capture, displayed pixels, focus, speaker behavior and playback
or scrubbing performance are unverified in this stage. Existing sharp-pulse
AAC fidelity and legacy split float diagnostics remain separate limitations.

## Reproduction

Run `npm ci`, prepare the pinned media tools, then `npm run test:preview` and
`npm run build`. Build the existing `preview_region_driver` example with the
Windows x64 toolchain. Use a disposable version-1 project template containing
V1/A1 tracks and one imported media item; no personal footage is needed by the
verifier. Pass a fresh output directory outside the checkout:

```powershell
node scripts/verify-timeline-waveforms.mjs <repository> <native-driver> <template.monocut> <output-directory> <native-source-freeze.json>
```

The verifier intentionally requires the recorded native source freeze and driver
hash. A future engine revision needs a newly validated driver/freeze rather than
silently accepting another binary. It creates synthetic media, compiles the
actual Timeline and writes raw files, plans and RPC captures outside the checkout.
Only the curated evidence belongs in published source; raw captures contain
machine-specific paths.

The freeze is the `native_source_hashes` object in the published evidence. Export
that object as the final argument's JSON file. The recorded driver build command
is `cargo build --locked --manifest-path src-tauri/Cargo.toml -j 1 --example
preview_region_driver` using the Windows x64 toolchain and the unchanged sources.
Compiler/toolchain differences may change the executable hash; validate such a
build and record its provenance before updating the verifier's binary pin.
