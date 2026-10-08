# Prepared-region continuity: Stage 4C

This record freezes the v7 source and evidence. The later [retiming stage](retiming.md)
adds v8's integer post-trim audio sample clock after a separate inherited-fade
counterexample; it preserves the prepared-region ownership model described here.
The original Stage 4C JSON, binaries and failed/accepted captures remain unchanged.

This ownership model and the later v8 clock correction ship in [0.1.2](release-0.1.2.md).
The older 0.1.1 assets remain intact. It implements look-ahead for ordinary forward
playback; native monitor display, media-element boundary latency and audible
handoff have not been observed for these stages or the new package. Validation stays silent and launches no editor,
browser, player or installer.

## Implemented flow

The production `ProgramPreviewController` first prepares a requested still and
five-second current region. While a positive transport intent is active, an
acknowledged current playback pin permits one successor request. Successor
coverage starts exactly at the current half-open end and only clamps its end to
the sequence duration. It does not reuse the tail-overlapping initial-seek rule.

`ProgramMedia` keeps two stable video elements. A completed successor file gets a
native consuming pin before loading into the hidden, muted element. Its source,
generation, finite duration, local time zero and `readyState >= 3` must match
before `canplay` admits it. A queued event or a finished encoder alone cannot
make a successor playable. The hidden element has no focus or pointer actions.
No decorative transition is applied to footage or playhead movement.

When current media ends, the actual App coverage-advance path adopts a ready
successor without clearing coverage or changing the signed transport speed.
The already loaded and pinned node becomes current in the same React commit;
its source and generation do not change and it is not reloaded. The predecessor
is paused, muted and detached before pin transfer. The next preparation waits
for the new current pin acknowledgment. Only the current element can play audio.

New initial files also wait for a confirmed native pin before becoming visible,
so an early continuous completion cannot leave a still consuming an unprotected
file. Candidate replacement detaches its prior node, then pins the actual new
path before publishing its props. Failed or superseded pin writes never count
as acknowledgments. Manual Render preview can retry a failed pin update.

At a boundary without a loaded successor, transport pauses and retains only the
matching signed resume request. Generic successor preparation/load failures
preserve valid current footage until its covered end. Stop, seek, edits, preview
settings, source identity or project changes reject obsolete candidates and
resume requests. Save and non-render metadata preserve useful work. Native
session/revision cancellation rejects held requests and pre-Stop same-key tokens.

Reverse remains silent stepping with existing look-behind preparation. This
change does not add direct GPU composition, pitch-preserving retiming or unlimited
look-ahead. The program subcache still targets 32 records / 512 MiB; active pins
may exceed retention targets. One useful preparation chain and one replaceable
pending identity check bound active work.

## Validation endpoints

The silent harness compiles the actual controller, bridge, scheduler, transport
and pin queue, and uses the current native ProjectStore and JobManager. It starts
with an empty application cache for each original/proxy cohort, uses a real
monotonic 20-second clock and inspects and silently decodes each newly produced
file before explicit **file-ready** admission. This is a measured file endpoint;
it does not substitute for observed media-element `canplay`, native display or
speaker continuity. The production monitor requires those real load events.

The ordinary fixture contains mixed source frame rates, source offsets, speed,
fades, dissolves, a title and audio automation. It has no inherited split fields;
the separate `split_envelopes` engine tests cover those fields. Full-sequence
references share the renderer and remain independent of final region trimming.
Content limits stay gray RMS <= 5 over all frames, RGB/source-marker RMS <= 6 at
first/middle/last frames, and ordinary native AAC-versus-full FLAC RMS <= 0.003.
Unquantized float PCM is compared with the full graph bit for bit, without an
epsilon or signal normalization. Encoder quality, GOP, tolerances and source
amplitudes remain fixed.

The first v6 reference runs exposed graph-dependent mixer precision at a middle
seek. Full audio mixed in DBLP, while the bounded graph rounded automation to
FLTP before mixing. The four adjoining regions remained exact, but the standalone
seek failed strict float equality. Retained controls reproduce both outputs;
forcing common post-gain DBLP makes the bounded samples match the original full
samples exactly. No gain, sample clock or source level changes are needed.

The shared renderer now explicitly fixes base audio and all post-gain branches
to DBLP before mixing. Pre-gain FLTP and static float volume multiplication stay
unchanged. Preview recipe v7 invalidates earlier prepared previews; precise source
audio, proxy and timing recipes remain unchanged. The same model applies to
preview and export. V6 failures and controls are retained rather than waived.
Projects whose previous full mixer selected FLTP can have different rounding;
this does not promise every older export is byte-identical. Cross-platform
runtime and bit-identical output have not been measured in this stage.

The accepted Stage 4B missing-initial-PTS correction remains in use. Its explicit
origin-prefix and oversized-audio fallbacks may miss interactive timing targets;
assessment of initial packets does not certify later timestamp anomalies. The
sharp-pulse AAC-versus-FLAC quality failures remain documented in
[initial video timestamps](initial-video-timestamps.md); exact float samples and
matching encoder controls classify them without relaxing the strict diagnostic.

## Current v7 results

