# Contributing

Contributions are accepted under GPL-3.0-or-later. There is no contributor
license agreement. Keep the application and its distributable dependencies open
source; do not introduce paid gates, mandatory accounts, nonfree FFmpeg builds,
proprietary services or mandatory vendor SDKs.

1. Follow the setup instructions in the README and use the lockfiles.
2. Keep project data, editing commands, media processing and UI code separate.
3. Run `npm run check`, `npm run build`, and `npm run test:engine`.
4. For media changes, run the actual-media tests with FFmpeg and FFprobe and
   compare preview and export. A fake timeline or a mocked encoder does not
   validate an editing feature.
5. Describe the concrete behavior, validation and remaining limits in the PR.

Use integer sequence frames and rational source time. Declare conversions and
rounding explicitly. Validate project and command inputs before committing
changes. Keep process work outside the UI thread, cancellation prompt, caches
bounded, and errors actionable. Preserve source footage on cancellation and
errors. Never commit footage, credentials, signing keys, recovery files or private
machine paths.

UI work should follow the smoked charcoal/neutral monitor design, accessible
labels and focus states, one Appearance menu, resizable panels, direct timeline
input, and reduced-motion/transparency preferences. Do not add green status
components or decorative marketing UI. New unfinished capabilities belong in the
roadmap, not as inert controls in the editor.

Document project migrations, update the feature matrix and review third-party
licenses before adding dependencies. Add meaningful tests for changed editing
semantics and report actual playback/scrubbing measurements for performance work.

