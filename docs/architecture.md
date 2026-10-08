# Architecture

Mono Cut 0.1 uses a Rust engine in a Tauri 2 desktop process and a React/TypeScript
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
| `lib.rs` | Tauri command boundary, managed state, events and resource paths |
| `src/` | Native command-driven interface, monitor playback and timeline input |

## Time and editing

Timeline coordinates are integer frames at the sequence's rational frame rate.
Media duration, source in-point and speed are rational numbers. Commands operate
on a validated project copy. A successful command writes the recovery snapshot
then enters history; errors leave the live project unchanged. Undo history keeps
100 states. Mixed input frame rates are converted explicitly to sequence fps in
the renderer and then to the requested export fps.

## Rendering and playback

The renderer produces one FFmpeg graph for video composition and audio mixing.
It trims sources, applies speed, fps conversion, crop, scale, rotation, color,
opacity/keyframes and fades, then composites video tracks over a neutral black
base. Audio is resampled, speed adjusted, faded, delayed in sample units and
mixed with a limiter. Export and program preview call this same compiler.
Cuts retain source conversion clocks and independent fade anchors. Pixel filters
use a stable canvas before animated scale; layer ordering survives fragmentation.
Audio gain evaluates per sample on the shared ceil placement/range grid. See
[cuts and inherited envelopes](split-envelopes.md) for exact semantics.
Preview may choose proxies and a smaller output resolution; final export uses
original media.

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
