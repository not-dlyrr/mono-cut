import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { join } from 'node:path';
const require = createRequire(import.meta.url), build = process.env.MONO_CUT_PREVIEW_TEST_BUILD;
const { PreviewScheduler } = require(join(build, 'previewScheduler.js'));
const { PreviewBridge } = require(join(build, 'previewBridge.js'));
const { ProgramPreviewController, PREVIEW_DEBOUNCE_MS } = require(join(build, 'programPreview.js'));
const { globalPreviewFrame, localPreviewTime, regionContains, previewRegion, validRegion } = require(join(build, 'previewRegion.js'));
const { monitorClockActive, monitorEventAllowed, settleMonitorPlay } = require(join(build, 'monitorLifecycle.js'));
const { ProgramTransport } = require(join(build, 'programTransport.js'));
const { editorShortcutAllowed } = require(join(build, 'editorShortcuts.js'));
const GuidedTour = require(join(build, 'GuidedTour.js')).default;
const React = require('react'), { renderToStaticMarkup } = require('react-dom/server');

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const flush = async () => { for (let n = 0; n < 20; n++) await Promise.resolve(); };
function controlledClock() {
  let now = 0, serial = 0; const pending = new Map();
  return {
    now: () => now,
    setTimeout: (callback, delay) => { const id = ++serial; pending.set(id, { callback, at: now + delay }); return id; },
    clearTimeout: id => pending.delete(id),
    async advance(milliseconds) {
      const until = now + milliseconds;
      while (true) { const due = [...pending].filter(([, item]) => item.at <= until).sort((a, b) => a[1].at - b[1].at)[0]; if (!due) break; now = due[1].at; pending.delete(due[0]); due[1].callback(); await flush(); }
      now = until; await flush();
    },
    get pending() { return pending.size; },
  };
}
function setup(overrides = {}, frame = 900, callbacks = {}) {
  const scheduler = new PreviewScheduler(), timer = controlledClock(), calls = [], assets = [], errors = [], times = [], caches = new Map();
  let model = 'A', stamp = 'source-1', serial = 0, hidden = 0;
  const key = region => `${model}:${stamp}:${region.start_frame}:${region.end_frame}`;
  const complete = (options, expectedKey, id = `job-${++serial}`) => ({ id, kind: 'preview', status: 'complete', progress: 1, path: `${expectedKey}.mp4`, error: null, preview_key: expectedKey, preview_region: { ...options.region } });
  const port = {
    identity: async options => { calls.push({ kind: 'identity', options }); const identity = key(options.region); return { key: identity, program_key: `${model}:${stamp}`, region: { ...options.region }, cached_path: caches.get(identity) || null }; },
    intent: async (options, expectedKey, revision) => { calls.push({ kind: 'intent', key: expectedKey, options, revision }); return { key: expectedKey, revision, session: 3 }; },
    render: async (options, expectedKey, intent) => { calls.push({ kind: 'render', key: expectedKey, options, intent }); const job = complete(options, expectedKey); caches.set(expectedKey, job.path); return job; },
    ...overrides,
  };
  const bridge = new PreviewBridge(scheduler, port);
  const controller = new ProgramPreviewController(scheduler, bridge, () => ({ height: 540, useProxies: true }), {
    asset: (job, region, playable) => { assets.push({ job, region, playable, at: timer.now() }); callbacks.asset?.(job, region, playable); },
    job: job => calls.push({ kind: 'job', job }), unavailable: reason => { hidden++; callbacks.unavailable?.(reason); }, error: error => errors.push(error), timing: event => times.push(event),
  }, timer);
  controller.observe(model, { num: 30, den: 1 }, 1800, frame, true);
  return { controller, scheduler, bridge, timer, calls, assets, errors, times, caches, complete, key, get hidden() { return hidden; }, set stamp(value) { stamp = value; }, edit(value, at = frame, fps = { num: 30, den: 1 }, length = 1800, reset = false) { model = value; return controller.observe(value, fps, length, at, true, reset); } };
}

