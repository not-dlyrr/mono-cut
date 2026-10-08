import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { join } from 'node:path';
const require = createRequire(import.meta.url);
const { previewModelDescriptor } = require(join(process.env.MONO_CUT_PREVIEW_TEST_BUILD, 'previewModel.js'));
const { PreviewScheduler, mergePreviewJob } = require(join(process.env.MONO_CUT_PREVIEW_TEST_BUILD, 'previewScheduler.js'));
const { PlaybackAssetQueue, SourceIntentQueue } = require(join(process.env.MONO_CUT_PREVIEW_TEST_BUILD, 'previewAssets.js'));
const { PreviewBridge } = require(join(process.env.MONO_CUT_PREVIEW_TEST_BUILD, 'previewBridge.js'));

const clone = value => structuredClone(value);
function fixture() {
  const media = { id: 'media-1', name: 'Picture', path: 'picture.mp4', kind: 'video', duration: { num: 10, den: 1 }, fps: { num: 30000, den: 1001 }, width: 1920, height: 1080, has_audio: true, bin_id: null, thumbnail: 'thumbnail.jpg', waveform: [.2, .8], proxy: null, missing: false, timing: { origin: { num: 0, den: 1 }, video_start: { num: 0, den: 1 }, audio_start: { num: 0, den: 1 }, video_end: { num: 10, den: 1 }, audio_end: { num: 10, den: 1 }, video_stream: 0, audio_stream: 1 } };
  const clip = { id: 'clip-1', media_id: media.id, track_id: 'video-1', name: 'First', start: 0, duration: 120, source_in: { num: 0, den: 1 }, speed: { num: 1, den: 1 }, linked_id: null, title: null, transform: { x: 0, y: 0, scale: 1, rotation: 0, crop_left: 0, crop_right: 0, crop_top: 0, crop_bottom: 0 }, opacity: 1, volume: 1, fade_in: 0, fade_out: 0, brightness: 0, contrast: 1, saturation: 1, keyframes: [] };
  return { version: 1, id: 'project-1', name: 'Edit', width: 1920, height: 1080, fps: { num: 30000, den: 1001 }, sample_rate: 48000, media: [media], bins: [], tracks: [{ id: 'video-1', name: 'Video', kind: 'video', muted: false, hidden: false, locked: false }], clips: [clip], markers: [], in_point: null, out_point: null };
}
const descriptor = (project, height = 540, proxies = true) => previewModelDescriptor(project, height, proxies);
const job = (id, key, status = 'running', path = null) => ({ id, kind: 'preview', status, progress: status === 'complete' ? 1 : .3, path, error: null, preview_key: key });
function start(key = 'key-A') {
  const scheduler = new PreviewScheduler(); scheduler.observe(descriptor(fixture()), true);
  scheduler.acceptIdentity(scheduler.beginIdentity(), key);
  return { scheduler, request: scheduler.beginRender() };
}

test('metadata, bin organization, markers, range, link and track labels do not change render inputs', () => {
  const original = fixture(), changed = clone(original);
  changed.id = 'other-project-id'; changed.name = 'Saved name'; changed.bins = [{ id: 'bin-1', name: 'Collection' }];
  changed.markers = [{ id: 'marker', frame: 25, name: 'Cut' }]; changed.in_point = 20; changed.out_point = 100;
  Object.assign(changed.media[0], { name: 'Rename', bin_id: 'bin-1', thumbnail: 'other.jpg', waveform: [1] });
  Object.assign(changed.tracks[0], { name: 'Rename track', locked: true });
  Object.assign(changed.clips[0], { name: 'Rename clip', linked_id: 'linked-clip' });
  assert.equal(descriptor(changed), descriptor(original));
});

test('unused media and empty tracks do not invalidate a sequence', () => {
  const original = fixture(), changed = clone(original);
  changed.media.push({ ...clone(original.media[0]), id: 'unused', path: 'unrelated.mp4', missing: true });
  changed.tracks.push({ ...clone(original.tracks[0]), id: 'empty', hidden: true });
  assert.equal(descriptor(changed), descriptor(original));
});

