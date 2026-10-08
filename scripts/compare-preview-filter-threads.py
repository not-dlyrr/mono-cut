"""Silent 1/2/4-thread comparison of a retained production plan, without encoding overrides."""
import ctypes
import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path

repo = Path.cwd().resolve()
report_path = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
out.mkdir(exist_ok=False)
ffmpeg = repo / 'src-tauri/resources/media/ffmpeg.exe'
report = json.loads(report_path.read_text('utf-8-sig'))
sample = next(s for s in report['samples'] if s['label'] == 'warm-edit-5')
plan = sample['plan']
base_args = plan['args']
project_path = Path(sample['project_snapshot'])
project = json.loads(project_path.read_text('utf-8-sig'))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def args_sha(args):
    return hashlib.sha256(json.dumps(args, ensure_ascii=False, separators=(',', ':')).encode('utf-8')).hexdigest()


def option(args, key):
    return args[args.index(key) + 1] if key in args else None


assert option(base_args, '-crf') == '20', 'Recorded plan is not production quality20'
assert option(base_args, '-c:v') == 'libx264'
assert option(base_args, '-preset') == 'veryfast'
assert [option(base_args, key) for key in ['-g', '-keyint_min', '-sc_threshold']] == ['15', '15', '0'], 'Recorded plan lacks the executable native preview profile'
assert report.get('source_changed_during_measurement') == []
assert report.get('compiled_helper_changed_during_measurement') == []
assert sha(ffmpeg) == report['tools']['ffmpeg_sha256']
assert sha(project_path) == sample['project_snapshot_sha256']
source_hashes_before = {name: sha(repo / name) for name in report['sources']}
assert source_hashes_before == report['sources'], 'Current sources do not match the recorded production run'
for i, argument in enumerate(base_args):
    if argument == '-i':
        assert Path(base_args[i + 1]).is_file(), 'Recorded warm input cache is missing'

(out / 'recorded-plan.json').write_text(json.dumps(plan, indent=2), 'utf-8')
graph = plan['filter_graph']
sidecars = {Path(option(base_args, '-filter_complex_script')): graph.encode('utf-8')}
text_paths = re.findall(r"textfile='([^']+)'", graph)
titles = [clip['title'] for clip in project['clips'] if clip.get('title')]
assert len(text_paths) == 1 and len(titles) == 1, 'This comparison expects the ordinary fixture single title'
for escaped in text_paths:
    # The recorded graph escapes Windows drive colons; no graph text is changed.
    path = Path(escaped.replace('\\:', ':').replace("\\'", "'")).resolve()
    sidecars[path] = titles[0].encode('utf-8')
created = []
cache_root = (report_path.parent / 'cache').resolve()
for path, payload in sidecars.items():
    assert path.resolve().is_relative_to(cache_root), 'Sidecar destination escapes the retained private cache'
    if path.exists():
        assert path.read_bytes() == payload, 'Existing sidecar differs from recorded content'
    else:
        path.write_bytes(payload)
        created.append(path)


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [('cb', ctypes.c_ulong), ('PageFaultCount', ctypes.c_ulong)] + [
        (key, ctypes.c_size_t) for key in ['PeakWorkingSetSize', 'WorkingSetSize',
        'QuotaPeakPagedPoolUsage', 'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage',
        'QuotaNonPagedPoolUsage', 'PagefileUsage', 'PeakPagefileUsage']]


def working_set(child):
    counters = ProcessMemoryCounters()
    counters.cb = ctypes.sizeof(counters)
    ok = ctypes.windll.psapi.GetProcessMemoryInfo(ctypes.c_void_p(int(child._handle)), ctypes.byref(counters), counters.cb)
    return counters.PeakWorkingSetSize if ok else 0


def decode_hash(path, kind):
    args = ['-v', 'error', '-i', str(path)] + (
        ['-map', '0:v:0', '-pix_fmt', 'rgb24', '-an', '-f', 'rawvideo'] if kind == 'rgb'
        else ['-map', '0:a:0', '-ac', '2', '-ar', '48000', '-vn', '-f', 'f32le']) + ['pipe:1']
    with (out / f'{path.stem}-{kind}-decode.stderr.log').open('wb') as stderr:
        child = subprocess.Popen([str(ffmpeg), *args], stdout=subprocess.PIPE, stderr=stderr, creationflags=0x08000000)
        digest = hashlib.sha256()
        size = 0
        while block := child.stdout.read(1024 * 1024):
            digest.update(block)
            size += len(block)
        assert child.wait(timeout=30) == 0, 'Silent decode failed'
    return {'sha256': digest.hexdigest(), 'bytes': size, 'args': args}