test('monitor mapping preserves rational nonzero global origins, first/last frame and half-open coverage', () => {
  const fps = { num: 30000, den: 1001 }, region = { start_frame: 827, end_frame: 977 };
  for (const frame of [827, 828, 910, 976]) assert.equal(globalPreviewFrame(localPreviewTime(frame, fps, region), fps, region), frame);
  assert.equal(localPreviewTime(827, fps, region), 0);
  assert.equal(globalPreviewFrame(1000, fps, region), 976);
  assert.equal(regionContains(region, 976), true); assert.equal(regionContains(region, 977), false);
  assert.equal(validRegion({ start_frame: 827.5, end_frame: 977 }), false);
  assert.deepEqual(previewRegion(1799, 1800, { num: 30, den: 1 }), { start_frame: 1650, end_frame: 1800 });
  assert.deepEqual(previewRegion(99, 100, { num: 30, den: 1 }), { start_frame: 0, end_frame: 100 });
});

test('actual production scheduling counts the 100ms debounce and shows correct frame before five-second coverage', async () => {
  const state = setup({}, 1799);
  await state.controller.request(false);
  assert.equal(state.calls.filter(c => c.kind === 'render').length, 0);
  await state.timer.advance(PREVIEW_DEBOUNCE_MS - 1); assert.equal(state.assets.length, 0);
  await state.timer.advance(1);
  assert.equal(state.assets.length, 2);
  assert.deepEqual(state.assets[0].region, { start_frame: 1799, end_frame: 1800 }); assert.equal(state.assets[0].playable, false);
  assert.deepEqual(state.assets[1].region, { start_frame: 1650, end_frame: 1800 }); assert.equal(state.assets[1].playable, true);
  assert.deepEqual(state.times.filter(t => t.kind === 'dispatch').map(t => [t.stage, t.at]), [['frame', 100], ['playback', 100]]);
  assert.equal(state.hidden, 1, 'correct new still stays visible while continuous coverage prepares');
  assert.equal(state.controller.boundary(), null);
});

test('native identity and intent delays count before the actual production debounce', async () => {
  const identity = deferred(), intent = deferred(); const state = setup({ identity: () => identity.promise, intent: () => intent.promise });
  const request = state.controller.request(); await state.timer.advance(11);
  identity.resolve({ key: 'held', program_key: 'held-program', region: { start_frame: 900, end_frame: 901 }, cached_path: null }); await flush();
  await state.timer.advance(17); intent.resolve({ key: 'held', revision: 2, session: 3 }); await request;
  await state.timer.advance(99); assert.equal(state.calls.some(c => c.kind === 'render'), false);
  await state.timer.advance(1); assert.equal(state.times.find(t => t.kind === 'dispatch').at, 128);
  state.controller.dispose();
});

test('seeking within coverage and metadata Save keep validated playback without another encoder', async () => {
  const state = setup(); await state.controller.request(); await state.timer.advance(100);
  const count = state.calls.filter(c => c.kind === 'render').length, hidden = state.hidden;
  assert.equal(state.controller.seek(1000), false); assert.equal(state.edit('A', 1000), false);
  await state.controller.request(); await flush();
  assert.equal(state.calls.filter(c => c.kind === 'render').length, count);
  assert.equal(state.controller.playable, true); assert.equal(state.hidden, hidden);
  assert.equal(state.calls.at(-1).kind, 'intent', 'cache shortcut still supersedes obsolete native intents');
});

test('seeking beyond prepared coverage hides unavailable footage and prepares two exact adjoining regions', async () => {
  const state = setup({}, 900); await state.controller.request(); await state.timer.advance(100);
  assert.deepEqual(state.controller.coverage, { start_frame: 900, end_frame: 1050 });
  const next = state.controller.boundary(); assert.equal(next, 1050);
  assert.equal(state.controller.seek(next), true); assert.equal(state.controller.coverage, null); assert.equal(state.controller.playable, false);
  await state.controller.request(); await state.timer.advance(100);
  assert.deepEqual(state.controller.coverage, { start_frame: 1050, end_frame: 1200 });
  assert.equal(globalPreviewFrame(0, { num: 30, den: 1 }, state.controller.coverage), 1050);
  assert.equal(state.controller.seek(1600), true); await state.controller.request(); await state.timer.advance(100);
  assert.deepEqual(state.controller.coverage, { start_frame: 1600, end_frame: 1750 });
});

test('wrong-region native identity or render replies are rejected and allow a correct retry', async () => {
  let wrong = true; const state = setup({ identity: async options => ({ key: state.key(options.region), program_key: 'program-K', region: wrong ? { start_frame: 0, end_frame: 1 } : options.region, cached_path: null }), render: async (options, key) => ({ ...state.complete(options, key), preview_region: wrong ? { start_frame: 0, end_frame: 1 } : options.region }) });
  await state.controller.request(); assert.match(String(state.errors[0]), /different sequence region/); assert.equal(state.assets.length, 0);
  wrong = false; await state.bridge.check({ height: 540, useProxies: true, region: state.controller.pendingRegion }); wrong = true;
  await assert.rejects(state.bridge.render({ height: 540, useProxies: true, region: state.controller.pendingRegion }), /different render identity/);
  wrong = false; await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.controller.playable, true);
});

