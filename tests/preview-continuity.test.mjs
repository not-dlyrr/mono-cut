import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { join } from 'node:path';
const require = createRequire(import.meta.url), build = process.env.MONO_CUT_PREVIEW_TEST_BUILD;
const { PreviewScheduler } = require(join(build, 'previewScheduler.js'));
const { PreviewBridge } = require(join(build, 'previewBridge.js'));
const { ProgramPreviewController } = require(join(build, 'programPreview.js'));
const { ProgramTransport } = require(join(build, 'programTransport.js'));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const flush = async () => { for (let n = 0; n < 30; n++) await Promise.resolve(); };

function fixture(overrides = {}, duration = 600, initialFrame = 0) {
  const scheduler = new PreviewScheduler(), calls = [], assets = [], candidates = [], errors = [], speeds = [], caches = new Map();
  let descriptor = 'A', stamp = 'source-1', serial = 0, hidden = 0, autoPin = true, armOnAsset = false;
  const key = region => `${descriptor}:${stamp}:${region.start_frame}:${region.end_frame}`;
  const job = (settings, identity, status = 'complete') => ({ id: `job-${++serial}`, kind: 'preview', status, progress: status === 'complete' ? 1 : .1, path: status === 'complete' ? `${identity}.mp4` : null, error: status === 'failed' ? 'encode failed' : null, preview_key: identity, preview_region: { ...settings.region } });
  const transport = new ProgramTransport(speed => speeds.push(speed));
  const port = {
    identity: async settings => { calls.push({ kind: 'identity', region: { ...settings.region } }); return { key: key(settings.region), program_key: `${descriptor}:${stamp}`, region: { ...settings.region }, cached_path: caches.get(key(settings.region)) ?? null }; },
    intent: async (settings, identity, revision) => { calls.push({ kind: 'intent', region: { ...settings.region }, revision }); return { key: identity, revision, session: 1 }; },
    cancel: async revision => { calls.push({ kind: 'cancel', revision }); },
    render: async (settings, identity, token) => { calls.push({ kind: 'render', region: { ...settings.region }, token }); const result = job(settings, identity); caches.set(identity, result.path); return result; },
    ...overrides,
  };
  const bridge = new PreviewBridge(scheduler, port);
  const controller = new ProgramPreviewController(scheduler, bridge, () => ({ height: 540, useProxies: true }), {
    asset: (result, region, playable) => { assets.push({ job: result, region, playable }); transport.resume(region, playable); if (armOnAsset) controller.setPlaybackSpeed(transport.currentSpeed); if (autoPin) controller.acknowledgeAsset(result.id); },
    successor: (result, region) => { if (result) candidates.push({ job: result, region }); },
    job: () => {}, unavailable: reason => { hidden++; transport.invalidate(reason === 'seek'); }, error: error => errors.push(error),
  }, { now: () => 0, setTimeout: callback => { queueMicrotask(callback); return 0; }, clearTimeout: () => {} });
  controller.observe(descriptor, { num: 30, den: 1 }, duration, initialFrame, true);
  const start = async () => { await controller.request(true); await flush(); };
  const play = speed => { transport.request(speed); controller.setPlaybackSpeed(speed); };
  const advance = frame => transport.advance(frame, (target, direction) => controller.advance(target, direction));
  const change = (value, frame = initialFrame, reset = false) => { descriptor = value; return controller.observe(value, { num: 30, den: 1 }, duration, frame, true, reset); };
  return { scheduler, controller, bridge, port, calls, assets, candidates, caches, errors, speeds, transport, job, key, start, play, advance, change,
    get hidden() { return hidden; }, set autoPin(value) { autoPin = value; }, set armOnAsset(value) { armOnAsset = value; }, set stamp(value) { stamp = value; } };
}

test('forward preparation requires both explicit playing intent and current playback pin acknowledgment', async () => {
  const state = fixture(); state.autoPin = false; await state.start();
  const current = state.assets.at(-1).job, requests = state.calls.filter(call => call.kind === 'render').length;
  state.controller.acknowledgeAsset('obsolete'); state.play(1); await flush();
  assert.equal(state.calls.filter(call => call.kind === 'render').length, requests);
  state.controller.acknowledgeAsset(current.id); await flush();
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
  assert.deepEqual(state.controller.successor.region, { start_frame: 150, end_frame: 300 });
  assert.equal(state.hidden, 1); assert.equal(state.transport.currentSpeed, 1);
  assert.equal(state.controller.successor.ready, false, 'file completion is not media readiness');
});

