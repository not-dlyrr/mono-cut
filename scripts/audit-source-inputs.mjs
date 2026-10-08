// SPDX-License-Identifier: GPL-3.0-or-later
// Inspect every native/WASM archive in the corresponding Cargo source bundle.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
const destination = path.resolve(process.argv[2]);
const vendor = path.join(destination, 'cargo-vendor');
const inventory = [];
async function walk(directory) {
  for (const item of await fs.readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, item.name);
    if (item.isDirectory()) await walk(file);
    else if (/\.(?:dll|exe|lib|a|o|so|dylib)$/i.test(item.name)) {
      const relative = path.relative(vendor, file).replaceAll(path.sep, '/');
      const crate = relative.split('/')[0];
      const manifest = await fs.readFile(path.join(vendor, crate, 'Cargo.toml'), 'utf8');
      const name = manifest.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
      const license = manifest.match(/^license\s*=\s*"([^"]+)"/m)?.[1];
      const repository = manifest.match(/^repository\s*=\s*"([^"]+)"/m)?.[1];
      let category, source;
      if (/^windows_(?:aarch64|i686|x86_64)_/.test(name) || /^winapi-(?:i686|x86_64)-pc-windows-gnu$/.test(name)) {
        category = 'Open-source generated Windows import declarations and thunks; no implementation of the operating-system APIs.';
        source = 'Generated from the corresponding open Windows bindings project. Repository, revision and package source are retained; import libraries are MIT/Apache-2.0 licensed.';
      } else if (name === 'wit-bindgen' && relative.includes('/src/rt/')) {
        category = 'Open-source WASM ABI runtime objects.';
        source = 'Matching preferred C and Rust source is in this crate under src/rt; upstream regeneration script is ci/rebuild-libwit-bindgen-cabi.sh.';
      } else throw new Error(`Unreviewed prebuilt native source input: ${relative}`);
      if (!license || !/MIT|Apache-2\.0/.test(license)) throw new Error(`Source input license needs review: ${relative}`);
      let revision = null;
      try { revision = JSON.parse(await fs.readFile(path.join(vendor, crate, '.cargo_vcs_info.json'), 'utf8')).git?.sha1 ?? null; } catch { /* Old import-only packages predate Cargo VCS metadata. */ }
      const bytes = await fs.readFile(file);
      inventory.push({ file: `cargo-vendor/${relative}`, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), component: name, license, repository, revision, category, source });
    }
  }
}
await walk(vendor);
await fs.writeFile(path.join(destination, 'native-source-input-inventory.json'), JSON.stringify(inventory, null, 2) + '\n');
const groups = Object.groupBy(inventory, item => item.component);
console.log(JSON.stringify({ reviewedFiles: inventory.length, components: Object.fromEntries(Object.entries(groups).map(([key, items]) => [key, items.length])) }, null, 2));
