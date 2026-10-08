# Preview reuse validation

Verified on Windows on 2026-10-08. All fixtures were generated locally and all
encoding, probing and decoding stayed silent. No native UI was launched or
controlled for this stage. Subsequent installer packaging and release evidence
is recorded separately in [the 0.1.0 release record](release-0.1.0.md).

## Behavior

All 47 Rust checks in the final scheduling snapshot passed: four library checks, nine existing
engine workflow tests, two legacy relink tests, five source-clock tests, ten
cut/envelope tests, thirteen preview reuse/lifecycle tests and four retention tests.
The earlier seven project-size/serialization boundary tests were not repeated;
the persisted project model and storage implementation are unchanged in this
stage. Their previous results remain in [persistence validation](persistence-validation.md).

Completed program previews now survive display-only project changes and Save.
Native render identity and cache records determine reuse; real editing commands
are not classified by a command-name whitelist. Source replacements, missing
files, preview settings and rendering fields invalidate the result.

The original ten actual-media/cache lifecycle regressions cover rename, bins,
markers, ranges, labels, locks, thumbnails/waveforms, unused media, save/reopen,
undo/redo, effects, source replacement, resolution/proxy switching, persistent
record reuse and six corrupt/evicted record cases. Eight simultaneous identical
requests share one job with exactly two media children: encode and output probe.
Cache hits launch neither. Fifty small-project identities took 35.683 ms in one
test run; this is not a large-library/UI latency measurement.

A 24-key request burst and project switch delivered the latest blue source in
0.765 s with bounded children and clean sidecars. Source handoff tests verify
that invalidation rejects an old preparation, releases its pending pin and
preserves acknowledged playback ownership. Clearing an empty program cancels
its obsolete render even before a preview path has loaded.

Four separate cache-retention tests passed. A 35-render test retained 32 records
and 624,014 bytes. An aged active preview and its record survived cleanup; final
owner release allowed eviction. Orphan/replaced outputs are accounted for and
removed when unprotected. Pins are reference-counted and use normalized path
aliases; output pins exist before file creation/rename.

A deterministic real-FFmpeg cancellation test pauses after atomic output commit,
then cancels before the completion acknowledgement. Committed exports/proxies
remain complete and readable; only the unique cancelled preview generation is
removed. Cancellation before commit still removes the partial output.

The interface's 28 tests passed, as did TypeScript checking and the
production interface build. They cover projection exclusions/render changes,
key/job/session arbitration, completion-before-response ordering, terminal state,
metadata reuse, invalid cache repair, empty programs, asset update serialization
and ordered source invalidation. Native UI playback continuity is supported by
the implementation and these state tests, but was not measured live here.

## Ready preview intent follow-up

A follow-up review found that Undo could restore ready preview A while render B
continued: the ready shortcut did not call the native render endpoint. That
shortcut now publishes A's authoritative session/revision/key through the same
production bridge used before rendering. Publication cancels differing-key work,
and pending renders carry their accepted token through native launch admission.

Three additional actual-media tests passed in the final native build. Publishing
ready A alone stopped an active B child; the observed wait after publication and
asset acknowledgement was 16.3593 ms, using 5 ms polling. A's file bytes/path were
unchanged and no additional media child launched. Delayed B admission was
rejected without reviving work. Same-key older/current revisions and metadata
changes shared one uncancelled worker. A new native session stopped the prior
worker in 15.5888 ms and rejected old tokens, even for the same render key.

A library regression also holds launch admission closed while B is pending,
publishes A, then releases admission: B returns a stale-request error with no
job, child or asset pin. Clearing empty programs now keeps its state guard through
cancellation, and the Tauri command keeps the project guard through the empty
check. This closes an older clear racing with a newer launch.

Six added production-bridge tests exercise the actual orchestration helper used
by App with injected native calls: ready A/pending B/Undo A, held intent replies,
same-key sharing, early completion, stale identity and wrong-key retry. These are
not a measurement of native desktop IPC or user playback continuity.

The final follow-up source identities are:

| Component | SHA-256 |
| --- | --- |
| Job lifecycle source | `fd18f2681853a0fdfa2d8d9829687caacd1d7114dd91ed8640891e98d01de4a7` |
| Tauri command source | `3d04bec1dec0c6e7eff1f002fc183a92afc1adff2e9f5b05d9d84619c89236c1` |
| Production preview bridge | `978e47226c01c8a816a44b448d102be197741fbf12673f3a7b3d66f26ef5d564` |
| App integration | `a575175200e309072b10bcd30f897bbfb45dd50018c4ec15d78ebb3fcc3fb745` |

## Measured workload

The ignored `src-tauri/tests/preview_benchmark.rs` ran against one frozen source
snapshot before the intent follow-up above, with the bundled software FFmpeg
8.1.1. The renderer, preview identity/records, media tools and lockfiles are
unchanged by that follow-up; the twelve-minute full workload was not repeated
after the scheduling corrections. Its build hashes below identify the measured
earlier snapshot, not the final job/command/bridge source. It generates a silent 12-second,
1920×1080, 30 fps test source using x264 ultrafast/CRF 30/two threads. Each sequence
contains 20 clips on four video tracks, five successive clips per track. The
small sequence is 3 seconds / 90 frames; the large sequence is 60 seconds / 1800
frames. Both render at 960×540, 30 fps, H.264 CRF 22/AAC, with a half-second GOP.
There is no source audio; preview output contains its normal silent audio mix.

