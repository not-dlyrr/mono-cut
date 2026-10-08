# Monitor keyboard ownership (unreleased source)

Source and Program now share an explicit keyboard target in Easy and Advanced.
The active monitor has a small **Keyboard** label and a line below its header,
using the existing focus token. Its region is reachable with Tab, has a visible
focus outline and identifies itself as the keyboard target to assistive tools.
Preview surfaces remain neutral and opaque. There is no transition delay.

## Choosing the target

- Clicking a monitor surface or focusing its region or controls activates it.
  A surface click focuses the region; clicking a button or scrubber preserves
  that control's focus. Direct transport and range buttons activate their own
  monitor even when invoked without a pointer event.
- Easy's Clip/Timeline tabs select Source/Program. The tab strip supports
  Left/Right, Home/End and a single tab stop for the selected tab. It consumes
  those navigation keys before the global shortcut handler.
- Timeline pointer or focus interaction chooses Program, including its tools,
  ruler, track controls and clips. Selecting a media item chooses Source,
  resets its range and frame, and retains the existing preparation-generation
  checks. Add to Timeline chooses Program.
- Focus elsewhere, including project menus and inspector fields, retains the
  last target. Those controls keep their own keyboard actions. Selecting a
  different editor mode chooses Program; Easy's hidden Source cannot remain
  the transport target after that switch.
- New/open/recovered projects clear Source and choose Program. Replacement,
  deletion, missing media or changed source metadata clears its prepared path,
  frame and range. Ordinary edits of the same project/source preserve them.
  Source identity covers its path, kind, frame rate, duration, dimensions,
  audio presence and measured source timing. Stale preparation replies cannot
  assign another asset. Unannounced replacement of bytes at an unchanged path
  with identical metadata still requires reselecting the source.

## Shortcut effects

Space toggles playback in the target monitor. J and L shuttle that monitor at
1x, 2x and 4x, with J using the existing silent reverse-seek clock. Frame arrows
pause and step that monitor using its own frame rate. I/O set Source's frame
range when Source is targeted, without issuing a sequence edit. Program I/O use
the global sequence frame, including when the prepared region begins later than
zero, and leave Source's range unchanged. The source range is converted using
source fps when inserted; audio without video fps uses the project's fallback
rate, as before. **K stops both transports**, independent of the target.

Source transport, steps and marks require a selected, prepared, nonmissing
moving/audio source. Preparing, failed, absent and still-image sources do not
fall through to Program commands. Images remain viewable and insertable, with
transport and range marks disabled. A prepared media node follows the existing
metadata/play-promise lifecycle; preparation does not claim it is already playing.

Source commands work while Program needs rendering. Program playback/shuttle
requires a playable, clean preview; a first-frame still cannot start playback.
Program stepping may request a new region and Program range marks remain
available while its preview is unavailable. Existing Program transport,
preview intent, native pin and successor ownership machinery is unchanged.

Editable fields, contenteditable regions, dialogs, menus, expanded menu triggers,
range/select and authored list/slider controls keep their keys. Focused buttons,
links and tabs retain Space/Enter activation and navigation arrows. Tab always
remains focus navigation. Remapping does not bypass those ownership rules.
Already-prevented events and composition events are ignored, including while
recording a shortcut. A recording consumes its own event without also running
an editor action. The document has one global handler with cleanup on replacement.

## Verification and limits

[Evidence](evidence/monitor-keyboard.json) records source and test hashes,
retained attempts and the unchanged native freeze. The frontend suite passes
142 tests: the previous 125 plus 17 ownership checks. The new checks extract and
transpile the actual App keyboard effect, activation, source selection/reset,
project replacement and frame-step callbacks, plus its actual Monitor and Easy
tab JSX bindings. They execute the compiled Monitor with controlled hooks and
event targets. They do not substitute a detached routing implementation.

Cases include both modes, both targets, all seven requested Source actions,
J/L escalation, K, differing Source/Program frames and fps, nonzero Program
region origins, dirty/first-still Program previews, original/remapped keys,
recording, dialogs and controls, tab navigation, direct buttons, one handler,
source replacement and obsolete preparation replies. The TypeScript/frontend
production build passes. All 32 frozen native files and prior evidence reports
remain unchanged. Engine media gates were not repeated because no engine,
render/audio recipe, preview controller or media ownership file changed.

The first test run retained seven failures in test instrumentation: five Monitor
checks lacked an injected frame callback, one asynchronous fixture inspected
preparation too early, and an older selector-substring mock needed its explicit
contenteditable selector retained. These captures and the corrected passing runs
are linked in the evidence; no media gate or tolerance was relaxed.

Testing stayed silent. No native app, browser, player, installer or audio device
was opened. Native focus transfer, pointer behavior, displayed styling and
audible transport behavior remain unverified. This is source only; the published
installer and release are unchanged.

Reproduce with `npm ci`, `npm run test:preview` and `npm run build`. No fixture
footage, media decoder or audio output is needed for this stage's tests.
