# Persistence reliability — Stage 1

The reported failure allowed a valid edit to grow a readable project past the
reader's 64 MiB limit, replacing its recovery and successfully saving a file
that could not reopen. This stage fixes that persistence contract without
removing bounded input handling.

## Change

`storage::MAX_PROJECT_BYTES` is the inclusive 67,108,864-byte limit for project
and recovery JSON. A bounded serializer validates the project and counts actual
pretty-printed UTF-8 JSON bytes, including escapes. It rejects oversized output
before creating a temporary file or replacing an existing file. The reader
checks metadata and limits the actual stream to the cap plus one sentinel byte.

Explicit save preflights both the relative-path document and the absolute-path
recovery before writing either. Committed edits, imports, new/open, hydration,
undo/redo and proxy attachment use the same writer contract. Proxy attachment
uses a candidate snapshot, and rejected attachments reach the interface's error
channel. Recovery availability and recovery share a canonical-size preflight,
so compact external JSON that cannot fit a future recovery is rejected before
changing state/history. Nonfinite waveform values are rejected rather than
being serialized as unreadable JSON `null` values.

Size rejection preserves the current project, undo/redo, previously saved
document and previous recovery. The error identifies the 64 MiB limit and
suggests shortening title text or removing unused clips/media.

## Regression evidence

Fixtures are generated in disposable directories and use the real project
store, writer and reader. The boundary suite serializes its own tests to bound
memory use. No giant fixture files are committed and no playback is needed.

| Case | Measured outcome |
| --- | --- |
| Below-limit project | 3,911 valid titles, 67,107,840 bytes; explicit save/reopen succeeds |
| Accepted autosave edit | 67,107,955 bytes; explicit save, reader, reopened store and recovery round-trip |
| Exact inclusive cap | 67,108,864 bytes; saved/reopened, recovery readable and available |
| One byte over | 67,108,865 bytes; writer and reader reject; existing save/recovery unchanged |
| Crossing title edit | 67,125,513 bytes rejected; current state, redo/undo and saved/recovery hashes preserved |
| Escaped text | Equal 16,384-byte raw titles yield 67,106,025 ASCII bytes (accepted) versus 67,122,409 newline-escaped bytes (rejected) |
| Save-as path expansion | 67,108,608-byte recovery fits; 67,109,324-byte relative-path document rejects before replacing its destination |
| Compact external JSON | 65,924,149-byte input requires 67,109,376 recovery bytes; open/recover reject transactionally and recovery is not offered |
| Proxy and nonfinite numbers | Oversized proxy attachment and NaN/positive/negative infinity waveform writes preserve readable state/files |

Commands from the repository root:

```powershell
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --test project_persistence -- --nocapture --test-threads=1
$env:MONO_CUT_FFMPEG = (Resolve-Path src-tauri/resources/media/ffmpeg.exe).Path
$env:MONO_CUT_FFPROBE = (Resolve-Path src-tauri/resources/media/ffprobe.exe).Path
$env:MONO_CUT_FONT = (Resolve-Path src-tauri/resources/media/Inter.ttf).Path
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --test engine_workflow -- --nocapture
cargo fmt --manifest-path src-tauri/Cargo.toml --check
npm run check
```

All seven new persistence tests passed, with zero failures, in 301.68 seconds
on Windows using the debug test build. The existing nine actual-media tests ran
concurrently and passed in 21.43 seconds against the bundled
FFmpeg/FFprobe/Inter. They retained trim/frame-rate/audio-sample accuracy,
preview/proxy/export geometry, undo/reopen/relink and cancellation/lifecycle
behavior. TypeScript and Rust formatting checks also passed. The normal CI
command runs all integration tests, including the new persistence suite.

This record concerns the source fix and engine checks. It does not claim a new
installer build, publication, or restarted native dialog/playback validation.
