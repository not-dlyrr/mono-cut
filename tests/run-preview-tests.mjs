import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const work = join(root, 'work'); mkdirSync(work, { recursive: true });
const output = mkdtempSync(join(work, 'preview-tests-'));
try {
  const compilation = spawnSync(process.execPath, [join(root, 'node_modules', 'typescript', 'bin', 'tsc'), '--target', 'ES2022', '--module', 'commonjs', '--moduleResolution', 'node', '--jsx', 'react-jsx', '--skipLibCheck', '--strict', '--outDir', output, '--rootDir', join(root, 'src'), join(root, 'src', 'previewModel.ts'), join(root, 'src', 'previewScheduler.ts'), join(root, 'src', 'previewAssets.ts'), join(root, 'src', 'previewBridge.ts'), join(root, 'src', 'programPreview.ts'), join(root, 'src', 'previewRegion.ts'), join(root, 'src', 'monitorLifecycle.ts'), join(root, 'src', 'programMediaSlots.ts'), join(root, 'src', 'programTransport.ts'), join(root, 'src', 'editorShortcuts.ts'), join(root, 'src', 'GuidedTour.tsx'), join(root, 'src', 'retime.ts'), join(root, 'src', 'Inspector.tsx'), join(root, 'src', 'Timeline.tsx'), join(root, 'src', 'Monitor.tsx')], { cwd: root, stdio: 'inherit' });
  if (compilation.status !== 0) process.exitCode = compilation.status || 1;
  else {
    const tests = spawnSync(process.execPath, ['--test', join(root, 'tests', 'preview-scheduler.test.mjs'), join(root, 'tests', 'preview-regions.test.mjs'), join(root, 'tests', 'preview-continuity.test.mjs'), join(root, 'tests', 'program-media.test.mjs'), join(root, 'tests', 'app-ownership.test.mjs'), join(root, 'tests', 'retime-inspector.test.mjs'), join(root, 'tests', 'timeline-waveform.test.mjs'), join(root, 'tests', 'monitor-keyboard.test.mjs')], { cwd: root, stdio: 'inherit', env: { ...process.env, MONO_CUT_PREVIEW_TEST_BUILD: output } });
    process.exitCode = tests.status ?? 1;
  }
} finally {
  // Remove only the temporary directory created for this test run.
  if (!resolve(output).startsWith(resolve(work) + sep)) throw new Error('Unexpected test output path');
  rmSync(output, { recursive: true, force: true });
}
