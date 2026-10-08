# Program preview scheduling and reuse

Program preview still renders the complete sequence to a local H.264/AAC MP4.
This change reduces unnecessary renders and prevents obsolete results from
replacing the current program. It does not implement incremental decoding or
native frame/audio playback.

## Render identity

`preview.rs` projects the validated project into render inputs, preserving
additive fields by default. It excludes project/clip/media/track names, bins,
markers, export in/out ranges, track locks, linked selection IDs, thumbnails and
waveforms. Unreferenced media and empty tracks are excluded. Referenced tracks
retain their relative order. Clip and media records are sorted by ID, matching
the renderer's ID-based lookup and deterministic layer ordering.

The identity includes sequence geometry, rational frame rate and sample rate;
clip position, duration, source in, speed, transforms, color, opacity, volume,
fade anchors, keyframes, titles, composition origins and conversion offsets;
referenced track kind/mute/hide controls; preview resolution and proxy selection;
and the versioned preview recipe. Proxy metadata is omitted when proxies are
disabled. Source/proxy paths and file size, modification time and creation time
are included, together with the FFmpeg executable and title font when used.

The post-0.1.0 source uses recipe `program-preview-v2` for the even-pixel initial
fit correction. Its keys differ from v1, so previously rendered v1 previews
cannot be reused with the corrected renderer. The published Windows 0.1.0
installer retains the original v1 recipe.

File inspection is metadata-only. Ordinary identity checks do not launch media
children, decode footage or hash complete source files. It detects missing files
and ordinary replacements but cannot detect a deliberate replacement that
preserves every recorded filesystem stamp. This is a reuse key, not a
cryptographic authenticity guarantee.

## Interface and request ordering

The interface compares a pure model projection synchronously. A render-input
change pauses the old program and marks it stale immediately. Metadata changes
preserve its playback state. Every project replacement, including Save, still
requests a native identity check to detect external source changes.

Native identity replies also validate the completed output's manifest and file
stamp. An in-memory ready result is reusable only when its key and path match
that validated record. Missing or changed output files trigger repair. Actual
renders retain the 500 ms edit debounce; manual refresh bypasses it.

The interface begins a native preview session before loading its project. The
production `PreviewBridge` publishes each accepted identity's monotonic revision
through `set_preview_intent`, including the shortcut that restores an already
loaded preview after Undo. Publishing A immediately cancels differing-key B
workers; A does not need another render request. Native publication validates
the current project snapshot while holding the project store guard.
`render_preview` carries the accepted session/revision/key token and expected
key. Stale differing-key requests, prior sessions and snapshot mismatches return
`PREVIEW_IDENTITY_CHANGED`; the current interface can flush edits and retry.
Older revisions for the same current key may share work. Late responses cannot
cancel a shared worker solely because their interface request has become stale.

Identical requests join one running job or return the existing completed job.
Different-key requests supersede older queued work, cancel obsolete preview
workers and wait for cancellation before launching their replacement. Launch
admission, intent publication and empty-program clearing share the state lock
through the job cancellation/insertion boundary. A queued obsolete render cannot
launch after a newer intent has scanned existing jobs. Empty programs cancel
obsolete previews through playback-asset release; the store guard protects the
empty check from intervening edits. Completion
events are accepted only for the active job ID, render key and current project
session. Early events are buffered until the request reply; a late running
snapshot cannot regress terminal job state.

## Records and retention

Each completed render has a generation-specific `preview-KEY-UUID.mp4` and a
bounded `preview-KEY.json` record. The record contains its recipe/key, safe local
filename, output file stamp, dimensions, frame rate, sample rate, sequence frames
and duration. New output is probed before the record is committed, including its
actual video frame count and audio sample rate. Restarted
managers can reuse valid disk records without another encoder/probe launch.
Changed source stamps during encoding reject the result. A corrupt, incompatible,
missing or changed record/output causes regeneration to a new filename.

The preview subcache targets 32 records and 512 MiB, using record access times
for eviction. Unindexed replaced outputs are removed when unprotected and their
bytes count toward the budget while protected. The shared media cache also
targets 2 GiB. These are retention targets; temporary render space and explicitly
protected assets can exceed them.

The native process registry provides reference-counted asset pins. Worker
inputs, output destinations, temporary files, active source/program playback and
the most recent completion awaiting UI adoption are protected from cleanup.
Source preparation has a separate one-result handoff pin and native request
ordering; an older completion cannot replace the newest pending source pin.
The interface serializes fast source-intent invalidations before starting a slow
preparation. Selecting an image, clearing the source or resetting the project
invalidates earlier pending handoffs without waiting for their media work.
Adopting the matching source transfers protection to the playback pin. Playback
asset updates are serialized and coalesced by the interface. Releasing the final
owner makes an old cache file eligible for pruning again. A pending source
handoff may retain one extra file until adoption, another source preparation or
application shutdown.

## Verification

`src-tauri/tests/preview_reuse.rs` exercises real silent renders and filesystem
repair, including metadata/save/history reuse, source and render changes,
proxies, concurrent requests, cancellation and quota/pin behavior.
`src-tauri/tests/cache_retention.rs` checks aged-file protection and cleanup.
`npm run test:preview` runs the interface projection, scheduler and asset-queue
tests independently of React, including the production bridge with injected
native calls. These verify call ordering and delayed replies, rather than live
desktop IPC or playback. The ignored
`preview_benchmark.rs` measures larger synthetic workloads explicitly; it is
not part of routine CI. Results and limits belong in
[preview reuse validation](preview-reuse-validation.md).
