// Silent file/engine measurement through the actual production preview controller.
// This does not launch Tauri, a browser, a media player, or an installer.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { cpus, platform, release, totalmem } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const argumentsMap = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  if (!process.argv[index]?.startsWith('--') || !process.argv[index + 1]) throw new Error('Use --name value arguments.');
  argumentsMap.set(process.argv[index].slice(2), process.argv[index + 1]);
}
const required = name => { const value = argumentsMap.get(name); if (!value) throw new Error(`Missing --${name}.`); return value; };
const projectPath = resolve(required('project'));
const output = resolve(required('output'));
const mode = argumentsMap.get('mode') ?? 'originals';
if (!['originals', 'proxies'].includes(mode)) throw new Error('--mode must be originals or proxies.');
const useProxies = mode === 'proxies';
const editCount = Number(argumentsMap.get('edits') ?? 10);
if (!Number.isSafeInteger(editCount) || editCount < 0 || editCount > 10) throw new Error('--edits must be an integer from 0 to 10; acceptance requires ten.');
const driverPath = resolve(argumentsMap.get('driver') ?? join(root, 'src-tauri/target/debug/examples', process.platform === 'win32' ? 'preview_region_driver.exe' : 'preview_region_driver'));
const resources = join(root, 'src-tauri/resources/media');
const ffmpeg = resolve(argumentsMap.get('ffmpeg') ?? process.env.MONO_CUT_FFMPEG ?? join(resources, process.platform === 'win32' ? 'ffmpeg.exe' : 'ffmpeg'));
const ffprobe = resolve(argumentsMap.get('ffprobe') ?? process.env.MONO_CUT_FFPROBE ?? join(resources, process.platform === 'win32' ? 'ffprobe.exe' : 'ffprobe'));
const font = resolve(argumentsMap.get('font') ?? process.env.MONO_CUT_FONT ?? join(resources, 'Inter.ttf'));
const toolHashes = () => ({ driver_sha256: hash(driverPath), ffmpeg_sha256: hash(ffmpeg), ffprobe_sha256: hash(ffprobe), font_sha256: hash(font) });
const fixtureSourceHashes = project => project.media.map(media => ({ id: media.id, original_sha256: hash(media.path), proxy_sha256: media.proxy ? hash(media.proxy) : null, source_timing: media.timing, proxy_timing_version: media.proxy_timing_version }));
mkdirSync(output, { recursive: true });
const helperOutput = join(output, `production-helpers-${randomUUID()}`);
mkdirSync(helperOutput);
const compilationInputs = Object.fromEntries(readdirSync(join(root, 'src')).filter(name => /\.tsx?$/.test(name)).map(name => [name, hash(join(root, 'src', name))]));
const compilation = spawnSync(process.execPath, [join(root, 'node_modules/typescript/bin/tsc'), '--target', 'ES2022', '--module', 'commonjs', '--moduleResolution', 'node', '--skipLibCheck', '--strict', '--outDir', helperOutput, '--rootDir', join(root, 'src'), join(root, 'src/programPreview.ts'), join(root, 'src/previewModel.ts'), join(root, 'src/previewAssets.ts')], { cwd: root, encoding: 'utf8', windowsHide: true });
if (compilation.status !== 0) throw new Error(`Production helper compilation failed:\n${compilation.stdout}\n${compilation.stderr}`);
const require = createRequire(import.meta.url);
const { ProgramPreviewController, PREVIEW_DEBOUNCE_MS } = require(join(helperOutput, 'programPreview.js'));
const { PreviewBridge } = require(join(helperOutput, 'previewBridge.js'));
const { PreviewScheduler } = require(join(helperOutput, 'previewScheduler.js'));
const { PlaybackAssetQueue } = require(join(helperOutput, 'previewAssets.js'));
const { previewModelDescriptor } = require(join(helperOutput, 'previewModel.js'));

