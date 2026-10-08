// Silent real-time measurement of the production controller/transport and native jobs.
// A decoded, inspected file is a file-readiness endpoint, not an observed video element.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { cpus, platform, release, totalmem } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const options = new Map();
for (let n = 2; n < process.argv.length; n += 2) {
  if (!process.argv[n]?.startsWith('--') || !process.argv[n + 1]) throw new Error('Use --name value arguments.');
  options.set(process.argv[n].slice(2), process.argv[n + 1]);
}
const required = key => { const value = options.get(key); if (!value) throw new Error(`Missing --${key}.`); return value; };
const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const projectPath = resolve(required('project')), output = resolve(required('output'));
const mode = options.get('mode') ?? 'originals';
if (!['originals', 'proxies'].includes(mode)) throw new Error('Mode must be originals or proxies.');
const useProxies = mode === 'proxies';
const driverPath = resolve(options.get('driver') ?? join(root, 'src-tauri/target/debug/examples', process.platform === 'win32' ? 'preview_region_driver.exe' : 'preview_region_driver'));
const resources = join(root, 'src-tauri/resources/media');
const ffmpeg = resolve(options.get('ffmpeg') ?? process.env.MONO_CUT_FFMPEG ?? join(resources, process.platform === 'win32' ? 'ffmpeg.exe' : 'ffmpeg'));
const ffprobe = resolve(options.get('ffprobe') ?? process.env.MONO_CUT_FFPROBE ?? join(resources, process.platform === 'win32' ? 'ffprobe.exe' : 'ffprobe'));
const font = resolve(options.get('font') ?? process.env.MONO_CUT_FONT ?? join(resources, 'Inter.ttf'));
mkdirSync(output, { recursive: true });
const cachePath = join(output, 'cache');
if (existsSync(cachePath) && readdirSync(cachePath).length) throw new Error('Use a fresh empty output cache; existing prepared assets cannot prove new preparation.');
const helperOutput = join(output, `production-helpers-${randomUUID()}`); mkdirSync(helperOutput);
const helperInputs = ['src/programPreview.ts', 'src/programTransport.ts', 'src/previewModel.ts', 'src/previewAssets.ts', 'src/previewBridge.ts', 'src/previewScheduler.ts', 'src/previewRegion.ts', 'src/types.ts'];
const sourceNames = [...helperInputs, 'src/App.tsx', 'src/Monitor.tsx', 'src/monitorLifecycle.ts', 'src/programMediaSlots.ts', 'src/ProgramMedia.tsx', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/src/render.rs', 'src-tauri/src/jobs.rs', 'src-tauri/src/preview.rs', 'src-tauri/src/audio_clock.rs', 'src-tauri/src/video_seek.rs', 'src-tauri/src/model.rs', 'src-tauri/src/edit.rs', 'src-tauri/src/storage.rs', 'src-tauri/src/media.rs', 'src-tauri/src/processes.rs', 'src-tauri/src/lib.rs', 'src-tauri/tests/support/stage4b.rs', 'src-tauri/tests/continuity_reference.rs', 'src-tauri/examples/preview_region_driver.rs', 'scripts/measure-preview-continuity.mjs'];
const sourceHashes = () => Object.fromEntries(sourceNames.map(name => [name, hash(join(root, name))]));
const toolHashes = () => ({ driver_sha256: hash(driverPath), ffmpeg_sha256: hash(ffmpeg), ffprobe_sha256: hash(ffprobe), font_sha256: hash(font) });
const fixtureHashes = project => project.media.map(m => ({ id: m.id, original_sha256: hash(m.path), proxy_sha256: m.proxy ? hash(m.proxy) : null, timing: m.timing, proxy_timing_version: m.proxy_timing_version }));
const report = {
  stage: '4C', mode, started_utc: new Date().toISOString(), project: projectPath, fixture_sha256: hash(projectPath),
  sources: sourceHashes(), tools: toolHashes(),
  environment: { cpu: cpus()[0]?.model, logical_processors: cpus().length, physical_memory_bytes: totalmem(), platform: platform(), os_release: release(), node: process.version },
  settings: { height: 540, width: 960, useProxies, run_frames: 600, boundary_frames: [150, 300, 450] },
  cache_state: 'Application preview/audio cache starts empty. OS file cache is not cleared. Prepared source proxies are inputs, never pre-existing ready program regions.',
  endpoints: {
    playback: 'Real monotonic 20-second global-frame drive through compiled production controller/transport and native ProjectStore/JobManager; freshly produced successor files inspected and silently decoded before explicit file-readiness admission.',
    readiness: 'Exact file metadata, encoded profile and first/last silently decoded RGB frames. This substitutes an explicit file-ready acknowledgement in the helper; it is not a measured media-element canplay event.',
    native_display_or_audio: 'Not observed. No native app, browser, player, installer or audio output is launched. Speaker continuity, media-element handoff and native display latency remain unverified.',
  },
  scenarios: [], traces: [], rpc: [], jobs: [], telemetry: [], tools_intervals: [], failures: [],
};
const compilation = spawnSync(process.execPath, [join(root, 'node_modules/typescript/bin/tsc'), '--target', 'ES2022', '--module', 'commonjs', '--moduleResolution', 'node', '--skipLibCheck', '--strict', '--outDir', helperOutput, '--rootDir', join(root, 'src'), ...['programPreview.ts', 'programTransport.ts', 'previewModel.ts', 'previewAssets.ts'].map(n => join(root, 'src', n))], { cwd: root, encoding: 'utf8', windowsHide: true });
if (compilation.status !== 0) throw new Error(`Production helper compilation failed:\n${compilation.stdout}\n${compilation.stderr}`);
report.compiled_helpers = Object.fromEntries(readdirSync(helperOutput).filter(n => n.endsWith('.js')).map(n => [n, hash(join(helperOutput, n))]));
const require = createRequire(import.meta.url);
const { ProgramPreviewController } = require(join(helperOutput, 'programPreview.js'));
const { ProgramTransport } = require(join(helperOutput, 'programTransport.js'));
const { PreviewBridge } = require(join(helperOutput, 'previewBridge.js'));
const { PreviewScheduler } = require(join(helperOutput, 'previewScheduler.js'));
const { PlaybackAssetQueue } = require(join(helperOutput, 'previewAssets.js'));
const { previewModelDescriptor } = require(join(helperOutput, 'previewModel.js'));
const epoch = performance.now(), at = () => performance.now() - epoch;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const driver = spawn(driverPath, [projectPath, cachePath, font], { cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, MONO_CUT_FFMPEG: ffmpeg, MONO_CUT_FFPROBE: ffprobe, MONO_CUT_MEASURE_PARENT_PID: String(process.pid) } });
let serial = 0, closed = false, shuttingDown = false, controllerDisposed = false, startup = null, controller = null, scenario = null, currentAsset = null, candidateAsset = null, callbackGeneration = 0, transport;
const pending = new Map(), fileTools = new Set(), tasks = new Set(), diagnostics = [];
function trace(kind, values = {}) {
  const record = { at_ms: at(), scenario: scenario?.label ?? null, kind, ...values };
  report.traces.push(record); return record;
}
function fail(error) { const message = error instanceof Error ? error.message : String(error); report.failures.push({ at_ms: at(), scenario: scenario?.label ?? null, message }); }
function rpc(op, payload = {}) {
  if (closed || shuttingDown) return Promise.reject(new Error('Native measurement driver is closing/closed.'));
  if (op === 'shutdown') shuttingDown = true;
  const id = ++serial, began = at();
  return new Promise((accept, reject) => {
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`Driver ${op} exceeded120seconds.`)); }, 120_000);
    pending.set(id, { op, began, accept, reject, timeout });
    report.rpc.push({ id, kind: 'request', op, at_ms: began, pending_rpc_count: pending.size, region: payload.region ?? null, expected_key: payload.expectedKey ?? payload.intent?.key ?? null, session: payload.session ?? payload.intent?.session ?? null, revision: payload.revision ?? payload.intent?.revision ?? null, command: payload.command ?? null });
    driver.stdin.write(`${JSON.stringify({ id, op, ...payload })}\n`);
  });
}
driver.stderr.on('data', data => diagnostics.push(data.toString()));
driver.stdin.on('error', error => {
  for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(error); }
  pending.clear(); if (!shuttingDown) fail(error);
});
createInterface({ input: driver.stdout }).on('line', line => {
  let value; try { value = JSON.parse(line); } catch { diagnostics.push(line); return; }
  if (value.event === 'ready') { startup = value; return; }
  if (value.event === 'job') { report.jobs.push({ at_ms: at(), scenario: scenario?.label ?? null, job: value.job }); controller?.receive(value.job); return; }
  const request = pending.get(value.id); if (!request) return;
  pending.delete(value.id); clearTimeout(request.timeout);
  report.rpc.push({ id: value.id, kind: 'reply', op: request.op, at_ms: at(), round_trip_ms: at() - request.began, backend_rpc_ms: Number(value.backend_rpc_seconds ?? 0) * 1000, error: value.error ?? null });
  if (value.error) request.reject(new Error(value.error)); else request.accept(value.result);
});
driver.on('error', error => { fail(error); for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(error); } pending.clear(); });
driver.on('exit', code => { closed = true; for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(new Error(`Driver exited${code}.`)); } pending.clear(); if (code) fail(new Error(`Driver exited${code}.`)); });
function work(action) { const task = Promise.resolve().then(action); tasks.add(task); void task.catch(fail).finally(() => tasks.delete(task)); return task; }
function runFileTool(tool, args, maximum = 8 * 1024 * 1024) {
  return new Promise((accept, reject) => {
    const child = spawn(tool, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] }); fileTools.add(child);
    const record = { kind: tool === ffmpeg ? 'silent-video-decode' : 'metadata-probe', at_ms: at(), pid: child.pid, args, active_file_tools: fileTools.size }; report.tools_intervals.push(record);
    const chunks = []; let bytes = 0, stderr = '', settled = false;
    const settle = (error, result) => { if (settled) return; settled = true; error ? reject(error) : accept(result); };
    const timeout = setTimeout(() => { child.kill(); settle(new Error('Silent file tool exceeded10seconds.')); }, 10_000);
    child.stdout.on('data', data => { bytes += data.length; if (bytes > maximum) { child.kill(); settle(new Error('Silent file tool exceeded its output cap.')); } else chunks.push(data); });
    child.stderr.on('data', data => { if (stderr.length < 32768) stderr += data.toString().slice(0, 32768 - stderr.length); });
    child.on('error', error => { clearTimeout(timeout); fileTools.delete(child); settle(error); });
    child.on('close', code => { clearTimeout(timeout); fileTools.delete(child); Object.assign(record, { finished_ms: at(), bytes, exit_code: code }); settle(code === 0 ? null : new Error(`Silent file tool failed${code}:${stderr}`), Buffer.concat(chunks)); });
  });
}
function encodedProfile(path) {
  const bytes = readFileSync(path), prefix = Buffer.from('options: '), start = bytes.indexOf(prefix), end = bytes.indexOf(0, start + prefix.length);
  if (start < 0 || end < 0 || end - start > 4096) throw new Error('Actual native preview lacks a boundedx264 encoder-optionsSEI.');
  const values = Object.fromEntries(bytes.subarray(start + prefix.length, end).toString('ascii').split(/\s+/).map(v => v.split('=')));
  const profile = { rate_control: values.rc, video_crf: Number(values.crf), gop_frames: Number(values.keyint), effective_min_keyint: Number(values.keyint_min), scenecut: Number(values.scenecut) };
  if (profile.rate_control !== 'crf' || profile.video_crf !== 20 || profile.gop_frames !== 15 || profile.scenecut !== 0) throw new Error(`Actual preview profile changed:${JSON.stringify(profile)}`);
  return profile;
}
function cacheFiles(directory = cachePath, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name), name = prefix + entry.name;
    return entry.isDirectory() ? cacheFiles(path, `${name}/`) : [{ name, bytes: statSync(path).size }];
  });
}
function cachePartials() { return cacheFiles().filter(file => /\.mono-(?:audio-)?part-|-(?:thumb|source)-part-|\.json\.tmp$/.test(file.name)); }
async function validateFile(job, region, playable) {
  const began = at(), bytes = readFileSync(job.path), fileSha = createHash('sha256').update(bytes).digest('hex');
  const metadata = JSON.parse((await runFileTool(ffprobe, ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', job.path])).toString());
  const video = metadata.streams?.find(s => s.codec_type === 'video'), count = region.end_frame - region.start_frame;
  if (Number(video?.nb_frames) !== count || video?.width !== 960 || video?.height !== 540 || video?.avg_frame_rate !== '30/1') throw new Error('Actual file cadence/coverage/dimensions differ from requested coverage.');
  if (!metadata.streams?.some(s => s.codec_type === 'audio' && Number(s.sample_rate) === 48000)) throw new Error('Actual file lacks48000Hz synchronized audio.');
  if (Math.abs(Number(video.duration) - count / 30) > 1 / 60) throw new Error('Actual file duration differs from declared half-open coverage.');
  const decoded = await runFileTool(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-nostdin', '-threads', '1', '-i', job.path, '-map', '0:v:0', '-an', '-vf', count === 1 ? 'select=eq(n\\,0)' : `select=eq(n\\,0)+eq(n\\,${count - 1})`, '-fps_mode', 'passthrough', '-frames:v', count === 1 ? '1' : '2', '-threads', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
  const expected = 960 * 540 * 3 * (count === 1 ? 1 : 2);
  if (decoded.length !== expected) throw new Error('Actual file omitted a first/last requested picture.');
  const filename = `${job.id}-${region.start_frame}-${region.end_frame}.mp4`, retained = join(output, 'assets', filename); mkdirSync(dirname(retained), { recursive: true }); copyFileSync(job.path, retained);
  const record = { job_id: job.id, key: job.preview_key, region: { ...region }, local_origin_frame: region.start_frame, local_origin_seconds: { num: region.start_frame, den: 30 }, path: job.path, retained_path: retained, file_sha256: fileSha, bytes: bytes.length, inspection_started_ms: began, file_ready_ms: at(), file_inspection_ms: at() - began, playable, video_frames: count, metadata, encoded_profile: encodedProfile(job.path), first_last_rgb_sha256: createHash('sha256').update(decoded).digest('hex') };
  trace('file-validated', { job_id: job.id, region, playable, file_sha256: fileSha, file_ready_ms: record.file_ready_ms }); return record;
}
const scheduler = new PreviewScheduler(); let session = null;
const bridge = new PreviewBridge(scheduler, {
  identity: settings => rpc('identity', settings),
  intent: (settings, expectedKey, revision) => rpc('intent', { ...settings, expectedKey, session, revision }),
  render: (settings, expectedKey, intent) => rpc('render', { ...settings, expectedKey, intent }),
  cancel: revision => rpc('cancel_intent', { session, revision }),
});
const assets = new PlaybackAssetQueue(values => rpc('assets', values), fail);
transport = new ProgramTransport(speed => trace('speed', { speed }));
// Boundary waiting changes transport speed temporarily; only a user's signed
// transport request may cancel the controller's useful preparation/resume.
function requestSpeed(speed) { transport.request(speed); if (!transport.currentSpeed) controller?.stopPlayback(); else controller?.setPlaybackSpeed(transport.currentSpeed); }
function pinAssets() { return assets.update({ previewPath: currentAsset?.job.path ?? null, sourcePath: null, successorPreviewPath: candidateAsset?.job.path ?? null }); }
controller = new ProgramPreviewController(scheduler, bridge, () => ({ height: 540, useProxies }), {
  unavailable: reason => { callbackGeneration++; currentAsset = null; trace('unavailable', { reason }); transport.invalidate(reason === 'seek'); void work(pinAssets); },
  job: job => trace('job-response', { job_id: job.id, status: job.status, region: job.preview_region }),
  error: fail,
  timing: event => trace('controller-timing', { ...event, at_ms: event.at - epoch }),
  asset: (job, region, playable) => {
    const generation = callbackGeneration, owner = scenario, old = currentAsset;
    currentAsset = { job, region, playable, validated: candidateAsset?.job.id === job.id ? candidateAsset.validated : null };
    if (candidateAsset?.job.id === job.id) candidateAsset = null;
    trace('asset', { job_id: job.id, region, playable, previous_region: old?.region ?? null });
    if (owner) owner.assets.push({ job_id: job.id, region: { ...region }, playable, offered_ms: at() });
    const shown = currentAsset;
    void work(async () => {
      const pinned = await pinAssets();
      if (!pinned || generation !== callbackGeneration || currentAsset !== shown) return;
      if (!shown.validated) shown.validated = await validateFile(job, region, playable);
      if (generation !== callbackGeneration || currentAsset !== shown) return;
      owner?.files.push(shown.validated);
      controller.acknowledgeAsset(job.id); trace('asset-acknowledged', { job_id: job.id, region });
      transport.resume(region, playable);
      if (transport.currentSpeed) controller.setPlaybackSpeed(transport.currentSpeed);
    });
  },
  successor: (job, region) => {
    const owner = scenario;
    if (!job || !region) { candidateAsset = null; trace('successor-cleared'); void work(pinAssets); return; }
    const candidate = { job, region: { ...region }, validated: null }; candidateAsset = candidate;
    trace('successor-offered', { job_id: job.id, region });
    void work(async () => {
      const pinned = await pinAssets();
      if (!pinned || candidateAsset !== candidate || controller.successor?.job.id !== job.id) return;
      candidate.validated = await validateFile(job, region, true);
      if (candidateAsset !== candidate || controller.successor?.job.id !== job.id) return;
      owner?.files.push(candidate.validated);
      controller.successorReady(job.id); trace('successor-admitted', { job_id: job.id, region });
    });
  },
});
function disposeController() { if (!controllerDisposed) { controllerDisposed = true; controller?.dispose(); } }
async function waitFor(predicate, label, timeout = 30_000) {
  const began = at(); while (!predicate()) { if (report.failures.length) throw new Error(report.failures.at(-1).message); if (at() - began > timeout) throw new Error(`${label} exceeded${timeout}ms.`); await sleep(10); }
}
let telemetryPending = false;
const poll = setInterval(() => { if (!closed && !shuttingDown && !telemetryPending) { telemetryPending = true; void rpc('telemetry').then(value => report.telemetry.push({ at_ms: at(), scenario: scenario?.label ?? null, ...value })).catch(fail).finally(() => { telemetryPending = false; }); } }, 200);
function observe(project, frame) { controller.observe(previewModelDescriptor(project, 540, useProxies), project.fps, Math.max(...project.clips.map(c => c.start + c.duration)), frame, project.clips.length > 0); }
async function newScenario(label, project, frame) {
  requestSpeed(0); controller.cancelPreparation(); callbackGeneration++; currentAsset = null; candidateAsset = null; await pinAssets();
  scenario = { label, began_ms: at(), assets: [], files: [], boundaries: [], start_frame: frame, speed_events: [], unavailable_events: [] };
  const snapshot = join(output, `${label}.monocut`); project = await rpc('save', { path: snapshot }); scenario.project_snapshot = snapshot; scenario.project_snapshot_sha256 = hash(snapshot); report.scenarios.push(scenario);
  observe(project, frame); await controller.request(false);
  await waitFor(() => currentAsset?.playable && currentAsset.validated, `${label} current file`);
  scenario.initial_ready_ms = at(); return project;
}
async function readyBoundary(frame, expectedAt) {
  const before = { speed: transport.currentSpeed, coverage: controller.coverage, successor: controller.successor ?? null };
  transport.advance(frame, (next, direction) => controller.advance(next, direction));
  const after = { speed: transport.currentSpeed, coverage: controller.coverage };
  const row = { frame, expected_at_ms: expectedAt, observed_at_ms: at(), before, after, ready_job_id: currentAsset?.job.id ?? null, ready_file_ms: currentAsset?.validated?.file_ready_ms ?? null };
  scenario.boundaries.push(row); trace('boundary', row);
  if (before.speed !== 1 || after.speed !== 1 || after.coverage?.start_frame !== frame || !currentAsset?.validated || currentAsset.validated.file_ready_ms > expectedAt) throw new Error(`Successor was not validated before actual boundary${frame}.`);
}
async function continuousRun(project) {
  project = await newScenario('forward-20s', project, 0);
  const owner = scenario; owner.playback_started_ms = at(); requestSpeed(1);
  let frame = 0, lastTraced = -1;
  while (frame < 600) {
    const now = at(); frame = Math.min(600, Math.floor((now - owner.playback_started_ms) * 30 / 1000));
    controller.trackFrame(Math.min(frame, 599));
    const active = controller.coverage;
    if (frame < 600 && active && frame >= active.end_frame) await readyBoundary(active.end_frame, owner.playback_started_ms + active.end_frame * 1000 / 30);
    if (frame !== lastTraced && frame < 600) { lastTraced = frame; trace('global-clock', { frame, speed: transport.currentSpeed, coverage: controller.coverage, pending: controller.pendingRegion, successor: controller.successor ?? null, active_job_id: scheduler.activeJobId, expected_key: scheduler.expectedKey, pending_native_identity_calls: [...pending.values()].filter(r => r.op === 'identity').length, pending_native_render_calls: [...pending.values()].filter(r => r.op === 'render').length }); }
    if (frame < 600 && (!currentAsset?.job.path || !existsSync(currentAsset.job.path))) throw new Error('Preparation invalidated/deleted the currentlyconsumed actual file.');
    if (transport.currentSpeed !== 1) throw new Error('Ordinary prepared playback forcibly stopped before20seconds.');
    if (report.failures.length) throw new Error(report.failures.at(-1).message);
    await sleep(10);
  }
  owner.playback_finished_ms = at(); owner.elapsed_playback_ms = owner.playback_finished_ms - owner.playback_started_ms; owner.end_frame = 600;
  requestSpeed(0); controller.cancelPreparation();
  owner.consumed = owner.files.filter(f => f.playable && f.region.start_frame < 600 && f.region.end_frame <= 600).filter((f, index, rows) => rows.findIndex(v => v.job_id === f.job_id) === index).sort((a, b) => a.region.start_frame - b.region.start_frame);
  if (owner.consumed.length !== 4 || owner.consumed.some((f, n) => f.region.start_frame !== n * 150 || f.region.end_frame !== (n + 1) * 150)) throw new Error('20-second consumed coverage omitted/duplicated a region.');
  owner.passed = owner.boundaries.length === 3 && owner.elapsed_playback_ms >= 20_000;
  trace('run-finished', { elapsed_ms: owner.elapsed_playback_ms, consumed_regions: owner.consumed.map(f => f.region) });
  return project;
}
async function interruptedScenario(project, label, edit) {
  project = await newScenario(label, project, 600); const owner = scenario;
  requestSpeed(1);
  await waitFor(() => scheduler.activeJobId && report.jobs.some(e => e.at_ms >= owner.began_ms && e.job.id === scheduler.activeJobId && e.job.status === 'running' && e.job.preview_region?.start_frame === 750), `${label} native successor running`);
  owner.interrupted_job_id = scheduler.activeJobId;
  const before = { coverage: controller.coverage, successor: controller.successor ?? null, pending: controller.pendingRegion, native_jobs: await rpc('list') };
  if (!before.native_jobs.some(job => job.id === owner.interrupted_job_id && job.status === 'running')) throw new Error('Intervention did not catch its current successor job while actually running.');
  owner.action_started_ms = at(); transport.cancelResume(); requestSpeed(0);
  const target = edit ? 620 : 1100; owner.playhead = target;
  if (edit) {
    const track = project.tracks.find(t => t.kind === 'video'), clip = project.clips.find(c => c.track_id === track.id && c.media_id && c.start <= target && target < c.start + c.duration);
    if (!clip) throw new Error('No visible base clip for real edit.');
    owner.command = { type: 'update_clip', id: clip.id, patch: { brightness: clip.brightness === .0875 ? .0975 : .0875 } };
    project = await rpc('edit', { command: owner.command }); observe(project, target);
  } else controller.seek(target, 1);
  await controller.request(false);
  await waitFor(() => owner.files.some(f => f.region.start_frame === target && !f.playable), `${label} changed decoded frame`);
  owner.first_frame_ms = owner.files.find(f => f.region.start_frame === target && !f.playable).file_ready_ms - owner.action_started_ms;
  await waitFor(() => currentAsset?.playable && currentAsset.validated && currentAsset.region.start_frame === target, `${label} changed playable region`);
  owner.continuous_ready_ms = currentAsset.validated.file_ready_ms - owner.action_started_ms;
  owner.after = { speed: transport.currentSpeed, coverage: controller.coverage, successor: controller.successor ?? null, native_jobs: await rpc('list') }; owner.before = before;
  if (transport.currentSpeed !== 0) throw new Error('A late successor restarted transport after explicit Stop/action.');
  if (edit && (owner.first_frame_ms > 2000 || owner.continuous_ready_ms > 5000)) throw new Error('Real edit during successor preparation exceeded ordinary response targets.');
  const snapshot = join(output, `${label}-after.monocut`); project = await rpc('save', { path: snapshot }); owner.after_project_snapshot = snapshot; owner.after_project_snapshot_sha256 = hash(snapshot);
  owner.passed = true; requestSpeed(0); controller.cancelPreparation(); return project;
}
try {
  session = await rpc('session');
  let project = await rpc('get_project'); report.startup = startup; report.fixture_sources = fixtureHashes(project);
  report.fixture = { fps: project.fps, sample_rate: project.sample_rate, width: project.width, height: project.height, clips: project.clips.length, tracks: project.tracks.length };
  if (project.fps.num !== 30 || project.fps.den !== 1 || project.sample_rate !== 48000 || Math.max(...project.clips.map(c => c.start + c.duration)) < 1800) throw new Error('This explicit ordinary acceptance harness requires60seconds30fps48k stereo fixture.');
  if (startup?.driver_executable_sha256 !== report.tools.driver_sha256) throw new Error('Running native driver executable differs from recorded hash.');
  project = await continuousRun(project);
  project = await interruptedScenario(project, 'middle-seek-during-successor', false);
  project = await interruptedScenario(project, 'edit-during-successor', true);
  requestSpeed(0); disposeController(); callbackGeneration++; currentAsset = null; candidateAsset = null; await pinAssets();
  await Promise.allSettled([...tasks]);
  await waitFor(() => fileTools.size === 0, 'silent file readers reaped');
  for (const item of report.scenarios) {
    const traces = report.traces.filter(e => e.scenario === item.label);
    item.speed_events = traces.filter(e => e.kind === 'speed'); item.unavailable_events = traces.filter(e => e.kind === 'unavailable');
    item.native_jobs = report.jobs.filter(e => e.scenario === item.label);
    const during = report.telemetry.filter(e => e.scenario === item.label);
    const previewPaths = t => new Set([t.preview_ownership?.current_preview, t.preview_ownership?.successor_preview, t.preview_ownership?.completed_preview].filter(Boolean)).size;
    item.resource_peaks = { managed_children: Math.max(0, ...during.map(t => t.active_managed_children)), cache_bytes: Math.max(0, ...during.map(t => t.cache_bytes)), protected_files: Math.max(0, ...during.map(t => t.protected_assets.files)), protected_pins: Math.max(0, ...during.map(t => t.protected_assets.pins)), protected_bytes: Math.max(0, ...during.map(t => t.protected_assets.bytes)), owned_preview_files: Math.max(0, ...during.map(previewPaths)) };
    if (item.resource_peaks.owned_preview_files > 2) throw new Error('More than current/adjoining preview files were owned simultaneously.');
  }
  const cleanupBegan = at();
  for (;;) {
    report.final_telemetry_before_shutdown = await rpc('telemetry');
    const actualPartials = cachePartials();
    report.cleanup_observations ??= []; report.cleanup_observations.push({ at_ms: at(), active_managed_children: report.final_telemetry_before_shutdown.active_managed_children, driver_partial_count: report.final_telemetry_before_shutdown.partial_count, actual_partial_files: actualPartials });
    if (report.final_telemetry_before_shutdown.active_managed_children === 0 && actualPartials.length === 0) break;
    if (at() - cleanupBegan > 20000) throw new Error('Cancelled preparation did not reap its children/partials within20seconds.');
    await sleep(50);
  }
  report.cleanup_wait_ms = at() - cleanupBegan;
  report.cache_inventory_after = cacheFiles(); report.actual_partial_files_after = cachePartials();
  const useful = report.telemetry.map(t => t.preview_ownership?.useful_active_preparations ?? 0);
  report.max_useful_active_preparations = Math.max(0, ...useful);
  if (report.max_useful_active_preparations > 1) throw new Error('More than one useful native preview preparation ran at once.');
  report.final_jobs = await rpc('list');
  for (const item of report.scenarios.filter(s => s.interrupted_job_id)) {
    item.interrupted_job_terminal = report.final_jobs.find(job => job.id === item.interrupted_job_id);
    if (item.interrupted_job_terminal?.status !== 'cancelled') throw new Error(`${item.label} did not cancel its actuallyrunning predecessor.`);
  }
  await rpc('shutdown');
  report.sources_after = sourceHashes(); report.tools_after = toolHashes(); report.fixture_sources_after = fixtureHashes(project); report.fixture_sha256_after = hash(projectPath);
  report.sources_unchanged = JSON.stringify(report.sources) === JSON.stringify(report.sources_after);
  report.tools_unchanged = JSON.stringify(report.tools) === JSON.stringify(report.tools_after);
  report.inputs_unchanged = report.fixture_sha256 === report.fixture_sha256_after && JSON.stringify(report.fixture_sources) === JSON.stringify(report.fixture_sources_after);
  report.verified_file_helper = report.scenarios.every(s => s.passed) && !report.failures.length && report.sources_unchanged && report.tools_unchanged && report.inputs_unchanged;
  if (!report.verified_file_helper) throw new Error('Continuity file/helper validation did not meet its declared gates.');
} catch (error) { fail(error); process.exitCode = 1; }
finally {
  clearInterval(poll); disposeController(); for (const child of fileTools) child.kill();
  if (!closed && !shuttingDown) { try { await rpc('shutdown'); } catch {} }
  driver.stdin.end(); if (!closed) await new Promise(resolve => { const timeout = setTimeout(() => { driver.kill(); resolve(); }, 5000); driver.once('exit', () => { clearTimeout(timeout); resolve(); }); });
  const fileCleanupBegan = at(); while (fileTools.size && at() - fileCleanupBegan < 5000) await sleep(10);
  report.finished_utc = new Date().toISOString(); report.diagnostics = diagnostics; report.closed = closed; report.pending_rpc_after = pending.size; report.file_tools_after = fileTools.size;
  writeFileSync(join(output, 'continuity-results.json'), JSON.stringify(report, null, 2));
  process.stdout.write(`${JSON.stringify({ mode, verified_file_helper: report.verified_file_helper ?? false, boundaries: report.scenarios[0]?.boundaries, interruptions: report.scenarios.slice(1).map(s => ({ label: s.label, first_frame_ms: s.first_frame_ms, continuous_ready_ms: s.continuous_ready_ms, passed: s.passed })), failures: report.failures, report: join(output, 'continuity-results.json') })}\n`);
}
