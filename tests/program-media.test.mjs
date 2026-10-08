import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
const require = createRequire(import.meta.url), build = process.env.MONO_CUT_PREVIEW_TEST_BUILD;
const { ProgramMediaSlots, commitProgramMedia, detachProgramMedia } = require(join(build, 'programMediaSlots.js'));
const { PlaybackAssetQueue } = require(join(build, 'previewAssets.js'));
const { localPreviewTime, globalPreviewFrame } = require(join(build, 'previewRegion.js'));
const fps = { num: 30, den: 1 };
const asset = (id, start) => ({ id, path: `${id}.mp4`, src: `http://asset.localhost/${id}.mp4`, region: { start_frame: start, end_frame: start + 150 }, playable: true });
const media = (a, extra = {}) => ({ currentSrc: a.src, readyState: 3, currentTime: 0, duration: 5, paused: true, ended: false, error: null, ...extra });
function pair() { const model = new ProgramMediaSlots(), a = asset('A', 0), b = asset('B', 150); model.reconcile(a, b, 100); return { model, a, b, index: model.successor, generation: model.slots[model.successor].generation }; }
test('file completion alone cannot admit successor; current canplay, duration and generation are required', () => {
  for (const extra of [{ readyState: 0 }, { readyState: 2 }, { duration: 4.9 }, { duration: NaN }, { currentSrc: 'old.mp4' }, { currentTime: 1 }, { currentTime: NaN }, { currentTime: -1 }, { ended: true }, { paused: false }, { error: new Error('decode') }]) {
    const { model, b, index, generation } = pair(); assert.equal(model.admit(index, generation, media(b, extra), b.src, 101, 0, fps), false);
  }
  const { model, b, index, generation } = pair(); assert.equal(model.admit(index, generation, media(b), b.src, 99, 0, fps), false); assert.equal(model.admit(index, generation, media(b), b.src, 101, 0, fps), true); assert.equal(model.admit(index, generation, media(b), b.src, 102, 0, fps), false);
});
test('prepared promotion preserves the loaded node, source, generation and exact local origin', () => {
  const { model, a, b, index, generation } = pair(); model.admit(index, generation, media(b), b.src, 101, 0, fps); const previousOwner = model.ownership;
  model.reconcile(b, null, 200); assert.equal(model.current, index); assert.equal(model.slots[index].generation, generation); assert.equal(model.slots[index].ready, true); assert.equal(model.ownership, previousOwner + 1); assert.equal(model.slots[1 - index], null); assert.equal(model.snapshot().current.path, b.path);
  assert.equal(localPreviewTime(150, fps, b.region), 0); assert.equal(globalPreviewFrame(0, fps, b.region), 150); assert.equal(globalPreviewFrame(149 / 30, fps, b.region), 299); assert.notEqual(a.id, model.snapshot().current.id);
});
test('many adjacent promotions retain exactly two slots and release their predecessors', () => {
  const model = new ProgramMediaSlots(); let current = asset('0', 0); model.reconcile(current, null, 0);
  for (let n = 1; n < 100; n++) { const next = asset(String(n), n * 150); model.reconcile(current, next, n * 100); const index = model.successor, gen = model.slots[index].generation; assert.equal(model.slots.filter(Boolean).length, 2); model.admit(index, gen, media(next), next.src, n * 100 + 1, 0, fps); model.reconcile(next, null, n * 100 + 2); assert.equal(model.current, index); assert.equal(model.slots[index].generation, gen); assert.equal(model.slots.filter(Boolean).length, 1); current = next; }
});
test('Stop or replacement rejects retained readiness events even when the same cached URL is reloaded', () => {
  const { model, a, b, index, generation } = pair(); model.reconcile(a, null, 200); assert.equal(model.admit(index, generation, media(b), b.src, 201, 0, fps), false);
  model.reconcile(a, b, 300); assert.notEqual(model.slots[model.successor].generation, generation); assert.equal(model.admit(model.successor, model.slots[model.successor].generation, media(b), b.src, 299, 0, fps), false);
  const old = model.slots[model.successor]; model.reconcile(asset('new-edit', 200), null, 400); assert.equal(model.owns(index, old.generation), false);
});
test('delayed boundary releases current while preserving the separately loaded successor node', () => {
  const { model, b, index, generation } = pair(); model.reconcile(null, b, 200); assert.equal(model.current, null); assert.equal(model.successor, index); assert.equal(model.slots[index].generation, generation); assert.equal(model.slots.filter(Boolean).length, 1); assert.equal(model.admit(index, generation, media(b), b.src, 201, 0, fps), true); model.reconcile(b, null, 202); assert.equal(model.current, index);
});
test('a replacement current can coexist with an already loaded successor without overwriting it', () => {
  const { model, b, index, generation } = pair(); const replacement = asset('replacement', 0); model.reconcile(replacement, b, 200); assert.equal(model.successor, index); assert.equal(model.slots[index].generation, generation); assert.equal(model.snapshot().current.id, replacement.id);
});
test('pin acknowledgment excludes skipped or failed writes and transfers both ownership fields', async () => {
  const writes = []; let release; const held = new Promise(resolve => release = resolve);
  const queue = new PlaybackAssetQueue(async snapshot => { writes.push(snapshot); if (writes.length === 1) await held; });
  const initial = queue.update({ previewPath: 'A', sourcePath: null, successorPreviewPath: 'B' }); await Promise.resolve();
  const skipped = queue.update({ previewPath: 'B', sourcePath: null, successorPreviewPath: 'A' }); const transfer = queue.update({ previewPath: 'B', sourcePath: null, successorPreviewPath: null }); release();
  assert.equal(await initial, true); assert.equal(await skipped, false); assert.equal(await transfer, true); assert.equal(writes.length, 2); assert.equal(writes.at(-1).previewPath, 'B'); assert.equal(writes.at(-1).successorPreviewPath, null);
  const failed = new PlaybackAssetQueue(async () => { throw new Error('missing file'); }); assert.equal(await failed.update({ previewPath: 'A', sourcePath: null }), false);
});
test('committed ownership aborts old resources before pin transfer and never reloads a loaded promotion', () => {
  const { model, b, index } = pair(), events = [], generations = [null, null];
  const nodes = [0, 1].map(i => ({ muted: false, src: null, loads: 0, pause() { events.push(['pause', i]); }, load() { this.loads++; events.push(['load', i, this.src]); }, setAttribute(_, src) { this.src = src; }, removeAttribute() { this.src = null; } }));
  commitProgramMedia(model, nodes, generations); assert.equal(nodes[index].muted, true); assert.equal(nodes[model.current].muted, false);
  model.reconcile(b, null, 200); commitProgramMedia(model, nodes, generations); assert.equal(nodes[index].loads, 1); assert.equal(nodes[index].muted, false); assert.equal(nodes[1 - index].src, null); assert.equal(nodes[1 - index].muted, true); assert.equal(nodes[1 - index].loads, 2);
  model.reconcile(asset('edit', 220), null, 300); commitProgramMedia(model, nodes, generations); assert.equal(nodes[model.current].loads, 2);
  detachProgramMedia(nodes, generations); assert.deepEqual(generations, [null, null]); assert.equal(nodes.every(node => node.src === null && node.muted), true);
  commitProgramMedia(model, nodes, generations); assert.equal(nodes[model.current].src, model.snapshot().current.src); assert.equal(nodes[model.current].muted, false);
});
test('App and monitor use persistent successor media, loaded readiness, pin acknowledgment and coverage advance', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8'), monitor = readFileSync(new URL('../src/Monitor.tsx', import.meta.url), 'utf8'), component = readFileSync(new URL('../src/ProgramMedia.tsx', import.meta.url), 'utf8');
  assert.match(app, /advance\(next, advanceProgram\)/); assert.match(app, /acknowledgeAsset\(current.id\)/); assert.match(app, /successorPreviewPath:/); assert.match(app, /successorReady\(id\)/); assert.match(app, /trackFrame\(n\)/); assert.match(monitor, /<ProgramMedia/); assert.match(component, /key=\{index\}/); assert.doesNotMatch(component, /key=\{src\}/); assert.match(component, /onCanPlay=/); assert.match(component, /\.admit\(/); assert.match(component, /commitProgramMedia\(/); assert.match(component, /detachProgramMedia\(/);
});
