# Source timing

Mono Cut uses one source clock for the selected video and audio streams. Source
time zero is the earliest selected stream presentation start. A stream that
starts later keeps its delay; it is not independently moved to zero. Container
epochs, including a common nonzero start, are removed once.

The version-1 project format has additive optional `Media.timing` metadata:

- `origin`: the common source origin, in exact rational seconds.
- `video_start` / `audio_start`: absolute stream presentation starts.
- `video_end` / `audio_end`: absolute presentation ends when known.
- `video_stream` / `audio_stream`: selected absolute stream indexes.
- `container_duration`: the original exact `format.duration` decimal, retained
  to identify the duration interpretation used by earlier version-1 imports.

Probe prefers integer `start_pts * time_base`, then exact decimal `start_time`,
then the container start. Stream durations in ticks are elapsed durations.
Matroska `DURATION` tags and its container duration identify presentation ends;
they must not be treated as elapsed durations when the epoch is nonzero. Media
duration covers the selected streams from the common origin to their latest end.
Attached artwork is excluded from video stream selection.

For a source cue at time `s`, a clip source in point `i`, speed `r`, and timeline
start `t`, its sequence position is `t + (s - i) / r`. Video follows the existing
nearest-frame conversion at the sequence frame rate. Original video is first
normalized to the declared source frame-rate grid, matching the prepared/proxy
grid, before source trimming and speed conversion. This prevents coarse
container timestamps from selecting a different frame than an MP4 cache during
rate changes. A missing or unsupported declared source rate uses 30 fps.
Variable-rate video is therefore converted to this source grid; native variable
frame cadence is not retained in the caches. Fragment conversion now retains a
stable inherited origin before selecting sequence-frame PTS and output samples.
Audio establishes the common-zero clock before rate conversion, then selects
half-open sample intervals on the same absolute ceil grid used for placement
and export ranges. This preserves delayed silence and fractional-rate cuts.
See [cuts and inherited envelopes](split-envelopes.md) for the precise mapping.
Basic speed changes retain the existing sample-rate conversion and change pitch.

The renderer requests original timestamps with FFmpeg `-copyts`, subtracts the
common origin, and retains video presentation positions through trimming. It
does not ask the frame-rate filter to duplicate a delayed first frame backwards.
A missing video interval reveals the lower video track or the sequence's black
background. Audio resampling pads the initial gap from the common origin without
time stretching. Repair of internal timestamp discontinuities is not provided.

Source preparation and proxies use the same rendering clocks as program preview
and export. They contain black/silence for absent source intervals and begin at
zero. Neutral cache rendering omits the program audio mix limiter. Cache coverage
rounds up to a source frame so a final partial frame cannot discard an audio tail.
When compositing a proxy, the renderer restricts video to the original stream's
known presentation interval, preventing synthetic black padding from covering a
lower track. The original stream ends remain in the media record.

`source-v5` and `proxy-v3` filenames invalidate earlier normalization recipes,
including shared-clock caches made before fragment preservation.
Only proxies with `proxy_timing_version: 1` use the normalized zero origin.
Unknown legacy proxies are discarded during hydration and the original remains
available. Program preview paths are unique for each render. Thumbnail and
waveform caches are also versioned for the new source clock.

Older version-1 documents omit these fields and deserialize with unknown timing.
Opening or recovering an available source re-probes its timing without changing
media IDs, clip edits, bins, or the previously saved media duration. Engine callers
that bypass hydration resolve unknown timing when rendering. Missing media stays
in the project and can be relinked; relinking probes the replacement source and
clears its old proxy. Saves and recovery persist the new metadata through the
same bounded, transactional serialization used for every project edit.

Some older Matroska projects saved the absolute container end as media duration.
For example, video starting at 3 s and audio at 3.2 s with two-second streams
produced a saved duration of 5.2 s and a full 156-frame clip at 30 fps. Its actual
shared-clock span is 2.2 s. Migration preserves the 5.2 s logical extent and all
existing edit/effect coordinates; the final three seconds remain absent footage
and silent audio, revealing underlying tracks or black. It does not move or
trim that old clip.

An additive `Media.legacy_source` record stores the measured span in `duration`
and a content `sha256` when available. Eligibility is limited to pre-timing
documents and the earlier timing-only migration that omitted
`container_duration`: the saved duration must exceed the measured span and equal
the exact historical container duration at a positive source epoch. New imports
carry this provenance field and cannot infer a legacy tail. The exact original
decimal is used even when stream-end tags have finer precision.

When an original is available, migration streams its SHA-256 in 64 KiB chunks.
Later open/relink checks the retained timing, measured span, geometry, rate,
audio presence and content identity before replacing metadata. A byte-identical
copy can recover missing media. A shorter or different identified source is
rejected transactionally. Neither this record nor relinking changes ordinary
source-bound validation; new imports use their real measured duration. Cache
coverage uses the physical 2.2 s span rather than the old 5.2 s tail.

An already-missing old document has no original timing or content fingerprint.
Its first relink can infer this specific historical pattern from the saved
kind, geometry, rate and audio presence plus the replacement's exact container
duration. Historical byte identity cannot be proven in that case. Successful
adoption records the new timing/span and hash, making later replacement checks
strict. Recorded legacy sources require matching content even for otherwise
compatible replacements. Hashing adds file-read cost to legacy migration and
relinking; it runs on the existing media worker and uses bounded memory.

These are source implementation rules. The earlier Windows installer predates
the timing and persistence corrections; its behavior is not evidence for them.