test('new edit and seek intents cancel queued timers and reject late native preparation replies/events', async () => {
  const held = deferred(); let old;
  const state = setup({ render: async (options, key) => { state.calls.push({ kind: 'render', key, options }); if (!old) { old = state.complete(options, key, 'old'); return held.promise; } return state.complete(options, key); } });
  await state.controller.request(); await state.timer.advance(100);
  state.edit('B', 300); await state.controller.request(); state.controller.seek(1500); await state.controller.request();
  assert.equal(state.timer.pending, 1); await state.timer.advance(100);
  assert.equal(state.assets.every(asset => asset.job.preview_key.startsWith('B:') && regionContains(asset.region, 1500)), true);
  const assets = state.assets.length; held.resolve(old); await flush(); state.controller.receive(old);
  assert.equal(state.assets.length, assets); assert.deepEqual(state.controller.coverage, { start_frame: 1500, end_frame: 1650 });
});

test('failed continuous preparation preserves the newly correct still and retry never exposes an old edit', async () => {
  let fail = true; const state = setup({ render: async (options, key) => options.region.end_frame - options.region.start_frame > 1 && fail ? { ...state.complete(options, key), status: 'failed', path: null, error: 'encode failed' } : state.complete(options, key) });
  await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.assets.length, 1); assert.equal(state.assets[0].playable, false); assert.equal(state.hidden, 1); assert.equal(state.errors[0], 'encode failed');
  fail = false; await state.controller.request(true); await flush();
  assert.equal(state.controller.playable, true); assert.equal(state.assets.length, 2);
});

test('settings/project replacement and changed file stamps invalidate displayed region and old session completions', async () => {
  const state = setup(); await state.controller.request(); await state.timer.advance(100);
  const old = state.assets.at(-1).job;
  state.stamp = 'replaced-source'; await state.controller.request(); await state.timer.advance(100);
  assert.ok(state.assets.at(-1).job.preview_key.includes('replaced-source')); assert.equal(state.hidden, 2);
  state.edit('settings-360-proxies-off', 300, { num: 30000, den: 1001 }, 1800, true); state.controller.receive(old);
  assert.equal(state.controller.coverage, null); await state.controller.request(); await state.timer.advance(100);
  assert.deepEqual(state.controller.coverage, { start_frame: 300, end_frame: 450 });
  state.edit('same-id-reopened', 1200, { num: 30, den: 1 }, 1800, true); state.controller.receive(old);
  assert.equal(state.controller.coverage, null);
});

test('dispose and close cancellation release queued callbacks; remount can prepare a fresh session', async () => {
  const state = setup(); await state.controller.request(); assert.equal(state.timer.pending, 1);
  state.controller.dispose(); await state.timer.advance(1000);
  assert.equal(state.calls.some(c => c.kind === 'render'), false); assert.equal(state.assets.length, 0);
  state.controller.activate(); state.edit('remounted', 600, { num: 30, den: 1 }, 1800, true); await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.controller.playable, true);
  state.controller.cancelPreparation(); state.edit('close-failed-retry', 600); await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.controller.playable, true);
});

test('production cached-region Undo publishes A before reusing its output and obsolete B cannot revive', async () => {
  const heldB = deferred(); let oldB, encodes = 0;
  const state = setup({ render: async (options, key, intent) => {
    state.calls.push({ kind: 'render', key, options, intent });
    if (key.startsWith('B:')) { oldB = state.complete(options, key, 'obsolete-B'); return heldB.promise; }
    const path = state.caches.get(key); if (!path) encodes++;
    const job = state.complete(options, key); state.caches.set(key, job.path); return job;
  } });
  await state.controller.request(); await state.timer.advance(100); const firstEncodes = encodes;
  state.edit('B'); await state.controller.request(); await state.timer.advance(100);
  state.edit('A'); await state.controller.request(); await state.timer.advance(100);
  assert.equal(encodes, firstEncodes, 'Undo only admits previously verified cached files');
  assert.equal(state.controller.playable, true);
  const assetCount = state.assets.length, newestIntent = state.calls.filter(c => c.kind === 'intent').at(-1);
  assert.ok(newestIntent.key.startsWith('A:'));
  assert.ok(newestIntent.revision > state.calls.find(c => c.kind === 'intent' && c.key.startsWith('B:')).revision);
  heldB.resolve(oldB); await flush(); state.controller.receive(oldB);
  assert.equal(state.assets.length, assetCount); assert.ok(state.assets.at(-1).job.preview_key.startsWith('A:'));
});

