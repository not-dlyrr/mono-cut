// SPDX-License-Identifier: GPL-3.0-or-later
// Produce an auditable license inventory from the exact locked dependencies.
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { resolve, sep } from 'node:path';
const root = resolve(import.meta.dirname, '..');
const lock = JSON.parse(readFileSync(resolve(root,'package-lock.json'),'utf8'));
const js = [];
for(const [key,value] of Object.entries(lock.packages)) {
  if (!key || value.dev) continue;
  const file = resolve(root,key,'package.json');
  if(!existsSync(file)) throw new Error(`Install dependencies before generating notices: ${key}`);
  const pkg = JSON.parse(readFileSync(file,'utf8'));
  js.push({name:pkg.name,version:pkg.version,license:pkg.license ?? value.license ?? 'UNKNOWN',repository:typeof pkg.repository==='string'?pkg.repository:pkg.repository?.url});
}
const host = execFileSync('rustc',['-vV'],{encoding:'utf8'}).match(/^host: (.+)$/m)[1];
const metadata = JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1','--filter-platform',host,'--manifest-path',resolve(root,'src-tauri/Cargo.toml')],{encoding:'utf8',maxBuffer:64*1024*1024}));
const selected = new Set(metadata.resolve.nodes.map(n=>n.id));
const rust = metadata.packages.filter(p=>selected.has(p.id)).map(p=>({name:p.name,version:p.version,license:p.license ?? 'UNKNOWN',repository:p.repository})).sort((a,b)=>a.name.localeCompare(b.name));
const unknown=[...js,...rust].filter(p=>p.license==='UNKNOWN');
if(unknown.length) throw new Error(`Unresolved licenses: ${unknown.map(p=>p.name).join(', ')}`);
mkdirSync(resolve(root,'licenses'),{recursive:true});
writeFileSync(resolve(root,'licenses/dependency-inventory.json'),JSON.stringify({javascript:js,rust},null,2)+'\n');
function licenseFiles(dir) {
  return readdirSync(dir,{withFileTypes:true}).filter(p=>p.isFile() && /^(LICENSE|LICENCE|COPYING|NOTICE|OFL)(\b|[-_.])/i.test(p.name)).map(p=>resolve(dir,p.name));
}
const supplementPath = resolve(root, 'licenses/upstream-notice-sources.json');
const supplementText = readFileSync(supplementPath, 'utf8');
const supplements = JSON.parse(supplementText).packages;
let supplemented = 0;
function packageNotices(pkg, directory) {
  const local = licenseFiles(directory);
  if (local.length) return local.map(file => readFileSync(file, 'utf8')).join('\n');
  const supplement = supplements.find(item => item.name === pkg.name && item.version === pkg.version);
  if (!supplement?.files.length) throw new Error(`Retain exact upstream notices before distribution: ${pkg.name} ${pkg.version}`);
  const vcsPath = resolve(directory, '.cargo_vcs_info.json');
  if (!existsSync(vcsPath) || JSON.parse(readFileSync(vcsPath, 'utf8')).git?.sha1 !== supplement.revision) {
    throw new Error(`Notice source revision does not match the locked package: ${pkg.name} ${pkg.version}`);
  }
  const prefix = resolve(root, 'licenses') + sep;
  const texts = supplement.files.map(input => {
    const file = resolve(root, 'licenses', input.file);
    if (!file.startsWith(prefix)) throw new Error(`Invalid retained notice path for ${pkg.name}`);
    const bytes = readFileSync(file);
    if (createHash('sha256').update(bytes).digest('hex') !== input.sha256) throw new Error(`Retained notice hash mismatch: ${input.file}`);
    return `Upstream notice: ${input.url}\nSHA-256: ${input.sha256}\n\n${bytes.toString('utf8')}`;
  });
  supplemented += 1;
  return `Preferred upstream source revision: ${supplement.revision}\n${texts.join('\n')}`;
}
let notices='Mono Cut dependency license notices\nGenerated from the locked distribution dependencies.\n\n';
for (const pkg of metadata.packages.filter(p=>selected.has(p.id) && p.name!=='mono-cut')) {
  const directory=resolve(pkg.manifest_path,'..');
  notices+=`\n===== ${pkg.name} ${pkg.version} | ${pkg.license} =====\n${pkg.repository ?? `https://crates.io/crates/${pkg.name}/${pkg.version}`}\n`;
  notices+=packageNotices(pkg,directory)+'\n';
}
for(const pkg of js) {
  const directory=resolve(root,'node_modules',pkg.name);
  notices+=`\n===== ${pkg.name} ${pkg.version} | ${pkg.license} =====\n${pkg.repository ?? ''}\n`;
  notices+=packageNotices(pkg,directory)+'\n';
}
writeFileSync(resolve(root,'licenses/DEPENDENCY_LICENSES.txt'),notices);
mkdirSync(resolve(root,'src-tauri/resources/notices'),{recursive:true});
writeFileSync(resolve(root,'src-tauri/resources/notices/DEPENDENCY_LICENSES.txt'),notices);
writeFileSync(resolve(root,'src-tauri/resources/notices/UPSTREAM_NOTICE_SOURCES.json'),supplementText);
const accepted=/^(?:MIT|Apache-2.0|BSD-2-Clause|BSD-3-Clause|0BSD|ISC|Zlib|Unicode-3.0|Unicode-DFS-2016|BSL-1.0|MPL-2.0|CC0-1.0|OFL-1.1|GPL-3.0-or-later|CC-BY-4.0|Unlicense|CDLA-Permissive-2.0|BlueOak-1.0.0|Apache-2.0 WITH LLVM-exception|BSD-2-Clause-Patent)(?:\s+(?:OR|AND)\s+.+)?$/;
const review=[...js,...rust].filter(p=>!accepted.test(p.license.replace(/[()]/g,'').replace(/\s*\/\s*/g,' OR ')));
console.log(JSON.stringify({javascript:js.length,rust:rust.length,supplemented_upstream_notices:supplemented,manual_review:review},null,2));
