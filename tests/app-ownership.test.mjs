// SPDX-License-Identifier: GPL-3.0-or-later
// Actual App callbacks and asset queue, controlled pin promises, and simulated DOM ownership.
// These checks do not launch a browser or verify native display, audio, or handoff latency.
import assert from 'node:assert/strict';
import { test, after } from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const require = createRequire(resolve(root, 'package.json')), ts = require('typescript');
const appPath = resolve(root, 'src/App.tsx'), queuePath = resolve(root, 'src/previewAssets.ts');
const original = { app: readFileSync(appPath, 'utf8'), queue: readFileSync(queuePath, 'utf8') };
const source = ts.createSourceFile(appPath, original.app, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const app = source.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'App');
assert.ok(app?.body, 'actual App function required');
const find = (node, predicate) => { if (predicate(node)) return node; return ts.forEachChild(node, child => find(child, predicate)); };
const functionText = name => { const node = app.body.statements.find(statement => ts.isFunctionDeclaration(statement) && statement.name?.text === name); assert.ok(node, name); return node.getText(source); };
const callbacks = find(app.body, node => ts.isNewExpression(node) && node.expression.getText(source) === 'ProgramPreviewController').arguments[3];
assert.ok(ts.isObjectLiteralExpression(callbacks));
const callbackText = name => { const property = callbacks.properties.find(item => item.name?.getText(source) === name); assert.ok(property && ts.isPropertyAssignment(property), name); return property.initializer.getText(source); };
const hookText = name => { const declaration = find(app.body, node => ts.isVariableDeclaration(node) && node.name.getText(source) === name); assert.ok(declaration && ts.isCallExpression(declaration.initializer), name); return declaration.initializer.arguments[0].getText(source); };
const extracted = [functionText('publishProgramAsset'), functionText('publishPlaybackAssets'), `globalThis.appCallbacks = { asset: ${callbackText('asset')}, successor: ${callbackText('successor')}, unavailable: ${callbackText('unavailable')}, onProgramAssets: ${hookText('onProgramAssets')}, setShuttle: ${hookText('setShuttle')}, publishPlaybackAssets };`].join('\n');
const transpile = text => ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS }, reportDiagnostics: true }).outputText;
const compiled = transpile(extracted), compiledQueue = transpile(original.queue);
const flush = async () => { for (let n = 0; n < 40; n++) await Promise.resolve(); };
const region = start => ({ start_frame: start, end_frame: start + 150 });
const asset = (id, start = 0) => ({ id, path: `${id}.mp4`, src: `asset://${id}.mp4?preview=${id}`, region: region(start), playable: true });
const job = (id, start = 0) => ({ id, path: `${id}.mp4`, preview_region: region(start) });
function harness() {
  const events = [], writes = [], state = { current: null, successor: null, speed: 0, dirty: false };
  const refs = { mediaOwnership: { current: { current: null, successor: null } }, candidate: { current: null }, pendingProgram: { current: null }, sourceAsset: { current: null }, previewAsset: { current: null }, currentPreview: { current: null }, previewMounted: { current: true }, closingRef: { current: false }, programVideo: { current: { pause() { events.push({ type: 'pause-node' }); } } } };
  const engine = { successor: null, acknowledgeAsset(id) { events.push({ type: 'ack', id }); }, setPlaybackSpeed(speed) { events.push({ type: 'engine-speed', speed }); }, stopPlayback() { events.push({ type: 'explicit-stop' }); } };
  const transport = { currentSpeed: 0, pending: null, request(value) { this.pending = null; this.currentSpeed = typeof value === 'function' ? value(this.currentSpeed) : value; state.speed = this.currentSpeed; }, resume(coverage, playable) { if (this.pending && playable && coverage.start_frame <= this.pending.frame && this.pending.frame < coverage.end_frame) { this.currentSpeed = this.pending.speed; state.speed = this.currentSpeed; this.pending = null; } events.push({ type: 'resume-check', speed: this.currentSpeed }); }, invalidate() { this.pending = null; this.currentSpeed = 0; state.speed = 0; events.push({ type: 'invalidated' }); } };
  const sandbox = { exports: {}, ...refs, native: true, programPreview: { current: engine }, programTransport: { current: transport }, convertFileSrc: path => `asset://${path}`, encodeURIComponent, Promise,
    setProgramAsset(value) { state.current = value; events.push({ type: 'publish-current', id: value?.id ?? null }); },
    setSuccessor(value) { state.successor = value; events.push({ type: 'publish-successor', id: value?.id ?? null }); },
    setPreviewRegion() {}, setPreviewPlayable() {}, setPreviewPath() {}, setPreviewDirty(value) { state.dirty = value; },
  };
  vm.createContext(sandbox); vm.runInContext(compiledQueue, sandbox, { filename: 'actual-previewAssets.js' });
  const queue = new sandbox.exports.PlaybackAssetQueue(snapshot => new Promise((yes, no) => { const item = { snapshot: structuredClone(snapshot), resolve() { events.push({ type: 'pin-complete', snapshot: item.snapshot }); yes(); }, reject() { events.push({ type: 'pin-failure', snapshot: item.snapshot }); no(new Error('pin failed')); } }; writes.push(item); events.push({ type: 'pin-begin', snapshot: item.snapshot }); }));
  sandbox.assetQueue = { current: queue }; vm.runInContext(compiled, sandbox, { filename: 'source-extracted-App.js' });
  const commit = (current, successor) => sandbox.appCallbacks.onProgramAssets({ current, successor });
  return { ...refs, callbacks: sandbox.appCallbacks, engine, transport, queue, events, writes, state, commit };
}