test('metadata during preparation coalesces the current region and duplicate completion is idempotent', async () => {
  const held = deferred(); let first;
  const state = setup({ render: async (options, key) => { state.calls.push({ kind: 'render', key }); if (!first) { first = state.complete(options, key, 'shared'); return held.promise.then(job => { state.caches.set(key, job.path); return job; }); } const job = state.complete(options, key); state.caches.set(key, job.path); return job; } });
  await state.controller.request(); await state.timer.advance(100);
  state.edit('A'); await state.controller.request(); await state.controller.request(); await state.timer.advance(200);
  assert.equal(state.calls.filter(c => c.kind === 'render').length, 1);
  held.resolve(first); await flush();
  const latest = state.assets.at(-1).job, assetCount = state.assets.length;
  state.controller.receive(latest); await state.controller.request();
  assert.equal(state.assets.length, assetCount); assert.equal(state.controller.playable, true);
});

test('a changed source stamp while five-second work is pending immediately hides the old first frame', async () => {
  const held = deferred(); let old;
  const state = setup({ render: async (options, key) => {
    if (options.region.end_frame - options.region.start_frame > 1 && key.includes('source-1')) { old = state.complete(options, key, 'old-continuous'); return held.promise; }
    return state.complete(options, key);
  } });
  await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.assets.length, 1); assert.equal(state.controller.coverage.end_frame, 901);
  state.stamp = 'source-2'; await state.controller.request();
  assert.equal(state.controller.coverage, null); assert.equal(state.hidden, 2);
  held.resolve(old); await flush(); state.controller.receive(old); assert.equal(state.controller.coverage, null);
  await state.timer.advance(100); assert.equal(state.controller.playable, true);
  assert.ok(state.assets.at(-1).job.preview_key.includes('source-2'));
});

test('program stamp identity also detects source changes between first-frame completion and initial continuous identity', async () => {
  let changed = false;
  const state = setup({ identity: async options => {
    if (!changed && options.region.end_frame - options.region.start_frame > 1) { state.stamp = 'source-2'; changed = true; }
    return { key: state.key(options.region), program_key: changed ? 'A:source-2' : 'A:source-1', region: options.region, cached_path: null };
  } });
  await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.hidden, 2); assert.equal(state.controller.coverage, null);
  assert.equal(state.assets.length, 1, 'first frame was only available before the source changed');
  await state.timer.advance(100); assert.equal(state.controller.playable, true);
  assert.ok(state.assets.at(-1).job.preview_key.includes('source-2'));
});

test('failed identity validation rejects pending completions until an explicit correct retry', async () => {
  const held = deferred(); let first, failed = false;
  const state = setup({ identity: async options => {
    if (failed) throw new Error('source missing');
    return { key: state.key(options.region), program_key: 'program-A', region: options.region, cached_path: null };
  }, render: async (options, key) => { if (!first) { first = state.complete(options, key, 'pending'); return held.promise; } return state.complete(options, key); } });
  await state.controller.request(); await state.timer.advance(100);
  failed = true; await state.controller.request(); held.resolve(first); await flush(); state.controller.receive(first);
  assert.equal(state.assets.length, 0); assert.equal(state.controller.coverage, null); assert.match(String(state.errors[0]), /source missing/);
  failed = false; await state.controller.request(); await state.timer.advance(100); assert.equal(state.controller.playable, true);
});

test('a source-change failure detected during continuous encoding immediately clears its first-frame asset', async () => {
  const state = setup({ render: async (options, key) => options.region.end_frame - options.region.start_frame > 1 ? { ...state.complete(options, key), status: 'failed', path: null, error: 'PREVIEW_IDENTITY_CHANGED: source changed while rendering' } : state.complete(options, key) });
  await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.assets.length, 1); assert.equal(state.controller.coverage, null); assert.equal(state.hidden, 2);
  assert.match(String(state.errors[0]), /PREVIEW_IDENTITY_CHANGED/);
});

