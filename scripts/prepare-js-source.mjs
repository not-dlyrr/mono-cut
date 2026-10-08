// SPDX-License-Identifier: GPL-3.0-or-later
// Archive exact npm runtime inputs, checked against the committed lockfile.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';

const projectRoot = path.resolve(import.meta.dirname, '..');
const destination = path.resolve(process.argv[2] || path.join(projectRoot, 'work/app-source/javascript'));
const lock = JSON.parse(await fs.readFile(path.join(projectRoot, 'package-lock.json'), 'utf8'));
await fs.mkdir(destination, { recursive: true });
const inventory = [];
for (const [location, entry] of Object.entries(lock.packages)) {
  if (!location || entry.dev) continue;
  const name = location.slice(location.lastIndexOf('node_modules/') + 'node_modules/'.length);
  if (!entry?.resolved || !entry?.integrity) throw new Error(`Locked source missing for ${name}`);
  const [algorithm, expected] = entry.integrity.split('-');
  const response = await fetch(entry.resolved);
  if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (createHash(algorithm).update(bytes).digest('base64') !== expected) throw new Error(`${name}: integrity mismatch`);
  const file = `${name.replaceAll('/', '-').replaceAll('@', '')}-${entry.version}.tgz`;
  await fs.writeFile(path.join(destination, file), bytes);
  const metadataResponse = await fetch(`https://registry.npmjs.org/${encodeURIComponent(name)}/${entry.version}`);
  const metadata = await metadataResponse.json();
  inventory.push({ name, version: entry.version, license: metadata.license, repository: metadata.repository, gitHead: metadata.gitHead || null, url: entry.resolved, integrity: entry.integrity, sha256: createHash('sha256').update(bytes).digest('hex'), file });
}
await fs.writeFile(path.join(destination, 'source-inputs.json'), JSON.stringify(inventory, null, 2) + '\n');
const extra = JSON.parse(await fs.readFile(path.join(projectRoot, 'scripts/extra-source-inputs.json'), 'utf8'));
for (const source of extra.javascriptUpstreams) {
  for (const [name, version] of Object.entries(source.packages)) {
    if (!inventory.some(item => item.name === name && item.version === version)) throw new Error(`Update upstream source pin for ${name}`);
  }
  const file = path.join(destination, source.file);
  let bytes;
  try { bytes = await fs.readFile(file); } catch { /* Download below. */ }
  if (!bytes) {
    const response = await fetch(source.url);
    if (!response.ok) throw new Error(`${source.name}: HTTP ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
  }
  if (createHash('sha256').update(bytes).digest('hex') !== source.sha256) throw new Error(`Upstream source hash mismatch: ${source.name}`);
  await fs.writeFile(file, bytes);
}
await fs.writeFile(path.join(destination, 'upstream-source-inputs.json'), JSON.stringify(extra.javascriptUpstreams, null, 2) + '\n');
console.log(`Archived ${inventory.length} locked JavaScript runtime packages.`);