const cachePath = join(output, 'cache');
const driver = spawn(driverPath, [projectPath, cachePath, font], { cwd: root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, MONO_CUT_FFMPEG: ffmpeg, MONO_CUT_FFPROBE: ffprobe, MONO_CUT_MEASURE_PARENT_PID: String(process.pid) } });
let nextId = 0, controller = null, current = null, finished = false, startup = null;
const pending = new Map(), jobLog = [], rpcLog = [], telemetry = [], diagnostics = [];
const fileTools = new Set(), fileToolLog = []; let fileToolsPeak = 0;
function rpc(op, payload = {}) {
  if (finished) return Promise.reject(new Error('Measurement driver is closed.'));
  const id = ++nextId;
  return new Promise((accept, reject) => {
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`Driver ${op} timed out.`)); }, 120_000);
    pending.set(id, { accept, reject, timeout, op, began: performance.now() });
    driver.stdin.write(`${JSON.stringify({ id, op, ...payload })}\n`);
  });
}
driver.stderr.on('data', data => diagnostics.push(data.toString()));
createInterface({ input: driver.stdout }).on('line', line => {
  let value; try { value = JSON.parse(line); } catch { diagnostics.push(line); return; }
  if (value.event === 'ready') { startup = value; return; }
  if (value.event === 'job') {
    jobLog.push({ at: performance.now(), job: value.job }); controller?.receive(value.job); return;
  }
  if (value.event === 'telemetry') { telemetry.push(value); return; }
  const request = pending.get(value.id); if (!request) return;
  pending.delete(value.id); clearTimeout(request.timeout);
  rpcLog.push({ at: performance.now(), op: request.op, round_trip_ms: performance.now() - request.began, backend_rpc_ms: Number(value.backend_rpc_seconds ?? 0) * 1_000, error: value.error ?? null });
  if (value.error) request.reject(new Error(value.error)); else request.accept(value.result);
});
driver.on('error', error => { for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(error); } pending.clear(); current?.reject(error); });
driver.on('exit', code => { finished = true; for (const request of pending.values()) { clearTimeout(request.timeout); request.reject(new Error(`Driver exited ${code}.`)); } pending.clear(); if (code && current) current.reject(new Error(`Driver exited ${code}.`)); });