test('Error-object identity failures from a render RPC invalidate its first frame and permit an explicit retry', async () => {
  let fail = true;
  const state = setup({ render: async (options, key) => {
    if (fail && options.region.end_frame - options.region.start_frame > 1) throw new Error('PREVIEW_IDENTITY_CHANGED: source changed');
    return state.complete(options, key);
  } });
  await state.controller.request(); await state.timer.advance(100);
  assert.equal(state.assets.length, 1); assert.equal(state.controller.coverage, null); assert.equal(state.hidden, 2);
  assert.match(state.errors[0].message, /^PREVIEW_IDENTITY_CHANGED:/);
  fail = false; await state.controller.request(); await state.timer.advance(100); assert.equal(state.controller.playable, true);
});

const lifecycleSource = 'http://asset.localhost/current.mp4?preview=current';
const lifecycleVideo = () => ({ currentSrc: lifecycleSource, readyState: 4, currentTime: 5, duration: 5, paused: true, ended: true, error: null });
const lifecycle = (kind, video, options = {}) => monitorEventAllowed(kind, video, options.src ?? lifecycleSource, options.node ?? true, options.timestamp ?? 120, 100, 1700000000000, { num: 30, den: 1 }, options.region ?? { start_frame: 900, end_frame: 1050 });

test('production monitor lifecycle rejects old source/ref and pre-generation events on a reused media element', () => {
  const video = lifecycleVideo();
  for (const kind of ['metadata', 'play', 'pause', 'ended', 'error']) {
    assert.equal(lifecycle(kind, video, { src: 'http://asset.localhost/old.mp4' }), false);
    assert.equal(lifecycle(kind, video, { node: false }), false);
    assert.equal(lifecycle(kind, video, { timestamp: 99 }), false);
  }
  assert.equal(lifecycle('metadata', video), true);
  assert.equal(lifecycle('metadata', video, { timestamp: 1700000000120 }), true, 'epoch event timestamps use the same generation clock');
  assert.equal(lifecycle('metadata', video, { timestamp: 1700000000099 }), false);
});

test('a queued ended event cannot advance a newly sought region until media actually ended at exact covered end', () => {
  const video = lifecycleVideo();
  assert.equal(lifecycle('ended', video), true);
  assert.equal(lifecycle('ended', { ...video, ended: false }), false);
  assert.equal(lifecycle('ended', { ...video, currentTime: 1.4 }), false);
  assert.equal(lifecycle('ended', { ...video, duration: 2, currentTime: 2 }), false, 'a truncated asset cannot skip unprepared coverage');
  assert.equal(lifecycle('ended', { ...video, currentTime: NaN }), false);
  const rational = { num: 30000, den: 1001 }, region = { start_frame: 827, end_frame: 977 }, end = 150 * 1001 / 30000;
  assert.equal(monitorEventAllowed('ended', { ...video, currentTime: end, duration: end }, lifecycleSource, true, 120, 100, 1700000000000, rational, region), true);
});

test('old metadata, pause, play and error events must agree with the current media state', () => {
  const video = lifecycleVideo();
  assert.equal(lifecycle('metadata', { ...video, readyState: 0 }), false);
  assert.equal(lifecycle('metadata', { ...video, duration: NaN }), false);
  assert.equal(lifecycle('pause', { ...video, paused: false }), false);
  assert.equal(lifecycle('pause', video), true);
  assert.equal(lifecycle('play', video), false);
  assert.equal(lifecycle('play', { ...video, paused: false }), true);
  assert.equal(lifecycle('error', video), false);
  assert.equal(lifecycle('error', { ...video, error: { code: 3 } }), true);
});

test('source monitor lifecycle uses its own full duration while program end editing coordinates remain global', () => {
  const video = { ...lifecycleVideo(), currentTime: 9, duration: 10 };
  assert.equal(monitorEventAllowed('ended', video, lifecycleSource, true, 120, 100, 1700000000000, { num: 30, den: 1 }), false);
  video.currentTime = 10;
  assert.equal(monitorEventAllowed('ended', video, lifecycleSource, true, 120, 100, 1700000000000, { num: 30, den: 1 }), true);
  const cursor = 1800, coverage = { start_frame: 1650, end_frame: 1800 };
  assert.equal(localPreviewTime(cursor, { num: 30, den: 1 }, coverage), 149 / 30);
  assert.equal(cursor, 1800, 'preview seek conversion does not change the sequence-end editing cursor');
});