test('production ready-successor helpers cross 150/300/450 without forced zero, hidden footage, or overlap', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush();
  for (const boundary of [150, 300, 450]) {
    const successor = state.controller.successor;
    assert.equal(successor.region.start_frame, boundary); assert.equal(successor.region.end_frame, boundary + 150);
    assert.equal(state.controller.successorReady(successor.job.id), true);
    assert.equal(state.advance(boundary), true); await flush();
    assert.deepEqual(state.controller.coverage, successor.region); assert.equal(state.transport.currentSpeed, 1);
  }
  assert.deepEqual(state.speeds.slice(1), [1]); assert.equal(state.hidden, 1);
  assert.deepEqual(state.assets.filter(asset => asset.playable).map(asset => asset.region), [
    { start_frame: 0, end_frame: 150 }, { start_frame: 150, end_frame: 300 }, { start_frame: 300, end_frame: 450 }, { start_frame: 450, end_frame: 600 },
  ]);
  assert.equal(state.controller.boundary(), null); assert.equal(state.controller.successor, null);
});

test('a file-ready but unloaded successor waits only at its actual boundary, then resumes its signed intent', async () => {
  const state = fixture(); await state.start(); state.play(2); await flush();
  const successor = state.controller.successor;
  assert.equal(state.transport.currentSpeed, 2); assert.equal(state.hidden, 1);
  state.advance(150); assert.equal(state.transport.currentSpeed, 0); assert.equal(state.controller.coverage, null);
  assert.deepEqual(state.controller.pendingRegion, { start_frame: 150, end_frame: 300 });
  assert.equal(state.controller.successorReady(successor.job.id), true); await flush();
  assert.equal(state.transport.currentSpeed, 2); assert.deepEqual(state.controller.coverage, successor.region);
  assert.equal(state.hidden, 2, 'one actual unavailable boundary, not an early placeholder');
});

test('a successor never starts a third preparation before handoff playback pin acknowledgment', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush();
  const successor = state.controller.successor; state.controller.successorReady(successor.job.id);
  state.autoPin = false; const renders = state.calls.filter(call => call.kind === 'render').length;
  state.advance(150); await flush();
  assert.equal(state.calls.filter(call => call.kind === 'render').length, renders);
  state.controller.acknowledgeAsset(state.assets[1].job.id); await flush();
  assert.equal(state.calls.filter(call => call.kind === 'render').length, renders, 'retired asset acknowledgment cannot release ownership');
  state.controller.acknowledgeAsset(successor.job.id); await flush();
  assert.equal(state.calls.filter(call => call.kind === 'render').length, renders + 1);
  assert.deepEqual(state.controller.successor.region, { start_frame: 300, end_frame: 450 });
});

test('failed successor render preserves current content and explicit retry admits the matching successor', async () => {
  let fail = true; const state = fixture({ render: async (settings, identity) => {
    const result = state.job(settings, identity, settings.region.start_frame === 150 && fail ? 'failed' : 'complete');
    if (result.path) state.caches.set(identity, result.path); return result;
  } });
  await state.start(); const current = state.controller.coverage; state.play(1); await flush();
  assert.equal(state.errors[0], 'encode failed'); assert.deepEqual(state.controller.coverage, current);
  assert.equal(state.hidden, 1); assert.equal(state.controller.successor, null);
  fail = false; await state.controller.request(true); await flush();
  const next = state.controller.successor; state.controller.successorReady(next.job.id); state.advance(150);
  assert.equal(state.transport.currentSpeed, 1); assert.deepEqual(state.controller.coverage, next.region);
});

test('successor inspection failure retains valid playing coverage, waits at the boundary and retries only its slot', async () => {
  let fail = true; const state = fixture({ identity: async settings => {
    if (settings.region.start_frame === 150 && fail) throw new Error('inspection failed');
    const identity = state.key(settings.region); return { key: identity, program_key: 'A:source-1', region: { ...settings.region }, cached_path: state.caches.get(identity) ?? null };
  } });
  await state.start(); state.play(1); await flush();
  assert.equal(state.errors[0].message, 'inspection failed'); assert.equal(state.hidden, 1); assert.equal(state.transport.currentSpeed, 1);
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 }); assert.ok(state.calls.some(call => call.kind === 'cancel'));
  state.advance(150); assert.equal(state.transport.currentSpeed, 0); assert.equal(state.hidden, 2);
  fail = false; await state.controller.request(true); await flush(); const next = state.controller.successor;
  assert.deepEqual(next.region, { start_frame: 150, end_frame: 300 }); state.controller.successorReady(next.job.id); await flush();
  assert.equal(state.transport.currentSpeed, 1); assert.deepEqual(state.controller.coverage, next.region);
});