test('initial first still never mounts before its native consuming pin succeeds', async () => {
  const h = harness(); h.callbacks.asset(job('still'), { start_frame: 0, end_frame: 1 }, false); await flush();
  assert.equal(h.state.current, null); assert.equal(h.writes.length, 1); assert.equal(h.writes[0].snapshot.previewPath, 'still.mp4');
  h.writes[0].resolve(); await flush(); assert.equal(h.state.current.id, 'still'); assert.equal(h.state.dirty, true);
  assert.ok(h.events.findIndex(event => event.type === 'pin-complete') < h.events.findIndex(event => event.type === 'publish-current'));
});

test('new continuous asset pins old current plus replacement before mounting and acknowledges after actual DOM transfer', async () => {
  const h = harness(), still = asset('still'); h.mediaOwnership.current = { current: still, successor: null };
  h.callbacks.asset(job('continuous'), region(0), true); await flush();
  assert.equal(h.state.current, null); assert.equal(h.writes[0].snapshot.previewPath, 'still.mp4'); assert.equal(h.writes[0].snapshot.successorPreviewPath, 'continuous.mp4');
  h.writes[0].resolve(); await flush(); assert.equal(h.state.current.id, 'continuous'); assert.equal(h.events.some(event => event.type === 'ack' && event.id === 'continuous'), false);
  h.commit(h.state.current, null); await flush(); assert.equal(h.writes[1].snapshot.previewPath, 'continuous.mp4'); assert.equal(h.writes[1].snapshot.successorPreviewPath, null);
  assert.equal(h.events.some(event => event.type === 'ack' && event.id === 'continuous'), false); h.writes[1].resolve(); await flush();
  assert.equal(h.events.some(event => event.type === 'ack' && event.id === 'continuous'), true);
});

test('a superseded initial still pin reply cannot mount old footage after a newer continuous file becomes pending', async () => {
  const h = harness(); h.callbacks.asset(job('old-still'), { start_frame: 0, end_frame: 1 }, false); await flush();
  h.callbacks.asset(job('new-continuous'), region(0), true); h.writes[0].resolve(); await flush();
  assert.equal(h.state.current, null); assert.equal(h.events.some(event => event.type === 'publish-current' && event.id === 'old-still'), false);
  h.writes[1].resolve(); await flush(); assert.equal(h.state.current.id, 'new-continuous');
});