test('an obsolete play promise rejection cannot pause a newer region, while a current rejection still pauses', async () => {
  const old = deferred(), latest = deferred(); let source = 'old-region.mp4', currentNode = 'old-video', generation = 1, paused = 0;
  const pendingOld = settleMonitorPlay(old.promise, () => source === 'old-region.mp4' && currentNode === 'old-video' && generation === 1, () => paused++);
  source = 'new-region.mp4'; currentNode = 'new-video'; generation = 2;
  const pendingNew = settleMonitorPlay(latest.promise, () => source === 'new-region.mp4' && currentNode === 'new-video' && generation === 2, () => paused++);
  old.reject(new Error('AbortError: old element removed')); await pendingOld;
  assert.equal(paused, 0, 'newer forward transport keeps playing after an obsolete failure');
  latest.reject(new Error('NotAllowedError: current playback rejected')); await pendingNew;
  assert.equal(paused, 1, 'the current play failure still stops its transport');
});

test('an old play rejection after stop and restart on the same asset cannot pause the current intent', async () => {
  const old = deferred(); let active = true, paused = 0;
  const pending = settleMonitorPlay(old.promise, () => active, () => paused++);
  active = false; // React effect cleanup invalidates the earlier play intent even if src/ref are unchanged.
  old.reject(new Error('AbortError: earlier play was paused')); await pending;
  assert.equal(paused, 0);
});

test('a deferred native identity bounds a 250-seek flood to one RPC and one replaceable latest check', async () => {
  const held = deferred(), sampled = []; let active = 0, maximum = 0;
  const state = setup({ identity: async options => {
    const region = { ...options.region }; sampled.push(region); active++; maximum = Math.max(maximum, active);
    if (sampled.length === 1) await held.promise;
    active--;
    return { key: state.key(region), program_key: 'A:source-1', region, cached_path: null };
  } });
  const first = state.controller.request(), requests = [];
  for (let n = 0; n < 250; n++) { state.controller.seek(1000 + n); requests.push(state.controller.request()); }
  await Promise.all(requests.slice(0, -1));
  assert.equal(sampled.length, 1, 'superseded queued callers finish before the held native identity');
  assert.equal(maximum, 1); assert.equal(state.assets.length, 0); assert.equal(state.timer.pending, 0);
  held.resolve(); await first; await requests.at(-1);
  assert.deepEqual(sampled, [{ start_frame: 900, end_frame: 901 }, { start_frame: 1249, end_frame: 1250 }]);
  assert.deepEqual(state.calls.filter(call => call.kind === 'intent').map(call => call.options.region), [{ start_frame: 1249, end_frame: 1250 }]);
  await state.timer.advance(100);
  assert.equal(maximum, 1); assert.equal(sampled.length, 3, 'only the newest frame and its continuous region are prepared');
  assert.deepEqual(state.controller.coverage, { start_frame: 1249, end_frame: 1399 });
  assert.equal(state.controller.playable, true); assert.equal(state.errors.length, 0);
});

test('identical metadata checks share a held useful identity and prepare the current region only once', async () => {
  const held = deferred(); let nativeCalls = 0;
  const state = setup({ identity: async options => {
    nativeCalls++; if (nativeCalls === 1) await held.promise;
    return { key: state.key(options.region), program_key: 'A:source-1', region: options.region, cached_path: null };
  } });
  const requests = [state.controller.request()];
  for (let n = 0; n < 100; n++) { assert.equal(state.edit('A'), false); requests.push(state.controller.request()); }
  assert.equal(nativeCalls, 1); held.resolve(); await Promise.all(requests);
  assert.equal(nativeCalls, 1); assert.equal(state.timer.pending, 1); assert.equal(state.calls.filter(c => c.kind === 'intent').length, 1);
  await state.timer.advance(100);
  assert.equal(nativeCalls, 2); assert.equal(state.assets.length, 2); assert.equal(state.hidden, 1);
  assert.equal(state.controller.playable, true); assert.equal(state.calls.filter(c => c.kind === 'render').length, 2);
});

