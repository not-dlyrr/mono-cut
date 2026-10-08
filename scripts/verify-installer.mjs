// SPDX-License-Identifier: GPL-3.0-or-later
// Static verification: extract the installer without executing it or the editor.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const installer = resolve(process.argv[2] || join(root, 'src-tauri/target/release/bundle/nsis/Mono Cut_0.1.0_x64-setup.exe'));
const work = join(root, 'work'); mkdirSync(work, { recursive: true });
const extracted = mkdtempSync(join(work, 'installer-verify-'));
execFileSync('7z', ['x', installer, `-o${extracted}`, '-y'], { windowsHide: true, stdio: 'pipe' });
function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? files(join(directory, entry.name)) : [join(directory, entry.name)]);
}
const entries = files(extracted);
const executables = entries.filter(file => basename(file) === 'mono-cut.exe');
if (executables.length !== 1) throw new Error('Installer must contain exactly one Mono Cut executable.');
const installed = readFileSync(executables[0]);
const built = readFileSync(join(root, 'src-tauri/target/release/mono-cut.exe'));
const expected = Buffer.from(built);
const unknown = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
const nsis = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS');
const position = expected.indexOf(unknown);
let bundleMarkerMutation = false;
if (!installed.equals(expected) && position >= 0 && expected.indexOf(unknown, position + 1) < 0) {
  // Tauri stamps the NSIS package, then restores its build executable to UNK.
  nsis.copy(expected, position);
  bundleMarkerMutation = true;
}
if (!installed.equals(expected)) throw new Error('Packaged executable differs beyond the documented Tauri NSIS marker.');
const privatePathDetected = [homedir(), homedir().replaceAll('\\', '/')].some(prefix =>
  installed.includes(Buffer.from(prefix)) || installed.includes(Buffer.from(prefix, 'utf16le')));
if (privatePathDetected) throw new Error('Packaged executable embeds a private home path. Rebuild with path remapping.');
const installationRoot = dirname(executables[0]);
const resources = join(root, 'src-tauri/resources');
let resourceFilesValidated = 0;
for (const directory of ['media', 'notices']) {
  for (const file of readdirSync(join(resources, directory), { withFileTypes: true })) {
    if (!file.isFile()) continue;
    const relative = join('resources', directory, file.name);
    if (!readFileSync(join(installationRoot, relative)).equals(readFileSync(join(root, 'src-tauri', relative)))) {
      throw new Error(`Installer resource differs from its reviewed input: ${directory}/${file.name}`);
    }
    resourceFilesValidated += 1;
  }
}
if (entries.some(file => /WebView2Loader\.(?:dll|lib)$|MicrosoftEdgeWebView2.*\.exe$/i.test(basename(file)))) {
  throw new Error('Installer contains an unapproved proprietary loader/runtime distribution.');
}
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ installer: basename(installer), installerSha256: sha256(readFileSync(installer)),
  builtExecutableSha256: sha256(built), packagedExecutableSha256: sha256(installed),
  bundleMarkerMutation, resourceFilesValidated, privatePathDetected, editorExecuted: false }, null, 2));