test('successor media-load failure preserves current ownership and permits a new candidate retry', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const first = state.controller.successor;
  state.controller.successorFailed(first.job.id, 'media load failed');
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 }); assert.equal(state.hidden, 1);
  assert.equal(state.controller.successorReady(first.job.id), false);
  await state.controller.request(true); await flush(); const retry = state.controller.successor;
  assert.notEqual(retry.job.id, first.job.id); assert.deepEqual(retry.region, first.region);
  state.controller.successorReady(retry.job.id); state.advance(150); assert.equal(state.transport.currentSpeed, 1);
});

test('Stop cancels successor admission and late file/media replies cannot hide or restart current content', async () => {
  const held = deferred(); let old;
  const state = fixture({ render: async (settings, identity) => {
    const result = state.job(settings, identity); if (settings.region.start_frame === 150) { old = result; return held.promise; }
    state.caches.set(identity, result.path); return result;
  } });
  await state.start(); state.play(1); await flush(); state.play(0); await flush();
  assert.ok(state.calls.some(call => call.kind === 'cancel'));
  const assets = state.assets.length, candidates = state.candidates.length;
  held.resolve(old); await flush(); state.controller.receive(old); state.controller.successorReady(old.id);
  assert.equal(state.assets.length, assets); assert.equal(state.candidates.length, candidates);
  assert.equal(state.transport.currentSpeed, 0); assert.equal(state.hidden, 1);
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
});

test('Stop at a delayed boundary cancels signed resume even if its former successor later loads', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.advance(150); assert.equal(state.transport.currentSpeed, 0); state.play(0);
  assert.equal(state.controller.successorReady(old.job.id), false); state.controller.receive(old.job); await flush();
  assert.equal(state.transport.currentSpeed, 0); assert.equal(state.controller.successor, null);
  assert.deepEqual(state.controller.pendingRegion, { start_frame: 150, end_frame: 151 });
});

test('manual seeks supersede prepared coverage even when the new target is inside the current asset', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.transport.cancelResume(); assert.equal(state.controller.seek(50), false); await flush();
  assert.equal(state.controller.successor, null); assert.equal(state.controller.successorReady(old.job.id), false);
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 }); assert.equal(state.hidden, 1);
  assert.ok(state.calls.some(call => call.kind === 'cancel'));
  assert.equal(state.controller.seek(410), true); await state.controller.request(true); await flush();
  assert.deepEqual(state.controller.coverage, { start_frame: 410, end_frame: 560 });
});

test('a manual seek inside a delayed successor starts its own still instead of waiting on cancelled work', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.advance(150); state.transport.cancelResume(); assert.equal(state.controller.seek(201), true);
  assert.deepEqual(state.controller.pendingRegion, { start_frame: 201, end_frame: 202 });
  assert.equal(state.controller.successorReady(old.job.id), false); await state.controller.request(true); await flush();
  assert.deepEqual(state.controller.coverage, { start_frame: 201, end_frame: 351 }); assert.equal(state.transport.currentSpeed, 0);
});

test('metadata Save preserves current plus successor work while model/settings/project changes reject old candidates', async () => {
  for (const replacement of ['edit', 'settings', 'project']) {
    const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
    const renders = state.calls.filter(call => call.kind === 'render').length, hidden = state.hidden;
    assert.equal(state.change('A', 50), false); await state.controller.request(); await flush();
    assert.equal(state.calls.filter(call => call.kind === 'render').length, renders); assert.equal(state.hidden, hidden);
    assert.equal(state.controller.successor.job.id, old.job.id);
    state.change(`B-${replacement}`, 350, replacement === 'project');
    assert.equal(state.controller.successorReady(old.job.id), false); state.controller.receive(old.job);
    await state.controller.request(true); await flush();
    assert.deepEqual(state.controller.coverage, { start_frame: 350, end_frame: 500 }); assert.equal(state.transport.currentSpeed, 0);
  }
});