test('dispose promptly releases a queued check and an old identity failure cannot poison a remounted session', async () => {
  const held = deferred(); let nativeCalls = 0;
  const state = setup({ identity: async options => {
    nativeCalls++; if (nativeCalls === 1) await held.promise;
    return { key: state.key(options.region), program_key: 'B:source-1', region: options.region, cached_path: null };
  } });
  const initial = state.controller.request(); state.controller.seek(1400); const queued = state.controller.request();
  state.controller.dispose(); await queued; assert.equal(nativeCalls, 1);
  state.controller.activate(); state.edit('B', 1400, { num: 30, den: 1 }, 1800, true); const reopened = state.controller.request();
  held.reject(new Error('old source missing')); await initial; await reopened; await state.timer.advance(100);
  assert.equal(state.errors.length, 0); assert.equal(state.controller.playable, true);
  assert.ok(state.assets.every(asset => asset.job.preview_key.startsWith('B:')));
  assert.deepEqual(state.controller.coverage, { start_frame: 1400, end_frame: 1550 });
});

test('production reverse intent resumes only playable look-behind coverage and continues inside it', async () => {
  const changes = [], adoptedSpeeds = [], transport = new ProgramTransport(speed => changes.push(speed));
  const state = setup({}, 900, { unavailable: reason => transport.invalidate(reason === 'seek'), asset: (_, region, playable) => { transport.resume(region, playable); adoptedSpeeds.push({ playable, speed: transport.currentSpeed }); } });
  const move = (frame, direction) => { const preparing = state.controller.seek(frame, direction); if (preparing) void state.controller.request(); return preparing; };
  await state.controller.request(); await state.timer.advance(100);
  transport.request(-2); assert.equal(transport.advance(898, move), true); await flush();
  assert.equal(transport.currentSpeed, 0, 'reverse waits while no validated playable asset exists');
  await state.timer.advance(100);
  assert.deepEqual(state.controller.coverage, { start_frame: 749, end_frame: 899 });
  assert.equal(transport.currentSpeed, -2); assert.deepEqual(adoptedSpeeds.slice(-2), [{ playable: false, speed: 0 }, { playable: true, speed: -2 }]);
  const calls = state.calls.filter(call => call.kind === 'render').length;
  transport.advance(897, move); await flush();
  assert.equal(transport.currentSpeed, -2); assert.equal(state.calls.filter(call => call.kind === 'render').length, calls);
  assert.deepEqual(previewRegion(0, 1800, { num: 30, den: 1 }, 5, -1), { start_frame: 0, end_frame: 150 });
  assert.deepEqual(previewRegion(1799, 1800, { num: 30, den: 1 }, 5, -1), { start_frame: 1650, end_frame: 1800 });
});

test('stop, manual seek and model invalidation each supersede pending signed transport resume', async () => {
  for (const action of ['stop', 'seek', 'edit']) {
    const transport = new ProgramTransport(() => {}), state = setup({}, 900, { unavailable: reason => transport.invalidate(reason === 'seek'), asset: (_, region, playable) => transport.resume(region, playable) });
    const move = (frame, direction) => { const preparing = state.controller.seek(frame, direction); if (preparing) void state.controller.request(); return preparing; };
    await state.controller.request(); await state.timer.advance(100); const old = state.assets.at(-1).job;
    transport.request(-1); transport.advance(899, move); await flush();
    if (action === 'stop') transport.request(0);
    if (action === 'seek') { transport.cancelResume(); move(1200, 1); }
    if (action === 'edit') { state.edit('B', 1200); void state.controller.request(); }
    const frame = state.controller.pendingRegion.start_frame;
    assert.equal(transport.advance(898, move), false, 'a queued old reverse tick cannot move after the newer intent');
    assert.equal(state.controller.pendingRegion.start_frame, frame);
    await flush(); await state.timer.advance(100); state.controller.receive(old);
    assert.equal(transport.currentSpeed, 0, `${action} wins even after valid or obsolete assets arrive`);
  }
});

test('forward boundary uses the live signed transport intent and stop prevents a queued advance', () => {
  const changes = [], transport = new ProgramTransport(speed => changes.push(speed));
  transport.request(2); let moves = 0;
  transport.advance(1050, () => { moves++; transport.invalidate(true); return true; });
  transport.resume({ start_frame: 900, end_frame: 1050 }, true); assert.equal(transport.currentSpeed, 0, 'an adjoining old region cannot resume the new target');
  transport.resume({ start_frame: 1050, end_frame: 1051 }, false); assert.equal(transport.currentSpeed, 0);
  transport.resume({ start_frame: 1050, end_frame: 1200 }, true); assert.equal(transport.currentSpeed, 2);
  transport.request(0); assert.equal(transport.advance(1200, () => { moves++; return true; }), false); assert.equal(moves, 1);
});

