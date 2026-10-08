# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Mono Cut contributors
# This helper performs silent file validation and never opens an audio device.
# Raw output contains private local paths and must stay outside the checkout.
# Requires open-source Python and NumPy; media tools come from the documented
# bundled GPL configuration or explicit --ffmpeg/--ffprobe arguments.
"""Silent changed-asset gate using the frozen production driver/compiler, not a copied renderer."""
import argparse, array, hashlib, json, math, os, pathlib, queue, re, subprocess, sys, threading, time
import numpy as np
parser = argparse.ArgumentParser(description='Silently compare a retained ordinary controller-edit asset with its exact saved project state. Requires a 30 fps, 48 kHz stereo, 150-frame originals cohort.')
parser.add_argument('--report', required=True, type=pathlib.Path)
parser.add_argument('--label', default='warm-edit-8')
parser.add_argument('--snapshot', required=True, type=pathlib.Path)
parser.add_argument('--output', required=True, type=pathlib.Path)
parser.add_argument('--driver', required=True, type=pathlib.Path)
parser.add_argument('--native', type=pathlib.Path)
parser.add_argument('--ffmpeg', type=pathlib.Path)
parser.add_argument('--ffprobe', type=pathlib.Path)
parser.add_argument('--font', type=pathlib.Path)
args = parser.parse_args()
repo = pathlib.Path(__file__).resolve().parents[1]
measure = args.report.resolve()
measured = json.loads(measure.read_text())
sample = next((s for s in measured['samples'] if s['label'] == args.label))
snapshot = args.snapshot.resolve()
native_input = args.native or pathlib.Path(sample['continuous']['path'])
native = native_input.resolve() if native_input.is_absolute() else (measure.parent / native_input).resolve()
out = args.output.resolve()
if out.is_relative_to(repo):
    parser.error('Keep private media, raw diagnostics, and local paths outside the source checkout.')
out.mkdir(parents=True, exist_ok=True)
cache = out / 'cache'
cache.mkdir(exist_ok=True)
driver_path = args.driver.resolve()
resources = repo / 'src-tauri/resources/media'
ffmpeg = (args.ffmpeg or resources / ('ffmpeg.exe' if os.name == 'nt' else 'ffmpeg')).resolve()
ffprobe = (args.ffprobe or resources / ('ffprobe.exe' if os.name == 'nt' else 'ffprobe')).resolve()
font = (args.font or resources / 'Inter.ttf').resolve()
start = sample['continuous']['region']['start_frame']
end = sample['continuous']['region']['end_frame']
samples0, samples1 = (start * 1600, end * 1600)
hidden = getattr(subprocess, 'CREATE_NO_WINDOW', 0)
sha = lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
byte_sha = lambda b: hashlib.sha256(b).hexdigest()
source_hashes = lambda: {name: sha(repo / name) for name in measured['sources']}
source_path = lambda text: pathlib.Path(text) if pathlib.Path(text).is_absolute() else snapshot.parent / pathlib.Path(text)
inputs_hashes = lambda: {'snapshot': sha(snapshot), 'native': sha(native), 'originals': {m['id']: sha(source_path(m['path'])) for m in json.loads(snapshot.read_text())['media']}}
report = {'stage': '4C', 'gate': f'retained originals {args.label} native asset versus exact cumulative saved state', 'helper_sha256': sha(__file__), 'measurement_sha256': sha(measure), 'snapshot_sha256': sha(snapshot), 'native_sha256': sha(native), 'region': [start, end], 'global_stereo_sample_coverage': [samples0, samples1], 'expected_video_frames': 150, 'expected_stereo_sample_frames': 240000, 'native_audio_or_ui_launched': False, 'static_equivalence_result_is_not_replaced_by_this_content_gate': True, 'tolerances': {'gray_all_and_per_frame_rms': 5, 'selected_rgb_marker_rms': 6, 'ordinary_aac_vs_full_flac_rms': 0.003, 'lossless_float_pcm': 'strict bits', 'lossless_rgb': 'strict bytes'}, 'sources_before': source_hashes(), 'inputs_before': inputs_hashes(), 'tools': {'driver_sha256': sha(driver_path), 'ffmpeg_sha256': sha(ffmpeg), 'ffprobe_sha256': sha(ffprobe), 'font_sha256': sha(font), 'python_executable_sha256': sha(sys.executable), 'numpy_version': np.__version__, 'numpy_module_sha256': sha(np.__file__), 'numpy_binary_sha256': sha(np._core._multiarray_umath.__file__)}, 'verified_content': False, 'failures': [], 'rpc': [], 'file_tools': []}
report_path = out / 'changed-asset-reference-results.json'
saved = json.loads(snapshot.read_text())
messages = queue.Queue()
driver = None
serial = 0
children = []