Fresh originals and prepared-proxy runs each used 600 frames / 20 seconds at
1x. At boundaries 150, 300 and 450 the file-ready successors were already
validated, the signed speed remained 1, and coverage was not cleared. Original
successors led the boundary by 2.747–2.928 seconds; proxies led by
2.985–3.183 seconds. Measured clock runs lasted 20.017 and 20.028 seconds.
These are actual file/helper results; native handoff remains unobserved.

| Endpoint | Originals | Prepared proxies |
| --- | ---: | ---: |
| Real edit during running successor: decoded frame | 738 ms | 617 ms |
| Same edit: five-second file ready | 3,072 ms | 2,621 ms |
| Middle seek during running successor: decoded frame | 732 ms | 618 ms |
| Same seek: five-second file ready | 2,742 ms | 2,263 ms |
| Ten actual edits: decoded-frame p95 | 887 ms | 702 ms |
| Ten actual edits: five-second-ready p95 | 3,244 ms | 2,853 ms |

The edit cohorts each include a separate cold sample and ten actual edits;
nearest-rank p95 uses the ten edits. Timings include the native edit/identity
requests, actual 100 ms debounce, background work and silent file endpoints.
They do not include a displayed monitor frame. A ready-cache shortcut is not
counted as new file preparation.

One useful native preparation and at most two owned preview files were observed.
Peak application cache usage was 19.29 MB for originals and 18.35 MB for proxies.
Protected source/worker assets are reported separately; those pins do not mean
additional preview elements. The seek and edit interventions both caught an
actually running successor and verified its cancelled terminal state. All
children, file readers and recursively inspected partial outputs were gone
after cleanup. Source, tool, input and compiled-helper hashes stayed fixed.

The interface build passes and 105 interface/ownership checks pass, including
the original 62. Eight checks extract the actual App callbacks and pin queue;
their DOM ownership notifications are simulated. They verify delayed pin
publication and loaded-promotion ownership, without claiming native rendering.

V6 originals and proxy content runs passed their ordinary picture/AAC checks,
lossless RGB frames and exact adjoining 600-frame / 960,000-stereo-sample coverage.
Their standalone seek failed strict float equality (3,036 original and 3,924 proxy
interleaved values). Both aggregate tests failed. The unchanged regression now
passes with zero differences after the common mixing-precision correction. Fresh
v7 originals and proxy references each pass all eight cases, including both still
endpoints and six playable regions. Every lossless RGB frame and unquantized
stereo sample matches its full reference exactly. All original native picture/AAC
limits and matching AAC controls pass. Each adjoining 20-second span covers
exactly 600 frames and 960,000 stereo sample frames; joined AAC-versus-FLAC RMS
is 0.00008866 for originals and 0.00009711 for proxies. No strict failure was waived.

Of the repeated edit assets, 43/44 MP4s and 22/22 decoded stills match the prior
evidence. The changed originals edit-8 region at frames 1300–1450 has a fresh
full-reference check against its exact saved cumulative edit state. All 150
lossless RGB frames and 240,000 stereo float sample frames match exactly. Native
gray RMS is 1.800, sampled RGB/marker values remain within 6, and AAC-versus-FLAC
RMS is 0.00008123. Its matching AAC control is exact. The static v6 byte-equivalence
failure remains recorded separately; it is not rewritten as an unchanged file.
All 73 engine/persistence tests pass with zero failures or ignored tests. They
cover source timing, rational split envelopes, bounded regions and mixing
precision, cancellation, cache retention/reuse, missing-media relink, project
save/reopen/autosave/recovery and file-size limits. Source/tool hashes stay fixed.
The [machine-readable record](evidence/preview-continuity.json) links the frozen
builds, timing cohorts, full references, retained failures and corrected results.

## Reproduction

Build the driver with the platform's Rust/FFmpeg prerequisites. Keep generated
media, projects, cache files and raw logs outside the public source checkout.

```powershell
npm run test:preview
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --example preview_region_driver -j 1
node scripts/measure-preview-continuity.mjs --project <ordinary-project> --output <fresh-private-output> --mode originals
```

Run a second fresh cohort with its prepared-proxy project and `--mode proxies`.
Use the ordinary 30 fps / 48 kHz fixture described in [region validation](preview-regions.md).
Set `MONO_CUT_CONTINUITY_REPORT` to a cohort's `continuity-results.json`, then run:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test continuity_reference -- --ignored --test-threads=1 --nocapture
```

The reference collector writes `content-reference-v3` alongside the timing
report. Historical failure manifests are optional for a fresh reproduction;
present but invalid manifests fail. The collector writes every strict float
failure before failing its aggregate assertion, so a failed run remains inspectable.

For a changed retained edit asset, the optional Python/NumPy helper uses the
exact saved snapshot and native driver; raw outputs must stay outside the checkout:

```powershell
py -3 scripts/verify-retained-edit-asset.py --report <controller-results.json> --label warm-edit-8 --snapshot <saved-state.monocut> --output <fresh-private-reference> --driver <native-driver>
```

The verified extra capture used Python 3.14.5 and NumPy 2.4.6. The portable helper
was checked for syntax, argument handling, output-path protection and exact shared
comparison-function source linkage; that port was not separately rerun on media.
The private capture and public helper hashes are both retained in the record.

Run performance cohorts exclusively; reference encodes and correctness tests run
after timing. Preserve failed/superseded cohorts. Never infer native gapless
playback or platform release readiness from these helpers and decoded files.
