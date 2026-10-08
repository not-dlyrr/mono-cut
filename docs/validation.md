# Validation record — 0.1.0

This page retains the earlier 0.1.0 native workflow and source follow-ups. Current
[0.1.2 package/publication validation](release-0.1.2.md) records exact-commit
Windows/Linux/macOS CI, static installer checks and all six rehashed public
downloads. It does not repeat native UI or fresh-install validation. Later
corrections described below now ship in 0.1.2; their earlier stage measurements
and failures remain historical evidence.

The editor was exercised inside its actual Windows Tauri/WebView2 application,
with real, generated video, audio and image files. Test playback was muted;
audio timing was checked by decoding exported audio samples. Test fixtures and
personal local paths are excluded from releases.

## Verified behavior

- Imports probe actual metadata and produce image thumbnails and audio waveforms.
  The native workflow used two video frame rates, a 44.1 kHz WAV and a PNG.
- Real multitrack edits include move, split, trim, slip, link/unlink, titles,
  layered video, color, fades, bins, markers, undo and redo.
- Relative project paths survive save/reopen. Missing media is identified and
  relinked. A failed hydration preserves the previous project and undo history.
- Linked clip moves preserve separate audio/video tracks. Shortening a sequence
  clamps its export range; undo restores that range.
- Original and proxy previews render through the same filter compiler as exports.
  Quarter-turn metadata rotation and non-square pixel aspect ratios preserve
  display geometry in source previews, proxies and exports.
- MP4 and FFV1/FLAC exports use the real local encoder. Cancelling an export
  removes its unfinished output.
- Easy mode is the first-launch default, with a single Clip/Timeline monitor.
  All seven tutorial steps, dismissal persistence, Help restart and Escape were
  checked in the native application. Advanced mode displays both monitors.
- Negative decimal inspector edits, undo, save-before-replacement cancellation,
  Space/Pause/Space and keyboard stop were checked through the actual interface.
  Light/dark themes and layouts at 1280×720 and 1920×1080 were visually inspected.

## Accuracy and performance

Engine regression fixtures independently check trim frame barcodes, rational
mixed-rate conversion, title escaping, fades and animated properties. A
30000/1001 source produced exactly 48 frames in the tested 30 fps range. An
exported audio pulse landed at sample 19200 (0.400 seconds at 48 kHz); the
varispeed pulse check stayed within eight samples. A paired 12-frame dissolve
gave midpoint red/blue levels 125/126 versus 127 expected, without a black dip.
Geometry fixtures produced zero RGB RMS difference between original and proxy
FFV1 renders; lossy color-bar proxy comparisons were approximately 6.6 RGB RMS.

An initial native seven-second, 640×360 workflow imported four files in about
2.0 seconds, reopened in 0.7 seconds, rendered its preview in 1.4 seconds and
exported in 1.3 seconds. A 1.8-second playback sample displayed 54 frames with
zero reported dropped frames. These are short synthetic fixtures on the local
Ryzen 7 3700X workstation, not long-form or 4K performance guarantees. Preview
keyframe spacing was subsequently shortened for scrubbing; final release
measurements are recorded below after installer validation.

## Limits

Program playback currently requires a background render of the whole sequence.
Editing a long sequence therefore has substantially higher preview refresh cost
than a professional editor with incremental decoding. Cache retention is bounded
to 2 GiB outside protected in-use files, and timeline clips are culled outside
the visible horizontal range. Long-form, high-track-count, 4K/8K, unusual camera
formats, hardware acceleration and speaker-output quality are not validated by
these short tests.

Exports are stereo and 8-bit YUV 4:2:0; there is no HDR/wide-gamut color management.
Speed changes affect audio pitch. Nested sequences, multicam, scopes, LUTs,
curves, masks, tracking, captions, advanced audio buses and plugins are not
implemented. Windows requires its installed WebView2 runtime; Mono Cut does not
redistribute that proprietary runtime. Linux/macOS CI jobs are defined, but
those operating systems have not received a verified native installer release.

The source contains the actual-media engine tests and native diagnostic scripts
used for these checks. Native scripts attach to a deliberately enabled local
WebView2 diagnostic port; that port is not enabled in normal installations.

Subsequent source reliability work is recorded in
[the persistence boundary validation](persistence-validation.md). Its seven
generated regression tests and the nine existing actual-media tests passed.
That record distinguishes the source fix from the earlier installer validation.

The subsequent shared source clock correction is recorded in
[source timing validation](source-timing-validation.md). Five new decoded-media
regressions, two metadata tests, all nine existing engine tests and all seven
persistence tests passed. The earlier installer does not include those source
corrections yet.

A focused historical-duration follow-up adds two legacy relink regressions.
Genuine 5.2 s old edits now recover their identified 2.2 s source without changing
the 156-frame timeline or its effects. All 25 source tests passed silently;
the source timing report documents identity checks and missing-first inference.

The later cut/envelope correction is recorded in
[split envelope validation](split-envelopes-validation.md). It preserves fades,
linear automation, source conversion cadence and paired-dissolve layering through
cuts and retained trim spans, with per-sample gain and fractional-frame sample
boundaries. All checks in that stage decode files silently. The earlier installer
also predates these corrections.

The later scheduling and cache correction is recorded in
[preview reuse validation](preview-reuse-validation.md). It reuses completed
program renders across metadata/save/history changes, protects active cache
assets and rejects stale requests. Forty-seven Rust checks and 28 interface
state tests passed silently, together with an explicit eleven-case benchmark
and independent output checks. The measured full-minute, twenty-clip timeline
still needs about four minutes for a cold preview or real edit; reuse takes
about 4–6 ms through the engine API. These are source-engine measurements, not
renewed native UI or installer validation. The follow-up also cancels obsolete
work when Undo restores a loaded cache and rejects delayed prior requests.

Current installer/source packaging, notice and privacy checks are recorded in
[the 0.1.0 release record](release-0.1.0.md). That record distinguishes static
package integrity from the earlier native UI workflow and current silent engine
validation.
