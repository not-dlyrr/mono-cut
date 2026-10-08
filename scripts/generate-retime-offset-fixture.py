# SPDX-License-Identifier: GPL-3.0-or-later
"""Silently remux and verify the public ordinary fixture's 200 ms A/V offset.

Run the repository's explicit prepare_reproducible_ordinary_fixture test first.
This helper preserves every encoded packet and decoded float audio sample.
"""
import argparse, hashlib, json, os, pathlib, subprocess, sys, time
from fractions import Fraction
import numpy as np

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--project', required=True, type=pathlib.Path, help='Generated ordinary-60s.monocut')
parser.add_argument('--output', required=True, type=pathlib.Path, help='Fresh capture directory outside the checkout')
parser.add_argument('--repo', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
args = parser.parse_args()
repo = args.repo.resolve()
out = args.output.resolve()
if out.is_relative_to(repo):
    parser.error('Keep raw fixture captures and local paths outside the checkout')
out.mkdir(parents=True, exist_ok=True)
resources = repo / 'src-tauri/resources/media'
ffmpeg = resources / ('ffmpeg.exe' if os.name == 'nt' else 'ffmpeg')
ffprobe = resources / ('ffprobe.exe' if os.name == 'nt' else 'ffprobe')
project = json.loads(args.project.read_text())
source_path = lambda m: pathlib.Path(m['path']).resolve() if pathlib.Path(m['path']).is_absolute() else (args.project.resolve().parent / m['path']).resolve()
original, mixed = [source_path(m) for m in project['media'][:2]]
offset = out / 'source-1080p30-offset.mkv'
sha = lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
report = {'version': 1, 'stage': 5, 'native_ui_or_audio_launched': False,
          'helper_sha256': sha(__file__), 'before_inputs': {'original': sha(original), 'mixed': sha(mixed)},
          'tools': {p.name: sha(p) for p in [ffmpeg, ffprobe, resources / 'Inter.ttf']},
          'commands': [], 'verified_fixtures': False}
report_path = out / 'fixture-results.json'

def run(tool, args):
    began = time.monotonic()
    child = subprocess.run([str(tool), *map(str, args)], stdin=subprocess.DEVNULL,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0), timeout=180)
    report['commands'].append({'tool': tool.name, 'args': list(map(str, args)),
                              'exit_code': child.returncode, 'stdout_bytes': len(child.stdout),
                              'seconds_correctness_only': time.monotonic() - began})
    if child.returncode:
        raise RuntimeError(child.stderr.decode(errors='replace'))
    return child.stdout

def probe(path):
    value = json.loads(run(ffprobe, ['-v', 'error', '-show_format', '-show_streams',
                                    '-show_packets', '-show_data_hash', 'sha256',
                                    '-show_entries', 'packet=stream_index,pts_time,dts_time,duration_time,data_hash:stream:format',
                                    '-of', 'json', path]))
    saved = out / f'{path.stem}-probe.json'
    saved.write_text(json.dumps(value, indent=2))
    return value, {'file': path.name, 'bytes': path.stat().st_size, 'sha256': sha(path),
                   'probe_file': saved.name, 'probe_sha256': sha(saved)}

def audio(path):
    return run(ffmpeg, ['-v', 'error', '-nostdin', '-threads', '1', '-i', path,
                       '-map', '0:a:0', '-vn', '-ar', '48000', '-ac', '2', '-f', 'f32le', 'pipe:1'])

try:
    if offset.exists():
        raise RuntimeError('Refuse to replace retained offset fixture; choose a fresh output directory.')
    run(ffmpeg, ['-v', 'error', '-nostdin', '-copyts', '-itsoffset', '3', '-i', original,
                 '-itsoffset', '3.2', '-i', original, '-map', '0:v:0', '-map', '1:a:0',
                 '-c', 'copy', '-fps_mode', 'passthrough', '-avoid_negative_ts', 'disabled', offset])
    ordinary, ordinary_meta = probe(original)
    moved, offset_meta = probe(offset)
    fractional, mixed_meta = probe(mixed)
    assert len([p for p in ordinary['packets'] if p['stream_index'] == 0]) == 480
    shifts = {}
    for stream, expected in [(0, Fraction(3)), (1, Fraction(16, 5))]:
        a = [p for p in ordinary['packets'] if p['stream_index'] == stream]
        b = [p for p in moved['packets'] if p['stream_index'] == stream]
        assert len(a) == len(b)
        assert all(x['data_hash'] == y['data_hash'] for x, y in zip(a, b))
        delta = {Fraction(y['pts_time']) - Fraction(x['pts_time']) for x, y in zip(a, b)}
        assert delta == {expected}, (stream, delta)
        shifts[str(stream)] = {'packets': len(a), 'encoded_payloads_exact': True,
                              'pts_shift': {'num': expected.numerator, 'den': expected.denominator},
                              'first_pts': b[0]['pts_time'], 'last_pts': b[-1]['pts_time']}
    original_pcm, offset_pcm = audio(original), audio(offset)
    assert original_pcm == offset_pcm and len(original_pcm) == 16 * 48000 * 8
    values = np.frombuffer(original_pcm, '<f4').reshape(-1, 2)
    indices = np.flatnonzero(np.max(np.abs(values), axis=1) > 0.001)
    pulses = np.split(indices, np.flatnonzero(np.diff(indices) > 128) + 1)
    pulse_bounds = [[int(p[0]), int(p[-1]) + 1] for p in pulses if len(p)]
    assert len(pulse_bounds) == 16
    report.update({'original': ordinary_meta, 'offset': offset_meta, 'mixed': mixed_meta,
                   'stream_shifts': shifts, 'common_origin': {'num': 3, 'den': 1},
                   'audio_relative_start': {'num': 1, 'den': 5},
                   'visual_source_span': {'num': 16, 'den': 1}, 'visual_source_frames': 480,
                   'decoded_pcm_exact': True, 'decoded_pcm_sha256': hashlib.sha256(original_pcm).hexdigest(),
                   'pcm_sample_frames': len(values), 'pcm_peak': float(np.max(np.abs(values))),
                   'pulse_payload_sample_bounds': pulse_bounds,
                   'mixed_source_video_packets': len([p for p in fractional['packets'] if p['stream_index'] == 0]),
                   'mixed_source_fps': fractional['streams'][0]['avg_frame_rate']})
    report['after_inputs'] = {'original': sha(original), 'mixed': sha(mixed)}
    assert report['after_inputs'] == report['before_inputs']
    report['verified_fixtures'] = True
finally:
    report_path.write_text(json.dumps(report, indent=2))
print(json.dumps({k: report[k] for k in ['verified_fixtures', 'stream_shifts', 'pcm_sample_frames', 'pcm_peak', 'mixed_source_fps']}))