function runFileTool(tool, args, maximumBytes = 4 * 1024 * 1024) {
  return new Promise((accept, reject) => {
    const child = spawn(tool, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    fileTools.add(child); fileToolsPeak = Math.max(fileToolsPeak, fileTools.size);
    const record = { kind: tool === ffmpeg ? 'silent-frame-decode' : 'metadata-probe', pid: child.pid, started: performance.now(), active_tools: fileTools.size }; fileToolLog.push(record);
    const timeout = setTimeout(() => { child.kill(); reject(new Error('Silent decoder/probe exceeded its ten-second limit.')); }, 10_000);
    const chunks = []; let size = 0, error = '';
    child.stdout.on('data', data => { size += data.length; if (size > maximumBytes) { child.kill(); reject(new Error('Unexpected decoder output size.')); } else chunks.push(data); });
    child.stderr.on('data', data => { error += data.toString(); });
    child.on('error', error => { clearTimeout(timeout); fileTools.delete(child); reject(error); });
    child.on('exit', code => { clearTimeout(timeout); fileTools.delete(child); record.finished = performance.now(); record.exit_code = code; code === 0 ? accept(Buffer.concat(chunks)) : reject(new Error(`Silent file decoder failed (${code}): ${error}`)); });
  });
}
function encodedProfile(path) {
  const bytes = readFileSync(path), prefix = Buffer.from('options: ');
  const start = bytes.indexOf(prefix);
  if (start < 0) throw new Error('Actual preview has no x264 encoder-options SEI.');
  const end = bytes.indexOf(0, start + prefix.length);
  if (end < 0 || end - start > 4096) throw new Error('Actual preview has an invalid encoder-options SEI.');
  const options = Object.fromEntries(bytes.subarray(start + prefix.length, end).toString('ascii').split(/\s+/).map(token => token.split('=')));
  const number = name => { const value = Number(options[name]); if (!Number.isFinite(value)) throw new Error(`Actual preview SEI lacks ${name}.`); return value; };
  return { rate_control: options.rc, video_crf: number('crf'), gop_frames: number('keyint'), effective_min_keyint: number('keyint_min'), scenecut: number('scenecut') };
}
async function decodeFrame(path, localFrame) {
  const bytes = await runFileTool(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-nostdin', '-threads', '1', '-i', path, '-map', '0:v:0', '-vf', `select=eq(n\\,${localFrame})`, '-fps_mode', 'passthrough', '-frames:v', '1', '-threads', '1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
  if (bytes.length !== 960 * 540 * 3) throw new Error(`Decoded frame has ${bytes.length} bytes; expected a real 960x540 RGB frame.`);
  return { sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length, encoded_profile: encodedProfile(path) };
}
async function inspectRegion(path, region) {
  const details = JSON.parse((await runFileTool(ffprobe, ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', path])).toString());
  const video = details.streams?.find(stream => stream.codec_type === 'video');
  if (Number(video?.nb_frames) !== region.end_frame - region.start_frame || video?.width !== 960 || video?.height !== 540) throw new Error('Playable region metadata does not match exact coverage.');
  const [numerator, denominator] = String(video.avg_frame_rate).split('/').map(Number), expected = report.fixture.fps;
  if (!(denominator > 0) || numerator * expected.den !== expected.num * denominator) throw new Error('Playable region frame cadence differs from the global sequence.');
  const expectedSeconds = (region.end_frame - region.start_frame) * expected.den / expected.num;
  if (!Number.isFinite(Number(video.duration)) || Math.abs(Number(video.duration) - expectedSeconds) > expected.den / expected.num / 2) throw new Error('Playable region duration differs from rational coverage.');
  if (!details.streams?.some(stream => stream.codec_type === 'audio')) throw new Error('Playable region has no synchronized audio stream.');
  return { ...details, encoded_profile: encodedProfile(path) };
}

const report = {
  stage: '4B', mode, started_utc: new Date().toISOString(), project: projectPath, fixture_sha256: hash(projectPath),
  environment: { cpu: cpus()[0]?.model, logical_processors: cpus().length, physical_memory_bytes: totalmem(), platform: platform(), os_release: release(), node: process.version },
  cache_state: 'Empty application preview/audio caches for first preparation. OS file cache is not forcibly cleared; fixture generation, source/proxy preparation and hashing are reported separately.',
  endpoints: { first_frame: 'Before real ProjectStore edit RPC through production observe/request, identity, 100ms debounce, native job, silent decoded RGB frame and actual encoded-profile inspection.', continuous_ready: 'Same edit start through sequential first-frame stage and at least five seconds of completed region, exact frame count/audio-stream metadata and actual encoded-profile inspection. Full decoded reference comparison is recorded separately.', native_display: 'Not measured; no native editor/browser/player or audio output was launched.' },
  settings: { height: 540, width: 960, useProxies, debounce_ms: PREVIEW_DEBOUNCE_MS },
  sources: Object.fromEntries(['src/programPreview.ts', 'src/programTransport.ts', 'src/editorShortcuts.ts', 'src/previewBridge.ts', 'src/previewScheduler.ts', 'src/previewModel.ts', 'src/previewRegion.ts', 'src/previewAssets.ts', 'src/monitorLifecycle.ts', 'src/types.ts', 'src/App.tsx', 'src/Monitor.tsx', 'src/ProgramMedia.tsx', 'src/programMediaSlots.ts', 'src/GuidedTour.tsx', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/src/render.rs', 'src-tauri/src/jobs.rs', 'src-tauri/src/preview.rs', 'src-tauri/src/audio_clock.rs', 'src-tauri/src/video_seek.rs', 'src-tauri/src/model.rs', 'src-tauri/src/lib.rs', 'src-tauri/src/edit.rs', 'src-tauri/src/storage.rs', 'src-tauri/src/media.rs', 'src-tauri/src/processes.rs', 'src-tauri/tests/support/stage4b.rs', 'src-tauri/examples/preview_region_driver.rs', 'scripts/measure-preview-regions.mjs'].map(name => [name, hash(join(root, name))])),
  compilation_inputs_before: compilationInputs,
  compiled_helpers: Object.fromEntries(readdirSync(helperOutput).filter(name => name.endsWith('.js')).map(name => [name, hash(join(helperOutput, name))])),
  tools: toolHashes(), samples: [],
};
const scheduler = new PreviewScheduler();
const session = await rpc('session');
const bridge = new PreviewBridge(scheduler, {
  identity: settings => rpc('identity', settings),
  intent: (settings, expectedKey, revision) => rpc('intent', { ...settings, expectedKey, session, revision }),
  render: (settings, expectedKey, intent) => rpc('render', { ...settings, expectedKey, intent }),
});
const assets = new PlaybackAssetQueue(values => rpc('assets', values), error => current?.reject(error));
controller = new ProgramPreviewController(scheduler, bridge, () => ({ height: 540, useProxies }), {
  unavailable: () => {}, job: job => { if (current) current.jobs.push({ at_ms: performance.now() - current.started, job }); },
  error: error => current?.reject(error instanceof Error ? error : new Error(String(error))),
  timing: event => { if (current) current.timings.push({ ...event, at_ms: event.at - current.started }); },
  asset: (job, region, playable) => {
    const sample = current; if (!sample) return;
    const readyAt = performance.now() - sample.started;
    sample.assets.push({ ready_at_ms: readyAt, region, playable, path: job.path, key: job.preview_key, job_id: job.id });
    const work = (async () => {
      await assets.update({ previewPath: job.path, sourcePath: null });
      if (!playable) {
        const decoded = await decodeFrame(job.path, sample.playhead - region.start_frame);
        sample.first_frame_ms = performance.now() - sample.started; sample.first_frame = { ...decoded, path: job.path, region };
      } else {
        const metadata = await inspectRegion(job.path, region);
        sample.continuous_ready_ms = performance.now() - sample.started;
        sample.continuous = { path: job.path, region, file_sha256: hash(job.path), bytes: readFileSync(job.path).length, video_frames: Number(metadata.streams.find(stream => stream.codec_type === 'video').nb_frames), encoded_profile: metadata.encoded_profile };
        if (sample.first_frame_promise) await sample.first_frame_promise;
        else if (sample.first_frame_ms === undefined) {
          const decoded = await decodeFrame(job.path, sample.playhead - region.start_frame);
          sample.first_frame_ms = performance.now() - sample.started; sample.first_frame = { ...decoded, path: job.path, region };
        }
        sample.resolve();
      }
    })();
    if (!playable) sample.first_frame_promise = work;
    void work.catch(error => sample.reject(error));
  },
});

async function prepare(project, playhead, label, command = null) {
  await rpc('reset_peak');
  let accept, reject;
  const done = new Promise((yes, no) => { accept = yes; reject = no; });
  const sample = { label, playhead, command, started: performance.now(), timings: [], jobs: [], assets: [], resolve: accept, reject };
  current = sample;
  if (command) project = await rpc('edit', { command });
  const descriptor = previewModelDescriptor(project, 540, useProxies);
  controller.observe(descriptor, project.fps, Math.max(1, ...project.clips.map(clip => clip.start + clip.duration)), playhead, project.clips.length > 0);
  await controller.request(false);
  const timeout = setTimeout(() => reject(new Error(`${label} did not prepare a playable region within two minutes.`)), 120_000);
  try { await done; } finally { clearTimeout(timeout); }
  const plain = { ...sample }; delete plain.resolve; delete plain.reject; delete plain.started; delete plain.first_frame_promise;
  plain.job_timings = jobLog.filter(item => sample.assets.some(asset => asset.job_id === item.job.id)).map(item => ({ at_ms: item.at - sample.started, job: item.job }));
  plain.backend_job_intervals = plain.assets.map(asset => {
    const events = plain.job_timings.filter(item => item.job.id === asset.job_id), began = events.find(item => item.job.status === 'running'), completed = events.find(item => item.job.status === 'complete');
    return { job_id: asset.job_id, coverage: asset.region, lifecycle_ms: began && completed ? completed.at_ms - began.at_ms : null, note: 'Native job publication through terminal completion, including queued preparation, encoding and manifest validation; excludes UI debounce and decoder confirmation.' };
  });
  plain.rpc_timings = rpcLog.filter(item => item.at >= sample.started).map(item => ({ ...item, at_ms: item.at - sample.started }));
  plain.measurement_tool_intervals = fileToolLog.filter(item => item.started >= sample.started).map(item => ({ ...item, started_ms: item.started - sample.started, finished_ms: item.finished - sample.started }));
  plain.telemetry = await rpc('telemetry');
  plain.plan = await rpc('plan', { height: 540, useProxies, region: plain.continuous.region });
  const argument = flag => { const index = plain.plan.args.indexOf(flag); return index >= 0 ? plain.plan.args[index + 1] : null; };
  const numericArgument = flag => argument(flag) === null ? null : Number(argument(flag));
  plain.encoder_settings = {
    video_codec: argument('-c:v'), video_crf: numericArgument('-crf'),
    preset: argument('-preset'), gop_frames: numericArgument('-g'), requested_min_keyint: numericArgument('-keyint_min'), scenecut: numericArgument('-sc_threshold'),
    gop_policy: argument('-g') === null ? 'encoder default; no explicit GOP override' : 'explicit',
    pixel_format: argument('-pix_fmt'), output_fps: argument('-r'),
    sample_rate: numericArgument('-ar'), audio_codec: argument('-c:a'),
    audio_bitrate: argument('-b:a'), filter_threads: numericArgument('-filter_complex_threads'),
  };
  if (report.encoder_settings && JSON.stringify(report.encoder_settings) !== JSON.stringify(plain.encoder_settings)) throw new Error('Production encoder settings changed during measurement.');
  report.encoder_settings ??= plain.encoder_settings;
  for (const [kind, asset] of [['first-frame', plain.first_frame], ['continuous', plain.continuous]]) {
    const actual = asset.encoded_profile, intended = plain.encoder_settings;
    if (actual.rate_control !== 'crf' || actual.video_crf !== intended.video_crf || actual.gop_frames !== intended.gop_frames || actual.scenecut !== intended.scenecut) throw new Error(`${kind} actual encoder profile differs from the shared production plan.`);
  }
  if (JSON.stringify(plain.first_frame.encoded_profile) !== JSON.stringify(plain.continuous.encoded_profile)) throw new Error('First-frame and continuous previews use different effective profiles.');
  if (report.encoded_profile && JSON.stringify(report.encoded_profile) !== JSON.stringify(plain.continuous.encoded_profile)) throw new Error('Actual encoded profile changed during measurement.');
  report.encoded_profile ??= plain.continuous.encoded_profile;
  plain.project_snapshot = join(output, `${label}.monocut`);
  project = await rpc('save', { path: plain.project_snapshot });
  plain.project_snapshot_sha256 = hash(plain.project_snapshot);
  report.samples.push(plain); writeFileSync(join(output, 'controller-results.json'), JSON.stringify(report, null, 2));
  process.stdout.write(`${JSON.stringify({ sample: label, first_frame_ms: plain.first_frame_ms, continuous_ready_ms: plain.continuous_ready_ms })}\n`);
  current = null; return project;
}

try {
  let project = await rpc('get_project');
  report.startup = startup;
  if (startup?.driver_executable_sha256 !== report.tools.driver_sha256) throw new Error('Running driver differs from the recorded executable.');
  report.fixture_sources = fixtureSourceHashes(project);
  report.fixture = { width: project.width, height: project.height, fps: project.fps, sample_rate: project.sample_rate, clips: project.clips.length, tracks: project.tracks.map(track => ({ id: track.id, kind: track.kind })) };
  project = await prepare(project, 0, 'cold-first-preparation');
  const frames = [30, 180, 340, 370, 720, 850, 1010, 1300, 1610, 1770];
  const baseTrack = project.tracks.find(track => track.kind === 'video');
  for (let index = 0; index < editCount; index += 1) {
    const frame = frames[index];
    const clip = project.clips.find(clip => clip.track_id === baseTrack.id && clip.media_id && clip.start <= frame && frame < clip.start + clip.duration);
    if (!clip) throw new Error(`No visible base clip at edit frame ${frame}.`);
    const command = { type: 'update_clip', id: clip.id, patch: { brightness: 0.025 + index * 0.0125 } };
    project = await prepare(project, frame, `warm-edit-${index + 1}`, command);
  }
  const edits = report.samples.filter(sample => sample.label.startsWith('warm-edit-'));
  const percentile = values => values.length ? [...values].sort((a, b) => a - b)[Math.ceil(values.length * .95) - 1] : null;
  report.p95 = { first_frame_ms: percentile(edits.map(sample => sample.first_frame_ms)), continuous_ready_ms: percentile(edits.map(sample => sample.continuous_ready_ms)) };
  report.percentile_method = { method: 'nearest rank: sorted values at ceil(n*0.95)-1', actual_edit_samples: edits.length, note: 'With ten actual edit samples, p95 is the maximum.' };
  report.provisional_targets = { ten_actual_edits: edits.length === 10, first_frame_passed: report.p95.first_frame_ms !== null && report.p95.first_frame_ms <= 2_000, continuous_ready_passed: report.p95.continuous_ready_ms !== null && report.p95.continuous_ready_ms <= 5_000, accepted: false, note: 'Timing success alone is not acceptance. Decoded full-reference correctness, cancellation/scheduling regressions and resource bounds must also pass.' };
  report.final_telemetry = await rpc('telemetry');
  report.measurement_tools_peak = fileToolsPeak;
  report.source_hashes_after = Object.fromEntries(Object.keys(report.sources).map(name => [name, hash(join(root, name))]));
  report.compilation_inputs_after = Object.fromEntries(Object.keys(compilationInputs).map(name => [name, hash(join(root, 'src', name))]));
  report.compilation_input_changed_during_measurement = Object.keys(compilationInputs).filter(name => compilationInputs[name] !== report.compilation_inputs_after[name]);
  report.compiled_helpers_after = Object.fromEntries(Object.keys(report.compiled_helpers).map(name => [name, hash(join(helperOutput, name))]));
  report.compiled_helper_changed_during_measurement = Object.keys(report.compiled_helpers).filter(name => report.compiled_helpers[name] !== report.compiled_helpers_after[name]);
  report.source_changed_during_measurement = Object.keys(report.sources).filter(name => report.sources[name] !== report.source_hashes_after[name]);
  report.tools_after = toolHashes();
  report.tool_changed_during_measurement = Object.keys(report.tools).filter(name => report.tools[name] !== report.tools_after[name]);
  report.fixture_sources_after = fixtureSourceHashes(project);
  report.fixture_source_changed_during_measurement = report.fixture_sources.filter(media => {
    const after = report.fixture_sources_after.find(next => next.id === media.id);
    return !after || media.original_sha256 !== after.original_sha256 || media.proxy_sha256 !== after.proxy_sha256;
  }).map(media => media.id);
  report.fixture_sha256_after = hash(projectPath);
  report.input_hashes_unchanged = report.tool_changed_during_measurement.length === 0 && report.fixture_source_changed_during_measurement.length === 0 && report.fixture_sha256_after === report.fixture_sha256;
  if (report.source_changed_during_measurement.length || !report.input_hashes_unchanged) report.provisional_targets.note += ' Source, tool or fixture changed during this run; repeat timing after freezing all inputs.';
  report.job_events = jobLog; report.telemetry_events = telemetry; report.diagnostics = diagnostics;
  writeFileSync(join(output, 'controller-results.json'), JSON.stringify(report, null, 2));
  process.stdout.write(`${JSON.stringify({ mode, samples: report.samples.length, p95: report.p95, targets: report.provisional_targets })}\n`);
} finally {
  for (const child of fileTools) child.kill();
  controller.dispose(); await assets.update({ previewPath: null, sourcePath: null }, true);
  await rpc('shutdown').catch(() => {}); driver.stdin.end();
}