test('different successor replacement detaches old DOM ownership before admitting the new consumed path', async () => {
  const h = harness(), current = asset('current'), old = asset('old-next', 150); h.mediaOwnership.current = { current, successor: old }; h.state.successor = old;
  h.engine.successor = { job: job('new-next', 150) }; h.callbacks.successor(job('new-next', 150), region(150)); await flush();
  assert.equal(h.state.successor, null); assert.equal(h.writes[0].snapshot.successorPreviewPath, old.path);
  h.writes[0].resolve(); await flush(); assert.equal(h.state.successor, null, 'acknowledging old path must not admit a different new file');
  h.commit(current, null); await flush(); assert.equal(h.writes[1].snapshot.successorPreviewPath, 'new-next.mp4');
  assert.equal(h.state.successor, null); h.writes[1].resolve(); await flush(); assert.equal(h.state.successor.id, 'new-next');
});

test('ready successor promotion uses its existing consuming pin immediately and releases old current only after committed transfer', async () => {
  const h = harness(), current = asset('current'), next = asset('ready-next', 150); h.mediaOwnership.current = { current, successor: next }; h.state.current = current; h.state.successor = next; h.transport.currentSpeed = 1;
  h.callbacks.asset(job(next.id, 150), region(150), true);
  assert.equal(h.state.current.id, next.id); assert.equal(h.state.successor, null); assert.equal(h.writes.length, 0, 'loaded promotion does not wait on another pin RPC');
  h.callbacks.successor(null, null); await flush(); assert.equal(h.writes[0].snapshot.previewPath, current.path); assert.equal(h.writes[0].snapshot.successorPreviewPath, next.path);
  h.commit(h.state.current, null); h.writes[0].resolve(); await flush(); assert.equal(h.writes[1].snapshot.previewPath, next.path); assert.equal(h.writes[1].snapshot.successorPreviewPath, null);
  assert.equal(h.events.some(event => event.type === 'ack' && event.id === next.id), false); h.writes[1].resolve(); await flush(); assert.equal(h.events.some(event => event.type === 'ack' && event.id === next.id), true); assert.equal(h.transport.currentSpeed, 1);
});

test('pin failure cannot mount a successor and forced manual publication retries the unchanged pending path', async () => {
  const h = harness(), current = asset('current'); h.mediaOwnership.current = { current, successor: null }; h.engine.successor = { job: job('next', 150) };
  h.callbacks.successor(job('next', 150), region(150)); await flush(); h.writes[0].reject(); await flush();
  assert.equal(h.state.successor, null); const retry = h.callbacks.publishPlaybackAssets(true); await flush(); assert.equal(h.writes[1].snapshot.successorPreviewPath, 'next.mp4'); h.writes[1].resolve(); await retry; await flush(); assert.equal(h.state.successor.id, 'next');
});

test('edit invalidation clears a pending file and late successful pin admission cannot revive it', async () => {
  const h = harness(); h.callbacks.asset(job('obsolete'), region(0), true); await flush(); h.callbacks.unavailable('invalidated'); h.writes[0].resolve(); await flush();
  assert.equal(h.state.current, null); assert.equal(h.pendingProgram.current, null); assert.equal(h.events.some(event => event.type === 'publish-current' && event.id === 'obsolete'), false);
});

test('explicit Stop during pending initial publication permits correct stopped footage but never restores a previous resume intent', async () => {
  const h = harness(); h.transport.pending = { frame: 0, speed: 1 }; h.callbacks.asset(job('initial'), region(0), true); await flush(); h.callbacks.setShuttle(0); h.writes[0].resolve(); await flush();
  assert.equal(h.state.current.id, 'initial'); assert.equal(h.transport.currentSpeed, 0); assert.equal(h.events.some(event => event.type === 'explicit-stop'), true); assert.equal(h.events.some(event => event.type === 'engine-speed' && event.speed > 0), false);
});

after(() => {
  assert.equal(readFileSync(appPath, 'utf8'), original.app, 'App source changed during ownership checks');
  assert.equal(readFileSync(queuePath, 'utf8'), original.queue, 'Asset queue source changed during ownership checks');
});

