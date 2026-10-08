// SPDX-License-Identifier: GPL-3.0-or-later
// Actual Inspector code with controlled hooks/events; no browser, display or audio.
import assert from 'node:assert/strict';
import { test, after } from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(join(root, 'package.json')), build = process.env.MONO_CUT_PREVIEW_TEST_BUILD;
const ts = require('typescript'), inspectorPath = join(root, 'src/Inspector.tsx'), original = readFileSync(inspectorPath, 'utf8');
const { parseSpeedDraft, retimeCommand } = require(join(build, 'retime.js'));
const { previewModelDescriptor } = require(join(build, 'previewModel.js'));
const { PreviewScheduler } = require(join(build, 'previewScheduler.js'));
const { PreviewBridge } = require(join(build, 'previewBridge.js'));
const { ProgramPreviewController } = require(join(build, 'programPreview.js'));
const { ProgramTransport } = require(join(build, 'programTransport.js'));
const flush = async () => { for (let n = 0; n < 40; n++) await Promise.resolve(); };
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
const r = (num, den = 1) => ({ num, den });

let activeHooks;
const react = {
  useState: value => activeHooks.useState(value), useRef: value => activeHooks.useRef(value),
  useEffect: (effect, deps) => activeHooks.useEffect(effect, deps), useId: () => activeHooks.useId(),
};
const element = (type, props, key) => ({ type, props: props ?? {}, key });
const sandbox = { exports: {}, require: name => {
  if (name === 'react') return react;
  if (name === 'react/jsx-runtime') return { jsx: element, jsxs: element, Fragment: 'Fragment' };
  if (name === 'lucide-react') return new Proxy({}, { get: (_target, key) => String(key) });
  if (name === './ui') return { Field: 'Field', NumberField: 'NumberField', IconButton: 'IconButton' };
  if (name === './types') return require(join(build, 'types.js'));
  if (name === './retime') return require(join(build, 'retime.js'));
  throw new Error(`Unexpected actual Inspector dependency ${name}`);
} };
vm.createContext(sandbox);
vm.runInContext(ts.transpileModule(original, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText, sandbox, { filename: 'actual-Inspector.js' });
const Inspector = sandbox.exports.default;

function component(render, initialProps) {
  const slots = [], effects = []; let cursor = 0, dirty = true, props = initialProps, tree;
  const slot = create => { const index = cursor++; return slots[index] ?? (slots[index] = create(index)); };
  const hooks = {
    useState(initial) { const item = slot(() => ({ value: typeof initial === 'function' ? initial() : initial })); return [item.value, value => { const next = typeof value === 'function' ? value(item.value) : value; if (!Object.is(next, item.value)) { item.value = next; dirty = true; } }]; },
    useRef(initial) { return slot(() => ({ current: initial })); },
    useId() { return slot(index => ({ value: `retime-test-${index}` })).value; },
    useEffect(effect, deps) { const item = slot(() => ({ deps: undefined })); if (!item.deps || deps.some((value, index) => !Object.is(value, item.deps[index]))) { item.deps = [...deps]; effects.push(effect); } },
  };
  function update(next = props) {
    props = next; dirty = true;
    for (let pass = 0; dirty; pass++) {
      assert.ok(pass < 20, 'Actual component state must settle'); dirty = false; cursor = 0; activeHooks = hooks;
      try { tree = render(props); } finally { activeHooks = undefined; }
      for (const effect of effects.splice(0)) effect();
    }
    return tree;
  }
  update(); return { update, get tree() { return tree; } };
}
function find(node, predicate) {
  if (!node) return undefined;
  if (Array.isArray(node)) { for (const child of node) { const match = find(child, predicate); if (match) return match; } return undefined; }
  if (typeof node !== 'object') return undefined;
  return predicate(node) ? node : find(node.props?.children, predicate);
}
function project(kind = 'video', overrides = {}) {
  return { version: 1, id: 'project', name: 'Speed fixture', width: 160, height: 90, fps: r(30), sample_rate: 48000,
    tracks: [{ id: 'video', name: 'Video', kind: 'video', muted: false, hidden: false, locked: false }, { id: 'audio', name: 'Audio', kind: 'audio', muted: false, hidden: false, locked: false }],
    media: [{ id: 'media', name: 'Fixture', path: 'fixture.mkv', kind, duration: r(16), fps: r(30), width: 160, height: 90, has_audio: true, bin_id: null, thumbnail: null, waveform: [], proxy: null, missing: false }],
    clips: [{ id: 'clip', media_id: 'media', track_id: kind === 'audio' ? 'audio' : 'video', name: 'Fixture', start: 0, duration: 480, source_in: r(0), speed: r(1), linked_id: null, title: null, transform: { x: 0, y: 0, scale: 1, rotation: 0, crop_left: 0, crop_right: 0, crop_top: 0, crop_bottom: 0 }, opacity: 1, volume: 1, fade_in: 0, fade_out: 0, brightness: 0, contrast: 1, saturation: 1, keyframes: [], ...overrides }],
    bins: [], markers: [], in_point: null, out_point: null };
}
function inspector(p, easy, edit = async () => {}) {
  const view = component(Inspector, { project: p, selected: ['clip'], frame: 0, edit, easy });
  const speed = find(view.tree, node => typeof node.type === 'function' && node.type.name === 'SpeedField');
  assert.ok(speed, 'Actual Inspector SpeedField wiring required');
  return { view, speed, field: component(speed.type, speed.props) };
}
const input = field => find(field.tree, node => node.type === 'input');
function type(field, value) { input(field).props.onFocus(); field.update(); input(field).props.onChange({ target: { value } }); field.update(); }
function blur(field) { input(field).props.onBlur(); field.update(); }
function key(field, value) { input(field).props.onKeyDown({ key: value, preventDefault() {}, currentTarget: { blur() { blur(field); } } }); field.update(); }

test('Speed keeps complete supported decimal requests exact and accepts the native range endpoints', () => {
  for (const [text, speed] of [['.05', r(1, 20)], ['0.5', r(1, 2)], ['2', r(2)], ['32', r(32)], ['1.25', r(5, 4)], ['2.5e-1', r(1, 4)], ['+2.000', r(2)], ['0.123456789', r(123456789, 1000000000)]]) assert.deepEqual(parseSpeedDraft(text), { speed, error: null });
  assert.deepEqual(retimeCommand('clip', r(1), '.5').command, { type: 'retime_clip', id: 'clip', speed: r(1, 2) });
  assert.deepEqual(retimeCommand('clip', r(1), '1.0'), { command: null, error: null });
  assert.deepEqual(retimeCommand('clip', r(1, 3), String(1 / 3)), { command: null, error: null }, 'Unchanged persisted rational display cannot create an edit');
});

test('invalid, incomplete, out-of-range and excessive-precision drafts never silently clamp or issue a command', () => {
  for (const text of ['', ' ', '.', '1.', '1e', '1e-', '-', '-.5', '0', '.049', '32.1', 'Infinity', 'NaN', '0x2', '2×', '1.1234567891', '0.049999999999999999999']) {
    const result = retimeCommand('clip', r(1), text); assert.equal(result.command, null, text); assert.equal(typeof result.error, 'string', text);
  }
});

test('actual Easy and Advanced Speed controls send the same native retime command only after Enter or blur', async () => {
  for (const easy of [true, false]) {
    const edits = [], p = project(), h = inspector(p, easy, async command => edits.push(command));
    type(h.field, '2'); assert.equal(edits.length, 0, 'Typing is not a timeline edit'); key(h.field, 'Enter');
    assert.deepEqual(edits, [{ type: 'retime_clip', id: 'clip', speed: r(2) }]);
    await flush(); h.field.update({ ...h.speed.props, speed: r(2) }); type(h.field, '.5'); blur(h.field);
    assert.deepEqual(edits.at(-1), { type: 'retime_clip', id: 'clip', speed: r(1, 2) });
    assert.ok(find(h.view.tree, node => node.type === 'p' && node.props.children === 'Audio pitch changes with speed.'));
    const next = project('video', { speed: r(2), duration: 240 }); h.view.update({ project: next, selected: ['clip'], frame: 0, edit: async () => {}, easy });
    const duration = find(h.view.tree, node => node.type === 'Field' && node.props.label === 'Duration' && find(node, child => child.type === 'input' && child.props.readOnly));
    assert.equal(find(duration, node => node.type === 'input').props.value, '240 frames · 00:00:08:00');
  }
});

test('actual Speed field restores Escape, reports invalid drafts nearby and never double-admits a pending edit', async () => {
  const held = deferred(), edits = [], h = inspector(project(), true, command => { edits.push(command); return held.promise; });
  type(h.field, '.5'); key(h.field, 'Escape'); assert.equal(edits.length, 0); assert.equal(input(h.field).props.value, '1');
  type(h.field, '1.'); blur(h.field); assert.equal(edits.length, 0); assert.equal(input(h.field).props['aria-invalid'], true);
  const error = find(h.field.tree, node => node.props?.role === 'alert'); assert.match(error.props.children, /complete speed/); assert.equal(input(h.field).props['aria-describedby'], error.props.id);
  type(h.field, '2'); blur(h.field); assert.equal(edits.length, 1); assert.equal(input(h.field).props.disabled, true); blur(h.field); assert.equal(edits.length, 1);
  held.resolve(); await flush(); h.field.update({ ...h.speed.props, speed: r(2) }); assert.equal(input(h.field).props.disabled, false); assert.equal(input(h.field).props.value, '2');
});

test('actual Speed field restores the committed speed after native rejection without losing useful feedback', async () => {
  const h = inspector(project(), false, async () => { throw new Error('Speed change would overlap the next clip'); });
  type(h.field, '.5'); blur(h.field); await flush(); h.field.update();
  assert.equal(input(h.field).props.value, '1'); assert.equal(input(h.field).props.disabled, false);
  assert.match(find(h.field.tree, node => node.props?.role === 'alert').props.children, /overlap the next clip/);
});

test('actual Inspector enables audio/video speed and disables titles, still images and locked tracks with explanations', () => {
  for (const kind of ['video', 'audio']) assert.equal(input(inspector(project(kind), true).field).props.disabled, false);
  for (const p of [project('image'), project('video', { media_id: null, title: 'Title' }), { ...project(), tracks: project().tracks.map(track => ({ ...track, locked: true })) }]) {
    const edits = [], h = inspector(p, true, async command => edits.push(command));
    assert.equal(input(h.field).props.disabled, true); assert.ok(h.speed.props.disabledReason);
    type(h.field, '2'); blur(h.field); assert.equal(edits.length, 0, 'The actual command gate also rejects disabled events');
    assert.match(find(h.field.tree, node => node.props?.className === 'field-help').props.children, /Trim out|Unlock/);
  }
});

test('retime source span, envelope and inherited source phase all participate in the production render descriptor', () => {
  const p = project(), current = previewModelDescriptor(p, 540, false);
  const retimed = project('video', { speed: r(2), duration: 240, retime: { source_span: r(16), envelope: { keyframes: [], fade_in: r(0), fade_out: r(0), fade_in_start: r(0), fade_out_end: r(16) }, render_source_origin: r(0) } });
  assert.notEqual(previewModelDescriptor(retimed, 540, false), current);
  for (const change of [clip => { clip.retime.source_span = r(15); }, clip => { clip.retime.envelope.fade_in = r(1); }, clip => { clip.retime.render_source_origin = r(1); }]) {
    const changed = structuredClone(retimed); change(changed.clips[0]); assert.notEqual(previewModelDescriptor(changed, 540, false), previewModelDescriptor(retimed, 540, false));
  }
  const metadata = structuredClone(retimed); metadata.name = 'Saved after speed'; metadata.clips[0].name = 'Renamed';
  assert.equal(previewModelDescriptor(metadata, 540, false), previewModelDescriptor(retimed, 540, false));
});

test('retime invalidates current and pending successor; Stop and Undo cannot let old work restore playback', async () => {
  const p = project(), scheduler = new PreviewScheduler(), held = deferred(), assets = [], renders = [], cancellations = [], caches = new Map();
  let descriptor = previewModelDescriptor(p, 540, false), serial = 0, encodes = 0, hold = false, old, hidden = 0;
  const key = region => `${descriptor}:${region.start_frame}:${region.end_frame}`;
  const port = {
    identity: async settings => ({ key: key(settings.region), program_key: descriptor, region: settings.region, cached_path: caches.get(key(settings.region)) ?? null }),
    intent: async (_settings, identity, revision) => ({ key: identity, revision, session: 1 }),
    cancel: async revision => cancellations.push(revision),
    render: async (settings, identity) => {
      const cached = caches.get(identity);
      const job = { id: `job-${++serial}`, kind: 'preview', status: 'complete', progress: 1, path: cached ?? `prepared-${serial}.mp4`, error: null, preview_key: identity, preview_region: { ...settings.region } }; renders.push(job);
      if (!cached) encodes++;
      if (!cached && hold && settings.region.start_frame === 150) { old = job; await held.promise; }
      caches.set(identity, job.path); return job;
    },
  };
  const transport = new ProgramTransport(() => {}), bridge = new PreviewBridge(scheduler, port);
  const controller = new ProgramPreviewController(scheduler, bridge, () => ({ height: 540, useProxies: false }), {
    asset: (job, region, playable) => { assets.push(job); transport.resume(region, playable); controller.setPlaybackSpeed(transport.currentSpeed); controller.acknowledgeAsset(job.id); },
    successor: () => {}, job: () => {}, error: error => { throw error; }, unavailable: reason => { hidden++; transport.invalidate(reason === 'seek'); },
  }, { now: () => 0, setTimeout: callback => { queueMicrotask(callback); return 0; }, clearTimeout: () => {} });
  const observe = next => { descriptor = previewModelDescriptor(next, 540, false); return controller.observe(descriptor, next.fps, next.clips[0].duration, 0, true); };
  observe(p); await controller.request(true); await flush(); hold = true; transport.request(1); controller.setPlaybackSpeed(1); await flush(); assert.ok(old, 'Actual successor admission was held');
  const retimed = project('video', { speed: r(2), duration: 240, retime: { source_span: r(16), envelope: { keyframes: [], fade_in: r(0), fade_out: r(0), fade_in_start: r(0), fade_out_end: r(16) }, render_source_origin: r(0) } });
  assert.equal(observe(retimed), true); assert.equal(controller.coverage, null); assert.equal(controller.successor, null); assert.equal(transport.currentSpeed, 0);
  transport.request(0); controller.stopPlayback(); const count = assets.length; held.resolve(); await flush(); controller.receive(old); await flush(); assert.equal(assets.length, count); assert.equal(controller.coverage, null); assert.equal(transport.currentSpeed, 0);
  assert.equal(observe(p), true, 'Undo returns to its own previous render descriptor'); hold = false;
  const encoded = encodes, rendered = renders.length; await controller.request(true); await flush();
  assert.equal(encodes, encoded, 'Native cache admission may reuse its exact descriptor without another encode');
  assert.ok(renders.slice(rendered).every(job => job.preview_key.startsWith(previewModelDescriptor(p, 540, false))), 'Undo admits only its own original descriptor');
  assert.equal(controller.playable, true); assert.equal(transport.currentSpeed, 0, 'Undo does not restore cancelled Play intent');
  const current = assets.at(-1); controller.receive(old); await flush(); assert.equal(assets.at(-1), current); assert.equal(controller.successor, null); assert.ok(cancellations.length > 0); assert.ok(hidden > 1);
  controller.dispose();
});

after(() => assert.equal(readFileSync(inspectorPath, 'utf8'), original, 'Actual Inspector source changed during tests'));