samples = []
try:
    for threads in [1, 2, 4]:
        args = base_args.copy()
        args[args.index('-filter_complex_threads') + 1] = str(threads)
        destination = out / f'threads-{threads}.mp4'
        args[-1] = str(destination)
        changes = [{'index': i, 'recorded': left, 'trial': right} for i, (left, right) in enumerate(zip(base_args, args)) if left != right]
        assert all(change['index'] in [base_args.index('-filter_complex_threads') + 1, len(base_args) - 1] for change in changes)
        start = time.perf_counter()
        peak = 0
        with (out / f'threads-{threads}.progress.log').open('wb') as stdout, (out / f'threads-{threads}.stderr.log').open('wb') as stderr:
            child = subprocess.Popen([str(ffmpeg), *args], stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, creationflags=0x08000000)
            while child.poll() is None:
                peak = max(peak, working_set(child))
                if time.perf_counter() - start > 30:
                    child.kill()
                    child.wait()
                    raise AssertionError('Encoder trial exceeded 30 seconds')
                time.sleep(.01)
            elapsed = time.perf_counter() - start
            assert child.returncode == 0, 'Production-plan encode failed'
        result = {'filter_threads': threads, 'encode_seconds': elapsed, 'peak_working_set_bytes': peak,
            'output_bytes': destination.stat().st_size, 'output_sha256': sha(destination), 'args': args,
            'args_sha256': args_sha(args), 'changes_from_recorded_args': changes,
            'rgb': decode_hash(destination, 'rgb'), 'audio_pcm': decode_hash(destination, 'audio')}
        assert result['rgb']['bytes'] == plan['output_frames'] * 960 * 540 * 3
        samples.append(result)
        print(json.dumps({key: value for key, value in result.items() if key not in ['args', 'rgb', 'audio_pcm']}), flush=True)
    assert len({result['output_sha256'] for result in samples}) == 1, 'Filter threads changed MP4 bytes'
    assert len({result['rgb']['sha256'] for result in samples}) == 1, 'Filter threads changed decoded RGB'
    assert len({result['audio_pcm']['sha256'] for result in samples}) == 1, 'Filter threads changed decoded PCM'
    source_hashes_after = {name: sha(repo / name) for name in source_hashes_before}
    assert source_hashes_after == source_hashes_before
    result = {'source_report': str(report_path), 'source_report_sha256': sha(report_path),
        'source_project_sha256': sha(project_path), 'source_plan_sha256': sha(out / 'recorded-plan.json'),
        'recorded_args_sha256': args_sha(base_args), 'filter_graph_sha256': hashlib.sha256(graph.encode('utf-8')).hexdigest(),
        'sidecars': [{'path': str(path), 'bytes': len(payload), 'sha256': hashlib.sha256(payload).hexdigest()} for path, payload in sidecars.items()],
        'ffmpeg_sha256': sha(ffmpeg), 'comparison_helper_sha256': sha(Path(__file__)),
        'encoder_settings': sample['encoder_settings'], 'working_region': plan['working_region'],
        'input_ranges': plan['input_ranges'], 'audio_prefix_fallbacks': plan['audio_prefix_fallbacks'],
        'source_hashes_before': source_hashes_before, 'source_hashes_after': source_hashes_after,
        'samples': samples, 'mp4_bytes_exact': True, 'rgb_all_frames_exact': True, 'audio_pcm_exact': True,
        'endpoints': 'One warm encoder-only trial per thread count. Only -filter_complex_threads and final output destination differ from the exact recorded quality20 production plan, including its existing GOP15/keyint_min15/scenecut0 native preview profile. No GOP, preset, quality or input/output thread argument is added or altered. Silent all-frame/all-sample decode hashes are recorded separately. Actual-controller timing and source preparation are separate evidence.'}
    (out / 'results.json').write_text(json.dumps(result, indent=2), 'utf-8')
finally:
    for path in created:
        path.unlink()