def save():
    report_path.write_text(json.dumps(report, indent=2))

def read_stdout(pipe):
    for line in pipe:
        try:
            messages.put(json.loads(line))
        except Exception:
            messages.put({'parse_failure': line})
    messages.put({'eof': True})

def rpc(op, **kw):
    global serial
    serial += 1
    request = {'id': serial, 'op': op, **kw}
    driver.stdin.write(json.dumps(request) + '\n')
    driver.stdin.flush()
    report['rpc'].append({'request': request})
    deadline = time.monotonic() + 120
    while True:
        item = messages.get(timeout=max(0.001, deadline - time.monotonic()))
        if item.get('eof'):
            raise RuntimeError('Driver closed during RPC')
        if item.get('id') != serial:
            continue
        report['rpc'].append({'reply': item})
        if item.get('error'):
            raise RuntimeError(item['error'])
        return item.get('result')

def run_tool(tool, args, decode=False):
    began = time.monotonic()
    child = subprocess.Popen([str(tool), *map(str, args)], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=hidden)
    children.append(child)
    try:
        raw, err = child.communicate(timeout=180)
    except subprocess.TimeoutExpired:
        child.kill()
        child.communicate()
        raise
    record = {'tool': tool.name, 'args': list(map(str, args)), 'pid': child.pid, 'exit_code': child.returncode, 'seconds_correctness_only': time.monotonic() - began, 'stdout_bytes': len(raw)}
    report['file_tools'].append(record)
    if child.returncode:
        raise RuntimeError(err.decode(errors='replace')[-8000:])
    return raw

def retained_plan(label, region):
    plan = rpc('plan', height=540, useProxies=False, region=region)
    graph = plan['filter_graph']
    gp = out / f'{label}-filter.txt'
    gp.write_text(graph)
    path = pathlib.Path(plan['args'][plan['args'].index('-filter_complex_script') + 1])
    path.write_text(graph)
    titles = [c['title'] for c in saved['clips'] if c.get('media_id') is None and c.get('title') is not None]
    refs = re.findall("textfile='((?:\\\\.|[^'])*)'", graph)
    assert len(refs) <= 1 and (not refs or len(titles) == 1)
    for title_ref in refs:
        title_path = pathlib.Path(title_ref.replace('\\:', ':').replace("\\'", "'"))
        title_path.write_text(titles[0])
    plan['retained_graph_sha256'] = sha(gp)
    return plan

def codec_args(plan, output, audio_codec):
    args = plan['args'].copy()
    for option in ['-preset', '-crf', '-b:a', '-movflags', '-g', '-keyint_min', '-sc_threshold']:
        while option in args:
            at = args.index(option)
            del args[at:at + 2]
    args[args.index('-c:v') + 1] = 'ffv1'
    args[args.index('-c:a') + 1] = audio_codec
    args[-1] = str(output)
    args[-1:-1] = ['-level', '3', '-coder', '1', '-context', '1']
    return args

