# Cuts, trims and inherited envelopes

Split is a structural edit. It preserves the rendered picture and sound over
the original interval, including fades, dissolves and linear automation. A cut
does not restart a fade, shorten its ramp, or flatten the last frame's audio.
Trim crops the same envelopes over the retained interval.

## Additive version-1 fields

Untouched clips omit the fields below. Older version-1 JSON loads with its
existing edge-relative fades, source clock and timeline stacking order.

| Optional clip field | Meaning when present | Default when absent |
| --- | --- | --- |
| `fade_in_start` | Signed local frame where the fade-in ramp starts | `0` |
| `fade_out_end` | Signed local exclusive frame where the fade-out ramp ends | `duration` |
| `composition` | `{group_id, offset}` preserving the original layer's ordering | Clip ID, offset `0` |
| `render_offset` | Signed sequence-frame offset from the inherited source conversion clock | `0` |

Fade lengths remain in `fade_in` / `fade_out`. At local frame `f`, fade-in gain
is `clamp((f - fade_in_start) / fade_in, 0, 1)` and fade-out gain is
`clamp((fade_out_end - f) / fade_out, 0, 1)`. A zero length contributes gain one.
Both gains multiply for ordinary clips. A paired video dissolve keeps its
outgoing picture opaque while the incoming picture fades over it; audio uses
both clips' gain envelopes.

Split freezes both edge defaults before resizing. The left retains its anchors;
the right subtracts the cut's local frame from both anchors and adds it to the
inherited offsets. Repeated cuts repeat this operation. A fragment can retain a
fade longer than its own duration: it displays only the corresponding part of
the original ramp. Explicitly changing one fade clears only its anchor, placing
that new ramp at the edited fragment's own edge. New fade edits must fit the
fragment. Inspector focus/blur without a changed value preserves inherited ramps.

Linear keyframes stay in clip-local frames. Split/trim retain original knots in
the retained interval and synthesize endpoint values from the original curve.
The terminal control point is at `frame == duration`, the exclusive out point,
so interpolation remains continuous through the audio samples after the last
video frame. It is labeled as an out point in the inspector and can be removed
as an intentional curve edit. No keyframe is regenerated until another split or
trim. Trim extensions hold the synthesized nearest value; discarded automation
knots outside a fragment are not restored by extension.

Fade lengths are bounded to 10,000,000 frames and signed anchors to
±20,000,000. Render/composition offsets are bounded to ±10,000,000. Checked
rational arithmetic must derive a nonnegative inherited source origin. Invalid
metadata, source bounds, track locks, and persistence limits reject store edits
without changing the active model, undo/redo, save or recovery file.

## Stable source conversion and sample boundaries

For render offset `o`, the inherited source in point is
`source_in - (o / sequence_fps) * speed`. Video conversion uses that stable clock
over the valid original stream interval, converts to sequence fps, then selects
the signed PTS interval `[o, o + duration)` in integer sequence-frame ticks.
It subtracts `o` only after selection. This preserves cadence and converter
lookahead across cuts, including mixed input rates, negative trim extensions,
and leading/trailing intervals where the video stream is absent. Looped still
images have no finite media-duration limit.

Audio normalizes the common source origin and applies varispeed before selecting
the fragment. Define `B(f) = ceil(f * output_sample_rate / sequence_fps)` and
`r = start - o`. The inherited source sample origin is
`ceil(inherited_source_in / speed * output_sample_rate)`. Add `B(start)-B(r)`
and `B(end)-B(r)` to get the fragment's half-open source sample bounds. Placement
uses `B(start)` too. Adjacent fragments partition the same sample grid without
overlap, gaps, or a new resampler startup at each cut.

Volume and audio fades evaluate per sample. Their local clock includes
`B(start)/sample_rate - start/sequence_fps`, aligning a fractional frame boundary
with the actual first output sample. Base silence and final in/out ranges use
the same ceil sample boundaries. Explicit speed/source-in edits and slips
establish a new conversion clock; moves/duplicates retain the existing source
conversion offset. This does not add pitch-preserving time stretching.

Video placement and visibility use integer sequence-frame ticks. Position
automation uses actual timeline PTS rounded to frame units, avoiding decimal
boundary errors and the overlay filter's distinct frame counter. Crop, fit,
color, rotation, opacity and fades operate on a stable canvas; animated uniform
scale follows those pixel filters. Composition stays in RGB until the final
output conversion. Preview and export compile these same operations.
Both final graph branches have explicit frame/sample caps. A muxing time guard
is rounded beyond their exact EOFs; a CLI video-frame limit is avoided because
it can stop the output before the final audio encoding block is drained.

## Layer ordering and dissolves

Fragments sort by track, `start - composition.offset`, stable group ID, actual
start and clip ID. An outgoing fragment cannot jump above its incoming dissolve
partner merely because a cut gave it a later start. Dissolve matching compares
global anchored fade windows and strictly later composition roots, so siblings
do not accidentally become transition partners.

Moving all remaining siblings together retains their order. Independently moving
a subset detaches those fragments into their ordinary start-based layers.
Duplicating a subset creates a new group whose origin is its earliest selected
fragment. A new dissolve resets its edited fade anchors and incoming composition
origin. Extracted audio has independent composition identity.

Prepared source and proxy recipes use `source-v5` and `proxy-v3` names. Earlier
cache files are invalidated; `proxy_timing_version: 1` still identifies the same
common-zero stream timing convention. Unique program-preview paths need no cache
migration. Old packaged executables do not implement these inherited semantics;
use the updated source build to reopen these edits.

See [decoded-media validation](split-envelopes-validation.md) for measured results
and the remaining release and playback limitations.