test('changed physical-source identity during successor validation clears the old current asset', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.stamp = 'source-2'; await state.controller.request(true); await flush();
  assert.equal(state.controller.successorReady(old.job.id), false); assert.equal(state.hidden, 2);
  assert.ok(state.assets.at(-1).job.preview_key.includes('source-2')); state.controller.receive(old.job);
  assert.ok(state.assets.at(-1).job.preview_key.includes('source-2'));
});

test('deleted successor cache drops only that candidate and retains currently consumed footage', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.caches.delete(old.job.preview_key); await state.controller.request(true); await flush();
  assert.equal(state.hidden, 1); assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
  assert.notEqual(state.controller.successor.job.id, old.job.id); assert.equal(state.controller.successorReady(old.job.id), false);
});

test('owned playback ticks recover a changed source at the current global frame without eager work', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush();
  const renders = state.calls.filter(call => call.kind === 'render').length;
  state.controller.trackFrame(103); await flush(); assert.equal(state.calls.filter(call => call.kind === 'render').length, renders);
  state.stamp = 'source-2'; await state.controller.request(true); await flush();
  assert.equal(state.assets.at(-2).region.start_frame, 103); assert.equal(state.assets.at(-2).playable, false);
  assert.deepEqual(state.controller.coverage, { start_frame: 103, end_frame: 253 });
});

test('sequence tail successors clamp only their end and never repeat earlier frames', async () => {
  const state = fixture({}, 510); await state.start(); state.play(1); await flush();
  for (const boundary of [150, 300, 450]) { const next = state.controller.successor; state.controller.successorReady(next.job.id); state.advance(boundary); await flush(); }
  assert.deepEqual(state.controller.coverage, { start_frame: 450, end_frame: 510 }); assert.equal(state.controller.boundary(), null);
  assert.deepEqual(state.candidates.map(item => item.region), [{ start_frame: 150, end_frame: 300 }, { start_frame: 300, end_frame: 450 }, { start_frame: 450, end_frame: 510 }]);
});

test('a held successor identity is superseded by cancellation before any stale native intent publication', async () => {
  const held = deferred(); let hold = true;
  const state = fixture({ identity: async settings => {
    const region = { ...settings.region }, identity = state.key(region); if (region.start_frame === 150 && hold) await held.promise;
    return { key: identity, program_key: 'A:source-1', region, cached_path: state.caches.get(identity) ?? null };
  } });
  await state.start(); state.play(1); await flush(); state.play(0); await flush(); hold = false;
  held.resolve(); await flush();
  assert.equal(state.calls.some(call => call.kind === 'intent' && call.region.start_frame === 150), false);
  const cutoff = state.calls.find(call => call.kind === 'cancel').revision;
  state.play(1); await flush(); const newest = state.calls.filter(call => call.kind === 'intent').at(-1);
  assert.ok(newest.revision > cutoff); assert.deepEqual(newest.region, { start_frame: 150, end_frame: 300 });
});

test('obsolete successor acknowledgments and duplicate job events cannot consume the replacement slot', async () => {
  const state = fixture(); await state.start(); state.play(1); await flush(); const old = state.controller.successor;
  state.controller.successorReady(old.job.id); state.advance(150); await flush(); const next = state.controller.successor;
  const renders = state.calls.filter(call => call.kind === 'render').length, assets = state.assets.length;
  state.controller.receive(old.job); state.controller.successorReady(old.job.id); state.controller.successorFailed(old.job.id, 'old load error'); state.controller.acknowledgeAsset(state.assets[1].job.id);
  await flush(); assert.equal(state.controller.successor.job.id, next.job.id); assert.equal(state.controller.successor.ready, false);
  assert.equal(state.assets.length, assets); assert.equal(state.calls.filter(call => call.kind === 'render').length, renders); assert.equal(state.errors.length, 0);
});

test('late stale Stop cancellation cannot report an error against a newer published Play intent', async () => {
  const held = deferred(), state = fixture({ cancel: () => held.promise });
  await state.start(); state.play(1); await flush(); state.play(0); state.play(1); await flush();
  held.reject(new Error('obsolete cancellation revision')); await flush();
  assert.equal(state.errors.length, 0); assert.equal(state.transport.currentSpeed, 1); assert.ok(state.controller.successor);
});

test('a current Stop cancellation failure is reported while valid current footage remains stopped', async () => {
  const state = fixture({ cancel: async () => { throw new Error('cancel failed'); } });
  await state.start(); state.play(1); await flush(); state.play(0); await flush();
  assert.equal(state.errors[0].message, 'cancel failed'); assert.equal(state.transport.currentSpeed, 0);
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
});

