# Architecture

Mono Cut 0.1.2 uses a Rust engine in a Tauri 2 desktop process and a React/TypeScript
interface in the system webview. The development Vite URL serves that interface;
media commands still require the native desktop process. There is no browser
fallback that simulates editing or exports.

| Module | Responsibility |
| --- | --- |
| `model.rs` | Versioned project types, rational time, model validation |
| `edit.rs` | Deterministic validated editing commands |
| `storage.rs` | Transactions, bounded undo/redo, project IO and recovery |
| `media.rs` | FFprobe metadata, thumbnails, waveforms, source preparation, cache |
| `render.rs` | Shared filter-graph compiler for preview and final export |
| `jobs.rs` | Bounded background FFmpeg jobs, progress and cancellation |
| `preview.rs` | Render identities, completed-preview records and retention |
| `audio_clock.rs` | Sample-clock caches and bounded audio preparation |
| `video_seek.rs` | Initial packet-PTS assessment, bounded classification cache and source-specific seek fallback |
| `lib.rs` | Tauri command boundary, managed state, events and resource paths |
| `programPreview.ts` | Two-stage region requests, stale-result rejection and coverage |
| `programTransport.ts` | Signed transport intent and guarded resume across prepared regions |
| `monitorLifecycle.ts` | Current-media event, play-promise and playback-clock guards |
| `src/` | Native command-driven interface, monitor playback and timeline input |

## Time and editing

Timeline coordinates are integer frames at the sequence's rational frame rate.
Media duration, source in-point and speed are rational numbers. Commands operate
on a validated project copy. A successful command writes the recovery snapshot
then enters history; errors leave the live project unchanged. Undo history keeps
100 states. Mixed input frame rates are converted explicitly to sequence fps in
the renderer and then to the requested export fps.

`retime_clip` plans linked changes on a copy and preserves an exact
source span. The integer duration is its floor projection at the requested
speed; canonical source-relative envelopes and the source conversion origin
survive repeated requests, cuts and trims. Ordering anchors stay on the original
timeline clock. New/increased overlaps and invalid explicit ranges reject the
transaction. See [retiming](retiming.md) for rounding, native command compatibility
and verification scope.

## Rendering and playback

The renderer produces one FFmpeg graph for video composition and audio mixing.
It trims sources, applies speed, fps conversion, crop, scale, rotation, color,
opacity/keyframes and fades, then composites video tracks over a neutral black
base. Audio is resampled, speed adjusted, faded, delayed in sample units and
mixed with a limiter. Base audio and each post-gain branch explicitly use DBLP
before mixing, preventing inactive clips from changing accumulator precision
between full renders and bounded previews. Pre-gain FLTP and static float gain
remain fixed. Preview recipe v8 invalidates older program-preview cache entries.
It also resets post-trim sample PTS directly with `N` under the explicit sample
timebase, avoiding divided-double truncation that could change automation gain.
The retained v7 precision record is historical; the new inherited-fade sample
clock counterexample and correction are in [retiming](retiming.md).
Export and program preview call this same compiler.
Cuts retain source conversion clocks and independent fade anchors. Pixel filters
use a stable canvas before animated scale; layer ordering survives fragmentation.
Audio gain evaluates per sample on the shared ceil placement/range grid. See
[cuts and inherited envelopes](split-envelopes.md) for exact semantics.
Preview may choose proxies and a smaller output resolution; final export uses
original media.

Published 0.1.2 requests exact global regions through the same compiler. A
production controller prepares one frame, then five seconds of playback;
ordinary contributing video uses optimized seeks and filter EOF. Missing initial
packet PTS instead requires explicit origin-prefix decoding, whose cost grows
with source trim/playhead. A capped, cancellable assessment checks the first 256
selected packets of the physical original or proxy; it does not certify later
timestamps. Sample-clock preparation
preserves audio phase across packet timestamp rounding. See
[bounded preview architecture and validation](preview-regions.md) for exact
coordinates, cache limits, timing endpoints and exceptions. Independent review
accepts the tested Stage 4B prepared-region clock/readiness scope: lossless RGB
and PCM coverage are exact, and matching float-PCM AAC controls identify the
sharp-pulse residual downstream of correct timeline samples. Its strict
AAC-versus-FLAC RMS 0.003 diagnostic still fails; native playback, every source
clock and universal codec fidelity are outside that acceptance. See
[initial video timestamp correction](initial-video-timestamps.md) for the
evidence and limits. The following complete-sequence behavior describes the
preserved historical 0.1.1 installer; 0.1.2 uses the regions described above.

The 0.1 playback implementation renders a timeline preview MP4 on background
workers and plays/seeks that cached file through the native webview media element.
Render-input edits invalidate the preview and start a new render; metadata/save
changes reuse a validated completed file. This delivers a complete
editable rendering workflow, but is a pre-rendered preview architecture: it is
not a low-latency, direct frame decoding/compositing engine. Long or complex edits
can require a substantial refresh before program playback. J reverse shuttle
seeks backwards and does not produce reverse audio. Forward playback uses the
media element's synchronized audio.

The planned next engine step is incremental, bounded frame/audio caching and a
native playback clock with direct decoding and a shared render graph backend.
The project/command boundary allows that replacement without replacing the UI or
project format. Native GPU composition should preserve a CPU implementation and
same renderer semantics.

## Job and memory limits

The job manager admits at most two non-cancelled encoder jobs. New preview
renders cancel and drain older previews. Cancellation kills the owned FFmpeg
child, deletes its partial
output and retains source files. FFmpeg diagnostic tails are bounded to 64 KiB.
Source preparation, import/hydration and capability probing run on separate
background workers; they share the native process registry with encoder jobs.
Actual application exit cancels jobs, kills and reaps
owned FFmpeg/FFprobe children, and removes registered temporary outputs. A close
cancelled by the unsaved-project dialog does not shut down media processing.
Cache pruning targets 2 GiB after jobs; this is a retention bound, not a hard
temporary working-space guarantee during a render. The timeline renders only
clips inside the visible horizontal range. Project files have a 64 MiB input
size guard.

The program-preview subcache targets 32 records / 512 MiB. Native asset pins
protect active worker and playback files; protected assets may exceed retention
targets. Preview requests coalesce by render identity and reject stale interface
keys. See [preview scheduling and reuse](preview-cache.md) for fingerprint,
manifest validation, cancellation and adoption semantics.

Forward preview maintains two persistent media nodes: the current
region and one adjoining successor. A completed file is a candidate; actual
media readiness requires the matching loaded source, generation, duration and
`canplay` state. Current and successor consuming pins survive until their nodes
are detached. Loaded promotion preserves the candidate node and local time zero,
then native pin transfer must be acknowledged before a third preparation starts.
The signed transport continues across ready coverage; it waits at an uncovered
boundary and resumes only its own still-current request. Ordered cancellation
uses session/revision tombstones so held admissions cannot revive after Stop.
See [continuity](preview-continuity.md) for silent helper/file measurements and
the separate native display/audio observation limit.

## Portability and trust boundaries

Media processing is an external software FFmpeg/FFprobe boundary, with paths
passed as process arguments rather than shell commands. Project parsing and
commands validate ranges, references and editable properties. Font and media
tools are resolved from Tauri resource paths. Sources and cached output stay
local. There are no accounts or rendering services.

Windows is the tested shipping target. The Rust/React architecture is portable;
Linux/macOS CI compilation and unit tests help keep it so. Packaging and runtime
media validation on those platforms remain separate work until reported in the
validation record.