def audio(path, a, b):
    raw = run_tool(ffmpeg, ['-v', 'error', '-nostdin', '-threads', '1', '-i', path, '-map', '0:a:0', '-vn', '-af', f'atrim=start_sample={a}:end_sample={b}', '-ar', '48000', '-ac', '2', '-f', 'f32le', 'pipe:1'], True)
    assert len(raw) == (b - a) * 8
    return raw

def video(path, a, b, fmt='rgb24', linear=False):
    args = ['-v', 'error', '-nostdin', '-threads', '1']
    origin = 0
    if not linear:
        origin = a // 30 * 30
        args += ['-ss', str(a // 30)]
    args += ['-i', path, '-map', '0:v:0', '-an', '-vf', f'trim=start_frame={a - origin}:end_frame={b - origin}', '-fps_mode', 'passthrough', '-threads', '1', '-f', 'rawvideo', '-pix_fmt', fmt, 'pipe:1']
    raw = run_tool(ffmpeg, args, True)
    assert len(raw) == (b - a) * 960 * 540 * (3 if fmt == 'rgb24' else 1)
    return raw

def metrics(a, b, dtype):
    x = np.frombuffer(a, dtype=dtype).astype(np.float64)
    y = np.frombuffer(b, dtype=dtype).astype(np.float64)
    assert x.shape == y.shape
    d = x - y
    return {'rms': float(np.sqrt(np.mean(d * d))), 'max': float(np.max(np.abs(d))), 'values': int(len(x))}

def gray_metrics(a, b):
    x = np.frombuffer(a, dtype=np.uint8).reshape(150, -1)
    y = np.frombuffer(b, dtype=np.uint8).reshape(150, -1)
    sums = []
    peaks = []
    for xx, yy in zip(x, y):
        d = xx.astype(np.float64) - yy.astype(np.float64)
        sums.append(float(np.sum(d * d)))
        peaks.append(float(np.max(np.abs(d))))
    return {'rms': math.sqrt(sum(sums) / len(a)), 'per_frame_max_rms': max((math.sqrt(s / x.shape[1]) for s in sums)), 'max': max(peaks), 'values': len(a)}

def marker(raw):
    return np.frombuffer(raw, dtype=np.uint8).reshape(540, 960, 3)[:75, :260, :].tobytes()

def profile(path):
    data = path.read_bytes()
    prefix = b'options: '
    at = data.index(prefix) + len(prefix)
    stop = data.index(b'\x00', at)
    values = dict((x.split('=', 1) for x in data[at:stop].decode().split() if '=' in x))
    result = {'rate_control': values['rc'], 'crf': float(values['crf']), 'gop_frames': int(values['keyint']), 'effective_min_keyint': int(values['keyint_min']), 'scenecut': int(values['scenecut'])}
    assert result == {'rate_control': 'crf', 'crf': 20.0, 'gop_frames': 15, 'effective_min_keyint': 8, 'scenecut': 0}
    return result
try:
    assert sha(snapshot) == sample['project_snapshot_sha256']
    assert sha(native) == sample['continuous']['file_sha256']
    assert report['tools']['driver_sha256'] == measured['tools']['driver_sha256']
    assert measured['mode'] == 'originals' and end - start == 150
    for name in ['ffmpeg_sha256', 'ffprobe_sha256', 'font_sha256']:
        assert report['tools'][name] == measured['tools'][name]
    assert all((report['sources_before'][name] == expected for name, expected in measured['sources'].items()))
    saved_range = (saved['in_point'], saved['out_point'])
    assert saved['fps'] == {'num': 30, 'den': 1} and saved['sample_rate'] == 48000
    assert sample['continuous']['region'] == {'start_frame': start, 'end_frame': end}
    env = {**os.environ, 'MONO_CUT_FFMPEG': str(ffmpeg), 'MONO_CUT_FFPROBE': str(ffprobe), 'MONO_CUT_MEASURE_PARENT_PID': str(os.getpid())}
    stderr = (out / 'driver-stderr.log').open('wb')
    driver = subprocess.Popen([str(driver_path), str(snapshot), str(cache), str(font)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, encoding='utf-8', creationflags=hidden, env=env)
    threading.Thread(target=read_stdout, args=(driver.stdout,), daemon=True).start()
    ready = messages.get(timeout=120)
    assert ready['event'] == 'ready'
    assert ready['driver_executable_sha256'] == report['tools']['driver_sha256']
    hydrated = rpc('get_project')
    assert hydrated['clips'] == saved['clips']
    assert (hydrated['in_point'], hydrated['out_point']) == saved_range
    report['preserved_saved_coordinates_and_export_range'] = True
    report['encoded_profile'] = profile(native)
    metadata = json.loads(run_tool(ffprobe, ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', native]))
    report['native_metadata'] = metadata
    vs = next((s for s in metadata['streams'] if s['codec_type'] == 'video'))
    aus = next((s for s in metadata['streams'] if s['codec_type'] == 'audio'))
    assert int(vs['nb_frames']) == 150 and vs['width'] == 960 and (vs['height'] == 540) and (vs['avg_frame_rate'] == '30/1')
    assert int(aus['sample_rate']) == 48000 and int(aus['channels']) == 2
    assert abs(float(vs['duration']) - 5.0) <= 1 / 60
    full_plan = retained_plan('full', None)
    full = out / 'full-flac.mkv'
    full_args = codec_args(full_plan, full, 'flac')
    run_tool(ffmpeg, full_args)
    report['full_reference'] = {'plan': full_plan, 'args': full_args, 'sha256': sha(full)}
    save()
    float_full = out / 'full-float.mka'
    float_graph = out / 'full-float-filter.txt'
    float_graph.write_text(full_plan['filter_graph'] + ';[vout]nullsink')
    float_args = full_args.copy()
    float_args[float_args.index('-filter_complex_script') + 1] = str(float_graph)
    at = next((i for i in range(len(float_args) - 1) if float_args[i:i + 2] == ['-map', '[vout]']))
    del float_args[at:at + 2]
    float_args[float_args.index('-c:a') + 1] = 'pcm_f32le'
    float_args[-1] = str(float_full)
    run_tool(ffmpeg, float_args)
    report['full_float_control'] = {'args': float_args, 'sha256': sha(float_full), 'filter_sha256': sha(float_graph), 'source_levels_changed': False}
    save()
    bounded_plan = retained_plan('bounded', {'start_frame': start, 'end_frame': end})
    lossless = out / 'bounded-lossless-float.mkv'
    bounded_args = codec_args(bounded_plan, lossless, 'pcm_f32le')
    run_tool(ffmpeg, bounded_args)
    report['lossless_region'] = {'plan': bounded_plan, 'args': bounded_args, 'sha256': sha(lossless)}
    save()
    for frame in [start, start + 75, end - 1]:
        assert video(full, frame, frame + 1) == video(full, frame, frame + 1, linear=True)
    report['whole_second_reference_seek_matches_linear'] = True
    expected_gray = video(full, start, end, 'gray')
    actual_gray = video(native, 0, 150, 'gray')
    gray = gray_metrics(expected_gray, actual_gray)
    assert gray['rms'] <= 5 and gray['per_frame_max_rms'] <= 5
    report['gray_allframes'] = gray
    del expected_gray, actual_gray
    pictures = []
    for local in [0, 75, 149]:
        a, b = (video(full, start + local, start + local + 1), video(native, local, local + 1))
        rgb = metrics(a, b, np.uint8)
        roi = metrics(marker(a), marker(b), np.uint8)
        assert rgb['rms'] <= 6 and roi['rms'] <= 6
        pictures.append({'global_frame': start + local, 'local_frame': local, 'rgb': rgb, 'marker': roi})
    report['selected_rgb'] = pictures
    a, b = (video(full, start, end), video(lossless, 0, 150))
    report['lossless_rgb_exact'] = a == b
    report['lossless_rgb_sha256'] = byte_sha(b)
    assert a == b
    del a, b
    full_audio = audio(full, samples0, samples1)
    native_audio = audio(native, 0, 240000)
    ordinary = metrics(full_audio, native_audio, np.float32)
    assert ordinary['rms'] <= 0.003
    report['ordinary_aac_vs_full_flac'] = ordinary
    expected_float = audio(float_full, samples0, samples1)
    bounded_float = audio(lossless, 0, 240000)
    report['strict_float_pcm_exact'] = expected_float == bounded_float
    report['float_metrics'] = metrics(expected_float, bounded_float, np.float32)
    report['full_float_sha256'] = byte_sha(expected_float)
    report['bounded_float_sha256'] = byte_sha(bounded_float)
    save()
    assert expected_float == bounded_float, 'Strict bounded float PCM differs from exact cumulative-state full float PCM'
    matching = out / 'matching-aac.m4a'
    run_tool(ffmpeg, ['-y', '-v', 'error', '-nostdin', '-i', lossless, '-map', '0:a:0', '-vn', '-c:a', 'aac', '-b:a', '160k', '-ar', '48000', '-ac', '2', matching])
    control = audio(matching, 0, 240000)
    report['matching_aac_control'] = {'sha256': sha(matching), 'native_decoded_exact': native_audio == control, 'metrics': metrics(native_audio, control, np.float32)}
    assert native_audio == control
    report['exact_frame_and_sample_coverage'] = True
    report['all_original_picture_audio_gates_passed'] = True
    report['verified_content'] = True
except BaseException as error:
    report['failures'].append(str(error) or repr(error))
    report['verified_content'] = False
finally:
    for child in children:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=10)
    if driver is not None and driver.poll() is None:
        try:
            report['native_final_telemetry'] = rpc('telemetry')
            rpc('shutdown')
            driver.stdin.close()
            driver.wait(timeout=20)
        except Exception as error:
            report['failures'].append('ordered driver cleanup: ' + str(error))
            report['verified_content'] = False
            driver.kill()
            driver.wait(timeout=10)
    report['driver_closed'] = driver is not None and driver.poll() is not None
    report['active_external_file_tools_after'] = sum((child.poll() is None for child in children))
    report['sources_after'] = source_hashes()
    report['inputs_after'] = inputs_hashes()
    report['sources_unchanged'] = report['sources_before'] == report['sources_after']
    report['inputs_unchanged'] = report['inputs_before'] == report['inputs_after']
    report['helper_unchanged'] = report['helper_sha256'] == sha(__file__)
    report['tools_after'] = {k: sha(p) for k, p in [('driver_sha256', driver_path), ('ffmpeg_sha256', ffmpeg), ('ffprobe_sha256', ffprobe), ('font_sha256', font)]}
    report['tools_unchanged'] = all((report['tools'][k] == v for k, v in report['tools_after'].items()))
    report['recursive_partial_files_after'] = [p.relative_to(cache).as_posix() for p in cache.rglob('*') if p.is_file() and re.search('\\.mono-(?:audio-)?part-|\\.json\\.tmp$', p.name)]
    cleanup = report.get('native_final_telemetry', {}).get('active_managed_children') == 0 and report['driver_closed'] and (report['active_external_file_tools_after'] == 0) and (not report['recursive_partial_files_after'])
    report['cleanup_passed'] = cleanup
    if not (cleanup and report['sources_unchanged'] and report['inputs_unchanged'] and report['tools_unchanged'] and report['helper_unchanged']):
        report['verified_content'] = False
        report['failures'].append('hash/cleanup invariant failed')
    save()
print(json.dumps({k: report.get(k) for k in ['verified_content', 'failures', 'gray_allframes', 'ordinary_aac_vs_full_flac', 'strict_float_pcm_exact', 'matching_aac_control', 'cleanup_passed', 'sources_unchanged', 'inputs_unchanged', 'tools_unchanged', 'helper_unchanged']}, indent=2))
sys.exit(0 if report['verified_content'] else 1)
