import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const work = join(root, 'work'); mkdirSync(work, { recursive: true });
const output = mkdtempSync(join(work, 'preview-tests-'));
try {
  const compilation = spawnSync(process.execPath, [join(root, 'node_modules', 'typescript', 'bin', 'tsc'), '--target', 'ES2022', '--module', 'commonjs', '--moduleResolution', 'node', '--skipLibCheck', '--strict', '--outDir', output, '--rootDir', join(root, 'src'), join(root, 'src', 'previewModel.ts'), join(root, 'src', 'previewScheduler.ts'), join(root, 'src', 'previewAssets.ts'), join(root, 'src', 'previewBridge.ts')], { cwd: root, stdio: 'inherit' });
  if (compilation.status !== 0) process.exitCode = compilation.status || 1;
  else {
    const tests = spawnSync(process.execPath, ['--test', join(root, 'tests', 'preview-scheduler.test.mjs')], { cwd: root, stdio: 'inherit', env: { ...process.env, MONO_CUT_PREVIEW_TEST_BUILD: output } });
    process.exitCode = tests.status ?? 1;
  }
} finally {
  // Remove only the temporary directory created for this test run.
  if (!resolve(output).startsWith(resolve(work) + sep)) throw new Error('Unexpected test output path');
  rmSync(output, { recursive: true, force: true });
}
