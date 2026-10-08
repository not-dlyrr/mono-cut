# Mono Cut project format, version 1

A `.monocut` file is UTF-8 JSON. `version` is currently `1`; unsupported versions
are rejected. A project references source media rather than embedding footage.
Keep the project and sources together or relink after moving them. Save writes a
temporary sibling, syncs it, and renames it into place. Recovery is written to
the app data directory after successful edits.

Saved project and recovery JSON have the same inclusive 64 MiB limit
(67,108,864 encoded UTF-8 bytes). Escaped text and JSON formatting count toward
this budget. Writers validate the model and bound its actual serialization
before replacing any file; save checks both its relative-path document and its
absolute-path recovery snapshot first. An operation that exceeds the budget is
rejected without changing the project, undo/redo, or previous project/recovery
files. Shorten title text or remove unused clips/media to reduce the size.
Readers limit the actual input stream as well as checking file metadata.
Compact external JSON that expands past the canonical recovery budget cannot
be accepted into the store. Nonfinite waveform values are also rejected because
JSON cannot preserve them as readable numbers.

## Time conventions

`{ "num": 30000, "den": 1001 }` is a rational frame rate. All `start`, `duration`,
marker, in/out, fade and keyframe frame values are integer sequence frames.
Intervals use an inclusive start and exclusive end. Source in and media duration
are rational seconds; speed is a positive rational multiplier. For a clip of `n`
frames at rate `f`, its consumed source interval is `n / f * speed`. The renderer
converts inputs to the sequence rate, then converts that result to export fps.

| Project field | Meaning |
| --- | --- |
| `version`, `id`, `name` | Format version, stable identifier and display name |
| `width`, `height`, `fps`, `sample_rate` | Sequence dimensions, rate and audio rate |
| `media` | Source metadata and file references |
| `bins` | User-created media folders |
| `tracks` | Ordered video/audio tracks with mute/hide/lock state |
| `clips` | Timeline instances, effects and links |
| `markers` | Named sequence-frame positions |
| `in_point`, `out_point` | Optional export range, exclusive out |

## Media

Each media item has `id`, `name`, `path`, `kind` (`video`, `audio`, `image`),
`duration`, `fps`, `width`, `height`, `has_audio`, `bin_id`, `thumbnail`, `waveform`,
`proxy`, and `missing`. On save, source/proxy paths are relative to the project
when they share a filesystem root. Cross-drive references can remain absolute.
Thumbnail paths are cleared in saved files and regenerated on reopen. Waveform
values are retained. `missing` is recomputed when resolving source paths. A
missing proxy falls back to the original source. Relinking preserves media IDs,
bin assignment and timeline references while replacing probed metadata.
Media `width` and `height` describe square-pixel display dimensions after supported
quarter-turn metadata rotation, rather than the source's coded buffer dimensions.
Reopening refreshes these dimensions alongside thumbnails and waveforms. Source
files retain their original encoded pixels and metadata.

Optional `timing` and `proxy_timing_version` fields are additive in version 1.
`timing` records one rational source origin and the selected streams' absolute
starts, known ends, and indexes. Relative audio/video delays are preserved on that
shared clock. A normalized proxy uses origin zero while retaining the original
stream interval for compositing. Old documents omit these fields; available
sources are reprobed on open/recovery, preserving IDs, edits and saved duration.
Missing sources remain editable and gain timing when relinked. Earlier proxies
without the normalization marker are invalidated. See
[source timing](source-timing.md) for the conversion and migration rules.

The optional `legacy_source` record contains the measured `duration` and a
content `sha256`. Eligible old projects that saved an absolute Matroska end keep
their larger logical `media.duration`, preserving existing clips and their
black/silent tail. Relink verifies the recorded source identity and timing before
retaining that extent. New imports use actual measured bounds. Already-missing
old records can infer the narrow historical pattern on first relink; original
byte identity is unavailable until adoption records a hash. The full
compatibility rule and limits are documented in the source timing reference.

## Tracks and clips

Track fields are `id`, `name`, `kind`, `muted`, `hidden`, `locked`. Higher video
track positions are composited over lower positions. Lock prevents edit commands;
mute suppresses audio and hide suppresses video.

Clip fields are `id`, `media_id`, `track_id`, `name`, `start`, `duration`,
`source_in`, `speed`, `linked_id`, `title`, `transform`, `opacity`, `volume`,
`fade_in`, `fade_out`, `brightness`, `contrast`, `saturation`, `keyframes`.
Title clips use `media_id: null` and store their text in `title`. Transform uses
pixel `x`/`y`, scale multiplier, degree rotation, and fractional
`crop_left/right/top/bottom`. Overlapping clips with opacity fades produce basic
cross dissolves; there is no separate transition-object type in v1.

Keyframes are `{property, frame, value}` at clip-local sequence frames.
Properties currently accepted are `opacity`, `volume`, `x`, `y`, `scale`.
Values interpolate linearly and hold the nearest endpoint outside their span.
Keyframes may include a control point at the exclusive out (`frame == duration`)
to preserve interpolation through the last frame's audio samples. Optional
`fade_in_start`, `fade_out_end`, `composition`, and `render_offset` fields retain
envelopes and conversion clocks through cuts/trims. Absent defaults preserve older
version-1 behavior and are omitted from JSON. Rules, bounds, explicit fade editing
and trim extensions are documented in [cuts and inherited envelopes](split-envelopes.md).
Links group related clip instances; unlinking an audio-bearing video can create a
separate audio instance rather than modifying the source file.

Unreleased source adds optional `clip.retime` to version 1. Its `source_span`
retains the exact rational selection, even when its current visible duration
has a fractional frame remainder. `envelope` contains rational source-progress
fade durations/anchors and keyframes `{property, time, value}`. Integer clip
effect fields are projections for the interface; contradictory projections are
rejected. `render_source_origin` retains the exact source conversion clock
through speed changes and cuts, including signed continuation phases after a
trim extension. Optional `composition_source_offset` encodes the existing
timeline ordering anchor at the current speed; it is rescaled on retime so the
anchor and layer order stay fixed. Absent `retime` preserves legacy clip/envelope
semantics; the shared renderer still uses the current preview recipe. See [Speed policy and validation](retiming.md) for rounding, linked
transactions, collision rejection and source-stage limits.
Older builds do not understand this canonical state and can discard it when
resaving. Use a build from the current source for projects with `retime`.

The Rust validators define exact ranges and reference constraints. Use editor
commands for modifications; manually edited invalid references, oversized files,
negative durations or unsupported versions are rejected with errors.

## Recovery and history

An autosaved recovery snapshot is separate from explicit project save. Recovery
can restore the last committed model after a crash or interrupted session.
Undo/redo is in-memory and is cleared when opening a project or recovering; it is
not serialized as part of v1. A saved project does not package source footage,
preview caches, custom UI preferences or shortcut mappings.
