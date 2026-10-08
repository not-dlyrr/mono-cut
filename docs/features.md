# Feature status

Mono Cut 0.1 is an early desktop editor foundation. It does not claim feature
parity with Premiere Pro or Resolve Studio. Runtime evidence belongs in
`validation.md`; the table below describes implemented interfaces and semantics.

The table describes published 0.1.1. Unreleased source adds requested-frame and
five-second program regions; its performance and decoded-media checks are in
[the region preview record](preview-regions.md). Unreleased source now prepares
one successor while playing and promotes its loaded media node when ready.
An uncovered boundary waits for the matching successor; explicit Stop or edits
supersede that resume. See [continuity and its validation scope](preview-continuity.md).
Native display and audible boundary behavior have not been observed in this stage.
The source-stage Speed repair uses transactional retiming in both workspaces,
retaining the selected source interval and linked A/V while rejecting collisions.
Its rounding, source-relative envelopes and validation scope are documented in
[retiming](retiming.md). It has not updated the published installer.
Unreleased timeline waveforms now follow edited source intervals and speed in
both workspaces, with visible-window max aggregation and live trim/slip previews.
See [waveform timing and validation](timeline-waveforms.md); these are coarse
source peaks before clip effects, and native gesture behavior remains unverified.
Unreleased monitor shortcuts follow the activated Source or Program monitor in
both modes. [Keyboard ownership](monitor-keyboard.md) documents focus, tabs,
target reset rules, widget protection and silent source/component validation.

| Capability | 0.1 status and limits |
| --- | --- |
| Easy mode / tutorial | Default CapCut-style workspace, first-run seven-step tooltips, restart through Help; Advanced restores dual monitors and keyframes |
| Import video/audio/images | Native FFprobe metadata, thumbnail and waveform generation |
| Bins / missing media | Create/assign bins, detect missing files, relink preserving IDs |
| Timeline | Video/audio tracks, visible-range clipping, zoom, snap, markers, lock/mute/hide |
| Editing | Move, split, trim, ripple delete, duplicate, link/unlink, basic slip; undo/redo |
| Monitors | Source preparation and validated cached program preview; synchronized forward audio; metadata/save changes reuse a completed preview |
| Frame stepping / scrub | Seek cached preview using rational sequence-frame coordinates |
| In/out / J/K/L | Source and sequence ranges, forward shuttle, stop, reverse seek without audio |
| Clip effects | Transform, crop, opacity, positive speed, volume, fades, brightness/contrast/saturation |
| Keyframes | Linear opacity, volume, x, y, scale; exclusive-out control points retain interpolation through audio samples |
| Titles / dissolve | Inter text titles and overlapping opacity fades |
| Project IO | Documented version-1 JSON, save/open, 100-state history, autosave/recovery |
| Proxy / preview resolution | Background H.264 proxies, original/proxy switch, reduced-resolution preview |
| Export | MP4 H.264/AAC and MKV FFV1/FLAC, dimensions/fps/quality/audio settings, progress, cancellation |
| Appearance / accessibility | Dark/light, glass toggle, reduced motion, focus states, shortcut remapping |
| Hardware support | Software CPU media pipeline; system capabilities reported; no GPU vendor requirement |

Cuts preserve existing fades, automation and paired dissolves; trims crop their
retained envelopes. Explicit fade edits anchor a new ramp to the edited fragment.
See [inherited envelope semantics](split-envelopes.md) and
[decoded-media checks](split-envelopes-validation.md).

Published 0.1.1 program playback requires a completed background preview render.
Its refresh cost grows with sequence length and effect complexity. H.264 preview uses lossy
compression and can look different from a lossless FFV1 export; both follow the
same edit/effect graph. Audio is mixed to stereo. FFV1 export currently outputs
8-bit YUV 4:2:0, so “lossless” refers to encoding that rendered format rather than
preserving every source color channel or bit depth.

Completed previews are keyed by render inputs and filesystem stamps, with
bounded records and active-file protection. Metadata changes preserve a correct
loaded preview; published 0.1.1 edits still rebuild the complete sequence. Unreleased
source instead prepares a requested frame and five-second regions. See
[preview reuse](preview-cache.md) and its validation record.

Video display dimensions account for non-square pixel aspect ratios and 90/180/270
degree display-matrix rotation. Source caches and proxies normalize to square
pixels; preview and export fit the same display geometry. Arbitrary non-quarter
metadata rotation and display-matrix shear are not supported; normalize those
sources externally before import. The clip rotation control remains available
for ordinary editing transforms.

## Not implemented

Nested sequences, multicam, scopes, curves, LUTs, masks, tracking, advanced
compositing, channel routing/buses, professional loudness tools, captions,
interchange formats and the plugin API are development goals. HDR/wide-gamut
color management, optical-flow retiming, reverse audio and direct GPU playback
are also not present. Speed changes currently use varispeed and change audio
pitch; pitch-preserving time stretch is not implemented. No inactive controls
pretend to offer these features.

## Incremental development order

1. Incremental native frame/audio playback and measured long-timeline behavior.
2. Color management and scopes; curves/LUTs with a shared preview/export path.
3. Audio buses, meters and loudness; captions and interchange.
4. Nested sequences and multicam with versioned project migrations.
5. Masks/compositing, tracking and a documented sandboxed plugin boundary.