test('an older native Stop cancellation cannot fail a newer Stop whose native reply already succeeded', async () => {
  const cancellations = [], state = fixture({ cancel: revision => { const held = deferred(); cancellations.push({ revision, ...held }); return held.promise; } });
  await state.start(); state.play(1); await flush(); state.transport.request(0); state.controller.stopPlayback(); state.controller.stopPlayback(); await flush();
  assert.equal(cancellations.length, 2); assert.ok(cancellations[1].revision > cancellations[0].revision);
  cancellations[1].resolve(); await flush(); cancellations[0].reject(new Error('PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this cancellation.')); await flush();
  assert.equal(state.errors.length, 0); assert.equal(state.transport.currentSpeed, 0); assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
});

test('a stale cancellation cannot fail newer identity inspection before that inspection has published its native intent', async () => {
  const cancellation = deferred(), identity = deferred(); let hold = false;
  const state = fixture({ cancel: () => cancellation.promise, identity: async settings => {
    const key = state.key(settings.region); if (hold && settings.region.start_frame === 150) await identity.promise;
    return { key, program_key: 'A:source-1', region: settings.region, cached_path: state.caches.get(key) ?? null };
  } });
  await state.start(); state.play(1); await flush(); state.transport.request(0); state.controller.stopPlayback(); hold = true; state.play(1); await flush();
  cancellation.reject(new Error('PREVIEW_IDENTITY_CHANGED: Newer native operation won.')); await flush();
  assert.equal(state.errors.length, 0); identity.resolve(); await flush(); assert.ok(state.controller.successor); assert.equal(state.transport.currentSpeed, 1);
});

test('close cancellation and disposal publish ordered cancellation before held initial admission can revive', async () => {
  for (const action of ['cancelPreparation', 'dispose']) {
    const held = deferred(); let old;
    const state = fixture({ render: async (settings, identity) => { old = state.job(settings, identity); return held.promise; } });
    const request = state.controller.request(true); await flush(); state.controller[action](); await flush();
    assert.ok(state.calls.some(call => call.kind === 'cancel'), action);
    held.resolve(old); await request; await flush(); state.controller.receive(old);
    assert.equal(state.assets.length, 0, action); assert.equal(state.controller.successor, null);
  }
});

test('explicit Stop rejects held first-frame admission while later Play can prepare a fresh current slot', async () => {
  const held = deferred(); let old, holding = true;
  const state = fixture({ render: async (settings, identity) => {
    const result = state.job(settings, identity); if (holding) { old = result; return held.promise; }
    state.caches.set(identity, result.path); return result;
  } });
  const request = state.controller.request(true); await flush(); state.transport.request(0); state.controller.stopPlayback(); await flush();
  assert.ok(state.calls.some(call => call.kind === 'cancel')); held.resolve(old); await request; await flush(); state.controller.receive(old);
  assert.equal(state.assets.length, 0); holding = false; state.play(1); await flush();
  assert.equal(state.controller.playable, true); assert.equal(state.transport.currentSpeed, 1);
  assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
});

test('explicit Stop cancels held initial continuous admission, retains its correct still and permits later Play', async () => {
  const held = deferred(); let old, holding = true;
  const state = fixture({ render: async (settings, identity) => {
    const result = state.job(settings, identity);
    if (holding && settings.region.end_frame - settings.region.start_frame > 1) { old = result; return held.promise; }
    state.caches.set(identity, result.path); return result;
  } });
  await state.start(); assert.equal(state.controller.playable, false); const still = state.assets[0].job;
  state.transport.request(0); state.controller.stopPlayback(); await flush(); held.resolve(old); await flush(); state.controller.receive(old);
  assert.equal(state.assets.length, 1); assert.equal(state.assets[0].job.id, still.id); assert.equal(state.transport.currentSpeed, 0);
  holding = false; state.play(1); await flush();
  assert.equal(state.controller.playable, true); assert.deepEqual(state.controller.coverage, { start_frame: 0, end_frame: 150 });
});

test('the App-style playing asset callback does not re-inspect the still before its continuous preparation', async () => {
  const state = fixture(); state.armOnAsset = true; state.play(1); await flush();
  assert.deepEqual(state.calls.filter(call => call.kind === 'identity').map(call => call.region), [
    { start_frame: 0, end_frame: 1 }, { start_frame: 0, end_frame: 150 }, { start_frame: 150, end_frame: 300 },
  ]);
  assert.equal(state.controller.playable, true);
});