test('play before metadata recovers through current playing and successful play, while obsolete success is ignored', async () => {
  const cold = { ...lifecycleVideo(), readyState: 0, paused: false, ended: false };
  assert.equal(lifecycle('play', cold), false);
  assert.equal(lifecycle('playing', cold), false);
  assert.equal(lifecycle('playing', { ...cold, readyState: 3 }), true);
  assert.equal(lifecycle('playing', { ...cold, readyState: 3 }, { timestamp: 99 }), false, 'old-generation playing event is rejected');
  const old = deferred(), current = deferred(); let generation = 1, tracked = 0, paused = 0;
  const first = settleMonitorPlay(old.promise, () => generation === 1, () => paused++, () => tracked++);
  generation = 2;
  const latest = settleMonitorPlay(current.promise, () => generation === 2, () => paused++, () => tracked++);
  old.resolve(); await first; assert.equal(tracked, 0);
  current.resolve(); await latest; assert.equal(tracked, 1); assert.equal(paused, 0);
});

test('a changed model during coverage advance cannot retain an earlier negative transport intent', async () => {
  const transport = new ProgramTransport(() => {}), state = setup({}, 900, { unavailable: reason => transport.invalidate(reason === 'seek'), asset: (_, region, playable) => transport.resume(region, playable) });
  await state.controller.request(); await state.timer.advance(100); transport.request(-1);
  transport.advance(899, (frame, direction) => {
    const preparing = state.controller.seek(frame, direction);
    state.edit('B', frame); void state.controller.request();
    return preparing;
  });
  await flush(); await state.timer.advance(100);
  assert.equal(state.controller.playable, true); assert.equal(transport.currentSpeed, 0);
  assert.ok(state.assets.at(-1).job.preview_key.startsWith('B:'));
});

test('controlled production tutorial preserves Export step through modal unmount and supports explicit restart', () => {
  let step = 6;
  const tour = () => React.createElement(GuidedTour, { step, setStep: value => { step = value; }, onStep: () => {}, onClose: () => {} });
  assert.match(renderToStaticMarkup(tour()), /Export your finished video/);
  assert.equal(renderToStaticMarkup(null), '', 'Export modal suspends the tutorial');
  const reopened = renderToStaticMarkup(tour());
  assert.match(reopened, /Export your finished video/); assert.match(reopened, /Quick tour · 7 of 7/);
  step = 0; assert.match(renderToStaticMarkup(tour()), /Bring in your media/);
});

test('stopped or replaced monitors cannot run a retained audio/frame clock before event cleanup', () => {
  assert.equal(monitorClockActive(true, 1, 'old.wav', 'old.wav', true), true);
  assert.equal(monitorClockActive(true, 0, 'old.wav', 'old.wav', true), false, 'Stop takes effect while playing state is still true');
  assert.equal(monitorClockActive(true, -1, 'old.wav', 'old.wav', true), false, 'reverse uses the explicit silent seek clock');
  assert.equal(monitorClockActive(true, 1, 'old.wav', 'new.wav', true), false, 'old source callback cannot track a replacement');
  assert.equal(monitorClockActive(true, 1, 'new.wav', 'new.wav', false), false, 'old node cannot track a replacement');
  assert.equal(monitorClockActive(false, 1, 'new.wav', 'new.wav', true), false, 'new source waits for its own playing confirmation');
  assert.equal(monitorClockActive(true, 1, 'new.wav', 'new.wav', true), true);
});

test('focused native and authored controls keep Space/Enter activation while workspace shortcuts remain available', () => {
  const target = selector => ({ closest: value => value.includes(selector) ? {} : null });
  for (const selector of ['button', 'a[href]', 'summary', '[role="button"]', '[role="menuitem"]', '[role="tab"]', '[role="checkbox"]', '[role="switch"]']) {
    assert.equal(editorShortcutAllowed('Space', target(selector), false), false, selector);
    assert.equal(editorShortcutAllowed('Enter', target(selector), false), false, selector);
  }
  for (const selector of ['input', 'textarea', 'select', '[contenteditable="true"]']) assert.equal(editorShortcutAllowed('j', target(selector), false), false);
  const workspace = { closest: () => null };
  for (const key of ['Space', 'Enter', 'j', 'k', 'l', 'ArrowLeft']) assert.equal(editorShortcutAllowed(key, workspace, false), true, key);
  assert.equal(editorShortcutAllowed('Space', workspace, true), false);
});