test('render-affecting edits, routing, source timing and resolution change the descriptor', () => {
  const original = fixture();
  for (const [name, modify] of [
    ['geometry', p => { p.width = 1280; }], ['frame rate', p => { p.fps = { num: 24, den: 1 }; }],
    ['audio sample rate', p => { p.sample_rate = 44100; }], ['mute', p => { p.tracks[0].muted = true; }],
    ['visibility', p => { p.tracks[0].hidden = true; }], ['relink', p => { p.media[0].path = 'relinked.mp4'; }],
    ['source timing', p => { p.media[0].timing.audio_start.num = 1; }], ['trim', p => { p.clips[0].source_in.num = 1; }],
    ['move', p => { p.clips[0].start = 10; }], ['volume', p => { p.clips[0].volume = .5; }],
    ['crop', p => { p.clips[0].transform.crop_left = .1; }], ['color', p => { p.clips[0].brightness = .1; }],
    ['title', p => { p.clips[0].title = 'Actual title'; }], ['keyframe', p => { p.clips[0].keyframes.push({ property: 'x', frame: 10, value: 30 }); }],
    ['composition', p => { p.clips[0].composition = { group_id: 'group', offset: 12 }; }],
    ['render offset', p => { p.clips[0].render_offset = 5; }], ['fade', p => { p.clips[0].fade_in_start = -10; }],
  ]) { const changed = clone(original); modify(changed); assert.notEqual(descriptor(changed), descriptor(original), name); }
  assert.notEqual(descriptor(original, 360), descriptor(original));
});

test('proxy generation is irrelevant while proxy playback is disabled', () => {
  const original = fixture(), changed = clone(original);
  changed.media[0].proxy = 'proxy.mp4'; changed.media[0].proxy_timing_version = 2;
  assert.equal(descriptor(changed, 540, false), descriptor(original, 540, false));
  assert.notEqual(descriptor(changed, 540, true), descriptor(original, 540, true));
  assert.notEqual(descriptor(original, 540, false), descriptor(original, 540, true));
});

test('object, clip and media storage order are irrelevant, while track compositing order is preserved', () => {
  const original = fixture(), reordered = Object.fromEntries(Object.entries(original).reverse());
  assert.equal(descriptor(reordered), descriptor(original));
  const two = fixture(); two.clips.push({ ...clone(two.clips[0]), id: 'clip-2', start: 50 });
  const reversed = clone(two); reversed.clips.reverse(); assert.equal(descriptor(two), descriptor(reversed));
  two.tracks.push({ ...clone(two.tracks[0]), id: 'video-2' }); two.clips[1].track_id = 'video-2';
  const tracksReversed = clone(two); tracksReversed.tracks.reverse(); assert.notEqual(descriptor(two), descriptor(tracksReversed));
});

test('completion arriving before render response waits for the matching active job ID', () => {
  const { scheduler, request } = start(); const completed = job('render-A', 'key-A', 'complete', 'cache-A.mp4');
  assert.equal(scheduler.receiveEvent(completed), null);
  const accepted = scheduler.receiveResponse(request, job('render-A', 'key-A'));
  assert.equal(accepted.status, 'complete'); assert.equal(accepted.path, 'cache-A.mp4');
  assert.equal(scheduler.markReady(accepted), true); assert.equal(scheduler.needsRender(), false);
});