The earlier review's exact offsets are retained: x=0/960, y=0/540, scale=0.5.
The editor centers transforms, so these are not four fully visible quadrants;
portions extend beyond the canvas. This preserves the measured workload rather
than changing it between comparisons. Brightness +0.05 is an actual rendered
edit; independent first-frame decoding measured +7.3225 luma levels in the
visible sample region.

Timings below are one observed run, not statistical averages. Cold/edit/proxy
times run from request to terminal completion. Reuse times end when the native
API returns the already-completed job; later hash/RSS instrumentation is outside
that endpoint. These are engine-library test calls, not Tauri IPC/UI timings.

| Request | 3 s sequence | 60 s sequence | New managed media children |
| --- | ---: | ---: | ---: |
| Cold original render | 12.852939 s | 240.197879 s | 2 |
| Identical repeat | 5.954 ms | 4.040 ms | 0 |
| Metadata-only change | 5.480 ms | 4.139 ms | 0 |
| Visible brightness edit | 12.984400 s | 241.949128 s | 2 |
| Undo to completed original | 4.377 ms | 4.741 ms | 0 |
| Render from prepared proxy | — | 248.901527 s | 2 |

Proxy generation took another 4.924583 s and produced 2,160,019 bytes; it is
excluded from the prepared-proxy render time. Proxies reduced observed FFmpeg
working set from 2,929,201,152 bytes (2.73 GiB original peak) to 1,911,304,192
bytes (1.78 GiB proxy peak), but did not improve render latency in this workload.
The Rust benchmark process peaked around 16 MiB; this excludes the desktop shell
and webview and is not full application memory use. Native Windows working sets
were sampled at roughly 100 ms intervals.

The earlier source review measured the same small cold workload at 13.002 s,
identical repeat at 12.880 s and rename at 13.288 s, with a new encoder/output
each time. The new cold output is byte-identical to that review output. The
earlier large run was cancelled at 75 s / 28.72% without finishing, so there is
no measured earlier large completion time and no cold-render speedup claim.
Two audit-interrupted attempts are retained as diagnostics only; the table uses
the completed final frozen run.

A final three-edit cancellation burst stopped in 81.176 ms after the last
cancel request, with zero remaining children and partials. Another eight-way
coalescing/24-key switch stress case is covered by the regression suite above.
The final cache occupied 14,491,660 bytes. Shutdown left zero managed children,
zero media-process working set and zero asset pins.

## Output and build identity

Independent post-run FFprobe checks counted all five unique outputs at exactly
960×540 / 30 fps, with 90 or 1800 frames as appropriate. Source and output hashes
remained unchanged. Repeats, metadata edits and undo retained the same file and
hash and created no graph, partial, probe or encoder.

| Output | Bytes | SHA-256 |
| --- | ---: | --- |
| Small original | 206,529 | `74be792034ccfcafe6521ccfe64031f37bbb0fa4e96e544d0ccfb85ecbbb0b5f` |
| Large original | 4,004,843 | `21fbdb0a7305b55bac980c71dd4a6445193fe97ebc6fd70108986ac6805da07e` |
| Large brightness edit | 4,011,275 | `2f408202e3545400f8c0524f99e827bae2c673c130c27500de55ae073774940b` |

The debug Rust integration-test executable was freshly linked before measuring.
Cargo and all recorded engine-source hashes matched before/after the full run.
The FFmpeg executable is the bundled source-built media component; no renderer
or proprietary service was substituted.

| Component | SHA-256 |
| --- | --- |
| Benchmark executable | `a6e3714fd81aeb0c6573660d8bd226ae7be2d80941f6da859cfb30d415e9ee14` |
| FFmpeg | `f3762e006f5ccbccef744637e36bd922767fa6d16182b36c4574c0272d23e0ab` |
| FFprobe | `79e40c9de4efd866cd3abe8e24824c0b5c3116c2eb6f397ad91c429896cd048f` |
| Shared renderer source | `d618df2d1631297aa9a48a838d449c4c5c46140f14d1eb10bf115603a7556d2c` |
| Preview identity/record source | `32d481e30bfcecb72c7f3cb1c4c779909763e350679d214a509803e74ec9ca2c` |
| Job/source lifecycle source | `524a3101f216bd7a3525a89619536026e46faff1e2cab7f7a2ef4597e5193600` |

Raw results, source hashes, logs and independent output verification are retained
in the local ignored `work/stage-4a-preview-benchmark/final-run/` directory.
They are development evidence, not public release artifacts. To reproduce,
provide `MONO_CUT_FFMPEG`, `MONO_CUT_FFPROBE`, `MONO_CUT_FONT` and a disposable
`MONO_CUT_BENCH_DIR`, set `MONO_CUT_BENCH_MODE=all` and optionally
`MONO_CUT_BENCH_PROXY=1`, then run:

```text
cargo test --locked --manifest-path src-tauri/Cargo.toml --test preview_benchmark -- --ignored --nocapture
```

## Remaining work

Actual timeline edits still render the complete sequence. The measured four
minutes to refresh this full-minute workload is the reason incremental native
frame/audio playback remains the next engine step. Native playback/scrubbing,
speaker output, long-form/4K performance, other operating systems and renewed
installer behavior were not measured in this stage.

Metadata/source stamps detect ordinary file changes; they cannot detect deliberate
replacement preserving all stamps. Protected live files and temporary outputs
can exceed cache retention targets. See [preview scheduling](preview-cache.md).

The earlier installer predates these and the timing/envelope corrections. The
subsequent corrected installer/source delivery belongs to the separate release
record above. This scheduling stage does not claim renewed native playback
validation or professional feature parity.
