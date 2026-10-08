# Feature status

Mono Cut 0.1 is an early desktop editor foundation. It does not claim feature
parity with Premiere Pro or Resolve Studio. Runtime evidence belongs in
`validation.md`; the table below describes implemented interfaces and semantics.

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

Program playback requires a completed background preview render. Preview refresh
cost grows with sequence length and effect complexity. H.264 preview uses lossy
compression and can look different from a lossless FFV1 export; both follow the
same edit/effect graph. Audio is mixed to stereo. FFV1 export currently outputs
8-bit YUV 4:2:0, so “lossless” refers to encoding that rendered format rather than
preserving every source color channel or bit depth.

Completed previews are keyed by render inputs and filesystem stamps, with
bounded records and active-file protection. Metadata changes preserve a correct
loaded preview; actual edits still rebuild the complete sequence. See
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
