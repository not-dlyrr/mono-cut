// SPDX-License-Identifier: GPL-3.0-or-later
// Keep local source paths out of Rust panic locations in distributed binaries.
import { homedir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const separator = '\u001f';
const existing = process.env.CARGO_ENCODED_RUSTFLAGS
  ? process.env.CARGO_ENCODED_RUSTFLAGS.split(separator)
  : (process.env.RUSTFLAGS || '').trim().split(/\s+/).filter(Boolean);
const mappings = [
  [homedir(), '/build/user'],
  [process.env.CARGO_HOME || resolve(homedir(), '.cargo'), '/build/cargo'],
  [root, '/build/mono-cut'],
];
const flags = [...existing];
for (const [path, replacement] of mappings) {
  for (const prefix of new Set([path, path.replaceAll('\\', '/')])) {
    flags.push(`--remap-path-prefix=${prefix}=${replacement}`);
  }
}
const result = spawnSync(process.execPath,
  [resolve(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', '--bundles', 'nsis', ...process.argv.slice(2)],
  { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags.join(separator) } });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