test('metadata checks preserve the active job and ready preview without scheduling another render', () => {
  const { scheduler, request } = start(); scheduler.receiveResponse(request, job('render-A', 'key-A'));
  const revision = scheduler.contextRevision; const renamed = fixture(); renamed.name = 'Saved';
  assert.equal(scheduler.observe(descriptor(renamed), true), false); assert.equal(scheduler.contextRevision, revision);
  assert.equal(scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A').changed, false);
  assert.equal(scheduler.activeJobId, 'render-A'); assert.equal(scheduler.needsRender(), false);
  const complete = scheduler.receiveEvent(job('render-A', 'key-A', 'complete', 'cache-A.mp4')); scheduler.markReady(complete);
  assert.equal(scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', 'cache-A.mp4').ready.path, 'cache-A.mp4');
  assert.equal(scheduler.beginRender(), null);
});

test('stale identity replies, render responses and old completion events cannot replace a new edit', () => {
  const { scheduler, request } = start(); const oldIdentity = scheduler.beginIdentity();
  const changed = fixture(); changed.clips[0].opacity = .4; scheduler.observe(descriptor(changed), true);
  const currentIdentity = scheduler.beginIdentity(); assert.equal(scheduler.acceptIdentity(oldIdentity, 'key-A'), null);
  scheduler.acceptIdentity(currentIdentity, 'key-B'); const next = scheduler.beginRender();
  assert.equal(scheduler.receiveResponse(request, job('render-A', 'key-A', 'complete', 'cache-A.mp4')), null);
  scheduler.receiveResponse(next, job('render-B', 'key-B'));
  assert.equal(scheduler.receiveEvent(job('render-A', 'key-A', 'complete', 'cache-A.mp4')), null);
  assert.equal(scheduler.receiveEvent(job('wrong-job', 'key-B', 'complete', 'wrong.mp4')), null);
  const complete = scheduler.receiveEvent(job('render-B', 'key-B', 'complete', 'cache-B.mp4'));
  assert.equal(scheduler.markReady(complete), true);
});

test('newer native identity check wins and external file changes invalidate identical project metadata', () => {
  const { scheduler, request } = start(); scheduler.receiveResponse(request, job('render-A', 'key-A'));
  const first = scheduler.beginIdentity(), latest = scheduler.beginIdentity();
  assert.equal(scheduler.acceptIdentity(first, 'key-A'), null);
  assert.equal(scheduler.acceptIdentity(latest, 'file-changed-key').changed, true);
  assert.equal(scheduler.receiveEvent(job('render-A', 'key-A', 'complete', 'cache-A.mp4')), null);
  assert.equal(scheduler.needsRender(), true);
});

test('reopening an equivalent project still rejects prior session replies and refreshes cache availability', () => {
  const { scheduler, request } = start(); const identity = scheduler.beginIdentity();
  scheduler.resetProject(); scheduler.observe(descriptor(fixture()), true);
  assert.equal(scheduler.acceptIdentity(identity, 'key-A'), null);
  scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A');
  assert.equal(scheduler.receiveResponse(request, job('render-A', 'key-A', 'complete', 'cache-A.mp4')), null);
  assert.equal(scheduler.needsRender(), true);
  const reopened = scheduler.beginRender();
  const cacheHit = scheduler.receiveResponse(reopened, job('cached-job', 'key-A', 'complete', 'cache-A.mp4'));
  assert.equal(scheduler.markReady(cacheHit), true);
});

test('failure/cancellation can be retried, duplicate requests are suppressed and wrong keys are rejected', () => {
  const { scheduler, request } = start(); assert.equal(scheduler.beginRender(), null);
  assert.equal(scheduler.receiveResponse(request, job('wrong', 'key-B')), null);
  scheduler.rejectRender(request); const retry = scheduler.beginRender();
  scheduler.receiveResponse(retry, job('retry', 'key-A'));
  assert.equal(scheduler.receiveEvent(job('retry', 'key-A', 'cancelled')).status, 'cancelled');
  assert.equal(scheduler.needsRender(), true);
  assert.ok(scheduler.beginRender());
});

test('terminal job state and progress never regress after late running snapshots', () => {
  const completed = job('render-A', 'key-A', 'complete', 'cache-A.mp4');
  assert.equal(mergePreviewJob(completed, job('render-A', 'key-A')).status, 'complete');
  const high = { ...job('render-A', 'key-A'), progress: .8 };
  assert.equal(mergePreviewJob(high, job('render-A', 'key-A')).progress, .8);
});

test('removing the last clip rejects pending replies and releases reuse of the old cache path', () => {
  const { scheduler, request } = start(); const empty = fixture(); empty.clips = [];
  scheduler.observe(descriptor(empty), false);
  assert.equal(scheduler.receiveResponse(request, job('old', 'key-A', 'complete', 'old.mp4')), null);
  assert.equal(scheduler.receiveEvent(job('old', 'key-A', 'complete', 'old.mp4')), null);
  scheduler.acceptIdentity(scheduler.beginIdentity(), 'empty-key'); assert.equal(scheduler.needsRender(), false);
  scheduler.observe(descriptor(fixture()), true); scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A');
  assert.equal(scheduler.needsRender(), true);
});

test('resolution/proxy changes reject old completions and reuse an already loaded identical key only after native validation', () => {
  const { scheduler, request } = start(); const ready = scheduler.receiveResponse(request, job('ready', 'key-A', 'complete', 'ready.mp4')); scheduler.markReady(ready);
  scheduler.observe(descriptor(fixture(), 360), true); const low = scheduler.beginIdentity();
  assert.equal(scheduler.receiveEvent(ready), null);
  scheduler.acceptIdentity(low, 'key-low'); const lowRequest = scheduler.beginRender();
  scheduler.observe(descriptor(fixture()), true); scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', 'ready.mp4');
  assert.equal(scheduler.needsRender(), false);
  assert.equal(scheduler.receiveResponse(lowRequest, job('low', 'key-low', 'complete', 'low.mp4')), null);
});

test('asset pin updates coalesce queued states and preserve serialization around an in-flight call', async () => {
  const writes = []; let unblock; const firstBlocked = new Promise(resolve => { unblock = resolve; });
  const queue = new PlaybackAssetQueue(async assets => { writes.push(assets); if (writes.length === 1) await firstBlocked; });
  const first = queue.update({ previewPath: 'A.mp4', sourcePath: 'source-A.mp4' }); await Promise.resolve();
  queue.update({ previewPath: 'B.mp4', sourcePath: 'source-B.mp4' });
  const final = queue.update({ previewPath: 'C.mp4', sourcePath: null });
  assert.deepEqual(writes, [{ previewPath: 'A.mp4', sourcePath: 'source-A.mp4' }]);
  unblock(); await first; await final;
  assert.deepEqual(writes, [{ previewPath: 'A.mp4', sourcePath: 'source-A.mp4' }, { previewPath: 'C.mp4', sourcePath: null }]);
});

test('a matching source key cannot reuse a deleted or replaced preview output without native validation', () => {
  const { scheduler, request } = start(); const ready = scheduler.receiveResponse(request, job('ready', 'key-A', 'complete', 'ready.mp4')); scheduler.markReady(ready);
  const invalidated = scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', null);
  assert.equal(invalidated.changed, false); assert.equal(invalidated.cacheInvalidated, true); assert.equal(invalidated.ready, null);
  assert.equal(scheduler.receiveEvent(ready), null); assert.equal(scheduler.needsRender(), true);
  const repair = scheduler.beginRender();
  const repaired = scheduler.receiveResponse(repair, job('repair', 'key-A', 'complete', 'ready.mp4')); assert.equal(scheduler.markReady(repaired), true);
  const validated = scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', 'ready.mp4');
  assert.equal(validated.cacheInvalidated, false); assert.equal(validated.ready.id, 'repair'); assert.equal(scheduler.needsRender(), false);
});

test('a native cache sample taken before completion rechecks the new output without invalidating it', () => {
  const { scheduler, request } = start(); scheduler.receiveResponse(request, job('render-A', 'key-A'));
  const identity = scheduler.beginIdentity();
  const ready = scheduler.receiveEvent(job('render-A', 'key-A', 'complete', 'cache-A.mp4')); scheduler.markReady(ready);
  const outdatedSample = scheduler.acceptIdentity(identity, 'key-A', null);
  assert.equal(outdatedSample.recheck, true); assert.equal(outdatedSample.cacheInvalidated, false);
  assert.equal(scheduler.needsRender(), false); assert.equal(scheduler.activeJobId, 'render-A');
  const fresh = scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', 'cache-A.mp4');
  assert.equal(fresh.recheck, false); assert.equal(fresh.ready.id, 'render-A');
  const laterDeletion = scheduler.acceptIdentity(scheduler.beginIdentity(), 'key-A', null);
  assert.equal(laterDeletion.recheck, false); assert.equal(laterDeletion.cacheInvalidated, true); assert.equal(scheduler.needsRender(), true);
});

test('an empty timeline can force a clearing RPC even before any preview path has loaded', async () => {
  const writes = [], queue = new PlaybackAssetQueue(async assets => { writes.push(assets); });
  await queue.update({ previewPath: null, sourcePath: null }, true);
  assert.deepEqual(writes, [{ previewPath: null, sourcePath: null }]);
});

test('stale source preparation can reassert unchanged current pins and empty project updates release both assets', async () => {
  const writes = []; const queue = new PlaybackAssetQueue(async assets => { writes.push(assets); });
  const current = { previewPath: 'program.mp4', sourcePath: 'selected.mp4' };
  await queue.update(current); await queue.update(current); assert.equal(writes.length, 1);
  // A stale prepare_source callback may have changed native pins without changing the UI.
  await queue.update(current, true); assert.equal(writes.length, 2);
  await queue.update({ previewPath: null, sourcePath: null });
  assert.deepEqual(writes.at(-1), { previewPath: null, sourcePath: null });
});

test('an obsolete pin error is suppressed and a later pin update still runs', async () => {
  const errors = [], writes = []; let failFirst;
  const blocked = new Promise((_, reject) => { failFirst = reject; });
  const queue = new PlaybackAssetQueue(async assets => { writes.push(assets); if (writes.length === 1) await blocked; }, error => errors.push(error));
  queue.update({ previewPath: 'obsolete.mp4', sourcePath: null }); await Promise.resolve();
  const latest = queue.update({ previewPath: 'current.mp4', sourcePath: null }); failFirst(new Error('Old asset disappeared'));
  await latest; assert.equal(errors.length, 0); assert.equal(writes.at(-1).previewPath, 'current.mp4');
});

test('source invalidations are ordered, while newer image/reset intents need not wait for media preparation', async () => {
  const actions = []; let releasePreparation;
  const slowPreparation = new Promise(resolve => { releasePreparation = resolve; });
  const queue = new SourceIntentQueue(async () => { actions.push('invalidate'); });
  await queue.invalidate(); actions.push('prepare video');
  const preparing = slowPreparation.then(() => actions.push('old video returned'));
  await queue.invalidate(); actions.push('show image');
  await queue.invalidate(); actions.push('reset source');
  assert.deepEqual(actions, ['invalidate', 'prepare video', 'invalidate', 'show image', 'invalidate', 'reset source']);
  releasePreparation(); await preparing; assert.equal(actions.at(-1), 'old video returned');
});

test('a failed source invalidation rejects that selection without stranding later source intents', async () => {
  let attempts = 0; const queue = new SourceIntentQueue(async () => { if (++attempts === 1) throw new Error('closed window'); });
  await assert.rejects(queue.invalidate(), /closed window/);
  await queue.invalidate(); assert.equal(attempts, 2);
});

const settings = { height: 540, useProxies: true };
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function bridgeFixture(overrides = {}) {
  const scheduler = new PreviewScheduler(), calls = []; let key = 'key-A', cachedPath = null;
  scheduler.observe('descriptor-A', true);
  const port = {
    identity: async options => { calls.push({ command: 'preview_identity', options }); return { key, cached_path: cachedPath }; },
    intent: async (options, expectedKey, revision) => { const intent = { session: 7, revision, key: expectedKey }; calls.push({ command: 'set_preview_intent', options, intent }); return intent; },
    render: async (options, expectedKey, intent) => { calls.push({ command: 'render_preview', options, expectedKey, intent }); return job(`render-${expectedKey}`, expectedKey, 'complete', `${expectedKey}.mp4`); },
    ...overrides,
  };
  const bridge = new PreviewBridge(scheduler, port);
  return { scheduler, bridge, calls, source: (nextKey, path = null) => { key = nextKey; cachedPath = path; } };
}

test('production bridge publishes native A intent before the Undo ready-cache shortcut and rejects late B work', async () => {
  const pendingB = deferred(); let bIntent;
  const state = bridgeFixture({ render: async (options, expectedKey, intent) => {
    state.calls.push({ command: 'render_preview', options, expectedKey, intent });
    if (expectedKey === 'key-B') { bIntent = intent; return pendingB.promise; }
    return job('ready-A', expectedKey, 'complete', 'A.mp4');
  } });
  await state.bridge.check(settings); const readyA = await state.bridge.render(settings); state.scheduler.markReady(readyA);
  state.scheduler.observe('descriptor-B', true); state.source('key-B'); await state.bridge.check(settings);
  const renderingB = state.bridge.render(settings);
  state.scheduler.observe('descriptor-A', true); state.source('key-A', 'A.mp4');
  const undo = await state.bridge.check(settings);
  assert.equal(undo.decision.ready.id, 'ready-A');
  assert.equal(state.calls.at(-1).command, 'set_preview_intent'); assert.equal(state.calls.at(-1).intent.key, 'key-A');
  assert.ok(undo.intent.revision > bIntent.revision); assert.equal(undo.intent.session, bIntent.session);
  // App applies the ready shortcut only after this production check resolves.
  assert.equal(state.scheduler.markReady(undo.decision.ready), true);
  pendingB.resolve(job('late-B', 'key-B')); assert.equal(await renderingB, null);
  assert.equal(state.scheduler.receiveEvent(job('late-B', 'key-B', 'complete', 'B.mp4')), null);
  assert.equal(state.scheduler.activeJobId, 'ready-A');
  assert.equal(state.calls.filter(call => call.command === 'render_preview' && call.expectedKey === 'key-A').length, 1);
});

test('production bridge cannot schedule B after its held intent reply is superseded by ready A', async () => {
  const heldB = deferred(); let bIntent;
  const state = bridgeFixture({ intent: async (options, expectedKey, revision) => {
    const intent = { session: 7, revision, key: expectedKey }; state.calls.push({ command: 'set_preview_intent', intent });
    if (expectedKey === 'key-B') { bIntent = intent; return heldB.promise; } return intent;
  } });
  await state.bridge.check(settings); const readyA = await state.bridge.render(settings); state.scheduler.markReady(readyA);
  state.scheduler.observe('descriptor-B', true); state.source('key-B'); const checkingB = state.bridge.check(settings);
  await Promise.resolve(); await Promise.resolve(); assert.equal(bIntent.key, 'key-B');
  state.scheduler.observe('descriptor-A', true); state.source('key-A', 'key-A.mp4');
  const undo = await state.bridge.check(settings); assert.equal(undo.decision.ready.id, readyA.id);
  heldB.reject('PREVIEW_INTENT_SUPERSEDED: a newer key is desired'); assert.equal(await checkingB, null);
  assert.equal(await state.bridge.render(settings), null);
  assert.equal(state.calls.some(call => call.command === 'render_preview' && call.expectedKey === 'key-B'), false);
});

test('production bridge binds render to accepted native token and preserves shared work across metadata revisions', async () => {
  const pending = deferred(); let renderIntent;
  const state = bridgeFixture({ render: async (options, expectedKey, intent) => {
    renderIntent = intent; state.calls.push({ command: 'render_preview', intent }); return pending.promise;
  } });
  const first = await state.bridge.check(settings); const rendering = state.bridge.render(settings);
  assert.deepEqual(renderIntent, first.intent);
  assert.equal(state.scheduler.observe('descriptor-A', true), false);
  const metadata = await state.bridge.check(settings); assert.ok(metadata.intent.revision > renderIntent.revision);
  pending.resolve(job('shared-A', 'key-A')); const accepted = await rendering;
  assert.equal(accepted.id, 'shared-A'); assert.equal(state.scheduler.activeJobId, 'shared-A');
  assert.equal(state.calls.filter(call => call.command === 'render_preview').length, 1);
  assert.equal(await state.bridge.render(settings), null);
});

test('production bridge merges a completion delivered before the native render reply', async () => {
  const state = bridgeFixture({ render: async () => {
    state.scheduler.receiveEvent(job('early', 'key-A', 'complete', 'A.mp4'));
    return job('early', 'key-A');
  } });
  await state.bridge.check(settings); const accepted = await state.bridge.render(settings);
  assert.equal(accepted.status, 'complete'); assert.equal(accepted.path, 'A.mp4');
});

test('production bridge skips stale identity replies and does not publish their native intent', async () => {
  const pendingIdentity = deferred(); let checks = 0;
  const state = bridgeFixture({ identity: async () => ++checks === 1 ? pendingIdentity.promise : { key: 'key-B', cached_path: null } });
  const old = state.bridge.check(settings); state.scheduler.observe('descriptor-B', true);
  const current = await state.bridge.check(settings); pendingIdentity.resolve({ key: 'key-A', cached_path: null });
  assert.equal(await old, null); assert.equal(current.intent.key, 'key-B');
  assert.deepEqual(state.calls.filter(call => call.command === 'set_preview_intent').map(call => call.intent.key), ['key-B']);
});

test('production bridge reports a wrong-key current render reply and releases the pending request for retry', async () => {
  const state = bridgeFixture({ render: async () => job('wrong', 'key-B') });
  await state.bridge.check(settings);
  await assert.rejects(state.bridge.render(settings), /different render identity/);
  assert.equal(state.scheduler.needsRender(), true);
});
