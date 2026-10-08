# SPDX-License-Identifier: GPL-3.0-or-later
"""Silent production-driver retime/save/reopen and actual-media reference gate.

All raw captures remain outside the published checkout. No audio device or UI
is opened. Timings here are correctness diagnostics, never playback benchmarks.
"""
import argparse, copy, hashlib, json, math, os, pathlib, queue, re, shutil, subprocess, sys, threading, time
from fractions import Fraction
import numpy as np

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--repo', required=True, type=pathlib.Path)
parser.add_argument('--driver', required=True, type=pathlib.Path)
parser.add_argument('--project', required=True, type=pathlib.Path)
parser.add_argument('--offset-source', required=True, type=pathlib.Path)
parser.add_argument('--output', required=True, type=pathlib.Path)
parser.add_argument('--cases', help='Comma-separated labels; omitted means the full matrix')
parser.add_argument('--mode', choices=['originals', 'proxies'], default='originals')
parser.add_argument('--ffmpeg-source', type=pathlib.Path, help='Optional exact bundled FFmpeg source directory for oracle provenance')
args = parser.parse_args()
repo, out = args.repo.resolve(), args.output.resolve()
if out.is_relative_to(repo):
    parser.error('Raw captures, media and local paths must stay outside the checkout')
out.mkdir(parents=True, exist_ok=True)
cache = out / 'cache'
cache.mkdir(exist_ok=True)
resources = repo / 'src-tauri/resources/media'
ffmpeg, ffprobe, font = resources / 'ffmpeg.exe', resources / 'ffprobe.exe', resources / 'Inter.ttf'
driver_path = args.driver.resolve()
sha = lambda p: hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
digest = lambda b: hashlib.sha256(b).hexdigest()
rat = lambda x: Fraction(x['num'], x['den'])
rv = lambda x: {'num': Fraction(x).numerator, 'den': Fraction(x).denominator}
sample = lambda frame, fps: math.ceil(Fraction(frame) / fps * 48000)

def nearest(x):
    x = Fraction(x)
    return (2 * x.numerator + x.denominator) // (2 * x.denominator) if x >= 0 else -nearest(-x)

source_names = ['src-tauri/src/model.rs', 'src-tauri/src/edit.rs', 'src-tauri/src/render.rs',
                'src-tauri/src/preview.rs', 'src-tauri/src/storage.rs', 'src-tauri/src/media.rs',
                'src-tauri/src/lib.rs', 'src-tauri/build.rs',
                'src-tauri/src/audio_clock.rs', 'src-tauri/src/video_seek.rs', 'src-tauri/src/jobs.rs',
                'src-tauri/src/processes.rs', 'src-tauri/examples/preview_region_driver.rs',
                'src-tauri/tests/support/stage4b.rs', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']
sources = lambda: {n: sha(repo / n) for n in source_names}
original_project = json.loads(args.project.read_text())
source_paths = {m['id']: pathlib.Path(m['path']) if pathlib.Path(m['path']).is_absolute()
                else args.project.resolve().parent / m['path'] for m in original_project['media']}
inputs = lambda: {'project': sha(args.project), 'offset': sha(args.offset_source),
                  'originals': {k: sha(p) for k, p in source_paths.items()}}
report = {'stage': 5, 'version': 1, 'test_source_version': 5, 'mode': args.mode, 'helper_sha256': sha(__file__),
          'native_ui_or_audio_launched': False, 'sources_before': sources(), 'inputs_before': inputs(),
          'tools': {p.name: sha(p) for p in [driver_path, ffmpeg, ffprobe, font]},
          'python_executable_sha256': sha(sys.executable), 'numpy_version': np.__version__,
          'tolerances': {'gray_all_and_per_frame_rms': 5, 'selected_rgb_marker_rms': 6,
                         'ordinary_aac_vs_full_flac_rms': 0.003, 'lossless_rgb': 'exact bytes',
                         'bounded_vs_full_float_pcm': 'exact bits', 'matching_aac': 'exact bits'},
          'known_separate_limitations': ['Retained sharp-pulse AAC versus FLAC .003 fidelity failure is unchanged.',
              'The separate legacy split diagnostic with two one-ULP float differences is not evidence from this fixture.'],
          'rpc': [], 'file_tools': [], 'cases': [], 'failures': [], 'verified_content': False}
report_path = out / 'retiming-results.json'
if args.ffmpeg_source:
    report['oracle_ffmpeg_source_hashes'] = {n: sha(args.ffmpeg_source / n) for n in
        ['libavfilter/vf_fps.c', 'libavfilter/settb.c', 'libavfilter/setpts.c', 'libavfilter/filters.h']}
save = lambda: report_path.write_text(json.dumps(report, indent=2))
driver = None
messages, children, serial = queue.Queue(), [], 0

def read_stdout(pipe):
    for line in pipe:
        try: messages.put(json.loads(line))
        except Exception: messages.put({'parse_error': line})
    messages.put({'eof': True})

def rpc(op, **kw):
    global serial
    serial += 1
    request = {'id': serial, 'op': op, **kw}
    driver.stdin.write(json.dumps(request) + '\n')
    driver.stdin.flush()
    report['rpc'].append({'request': request})
    deadline = time.monotonic() + 180
    while True:
        item = messages.get(timeout=max(.001, deadline - time.monotonic()))
        if item.get('eof'): raise RuntimeError('Driver closed during RPC')
        if item.get('id') != serial: continue
        report['rpc'].append({'reply': item})
        if item.get('error'): raise RuntimeError(item['error'])
        return item.get('result')

def tool(executable, options):
    began = time.monotonic()
    child = subprocess.Popen([str(executable), *map(str, options)], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    children.append(child)
    try: data, error = child.communicate(timeout=240)
    except subprocess.TimeoutExpired:
        child.kill(); child.communicate(); raise
    report['file_tools'].append({'tool': executable.name, 'args': list(map(str, options)),
        'exit_code': child.returncode, 'stdout_bytes': len(data), 'seconds_correctness_only': time.monotonic() - began})
    if child.returncode: raise RuntimeError(error.decode(errors='replace')[-10000:])
    return data

def wait_job(job):
    deadline = time.monotonic() + 240
    while True:
        current = next(j for j in rpc('list') if j['id'] == job['id'])
        if current['status'] == 'complete': return current
        if current['status'] in ['failed', 'error', 'canceled', 'cancelled']: raise RuntimeError(json.dumps(current))
        if time.monotonic() > deadline: raise RuntimeError('Native job deadline exceeded')
        time.sleep(.05)

def write_project(label, project):
    path = out / (label + '.monocut')
    path.write_text(json.dumps(project, indent=2))
    return path

def base_project(label, source='offset', fps=Fraction(30), duration=480, source_in=Fraction(0), envelopes=False):
    p = copy.deepcopy(original_project)
    selected = 1 if source == 'mixed' else 0
    m = copy.deepcopy(p['media'][selected])
    m['path'] = str(args.offset_source.resolve() if source == 'offset' else source_paths[m['id']].resolve())
    if source == 'offset':
        m['id'] = 'stage5-offset'; m['name'] = args.offset_source.name
        m['duration'] = rv(Fraction(81, 5)); m['timing'] = None
    m['proxy'] = None; m['thumbnail'] = None; m['waveform'] = []
    m.pop('proxy_timing_version', None)
    p.update(id='stage5-' + label, name='Retime actual media ' + label, fps=rv(fps), media=[m],
             bins=[], markers=[], in_point=None, out_point=None)
    p['tracks'] = [copy.deepcopy(t) for t in p['tracks'] if t['id'] in ['V1', 'A1']]
    clips = []
    for track, volume in [('V1', 0), ('A1', .55)]:
        c = copy.deepcopy(next(c for c in original_project['clips'] if c['track_id'] == track))
        for name in ['fade_in_start', 'fade_out_end', 'composition', 'render_offset', 'retime']: c.pop(name, None)
        c.update(id=track + '-retime', media_id=m['id'], linked_id='stage5-linked', start=0,
                 duration=duration, source_in=rv(source_in), speed=rv(1), fade_in=90 if envelopes else 0,
                 fade_out=120 if envelopes else 0, keyframes=[], volume=volume, opacity=1,
                 brightness=0, contrast=1, saturation=1)
        c['transform'].update(x=0, y=0, scale=1, rotation=0, crop_top=0, crop_right=0, crop_bottom=0, crop_left=0)
        if envelopes and track == 'A1':
            c['keyframes'] = [{'property': 'volume', 'frame': f, 'value': v} for f, v in [(0,.25),(210,.75),(479,.5)]]
        clips.append(c)
    p['clips'] = clips
    return p

def plan(label, region, proxies):
    p = rpc('plan', height=540, useProxies=proxies, region=region)
    graph_path = pathlib.Path(p['args'][p['args'].index('-filter_complex_script') + 1])
    graph_path.write_text(p['filter_graph'])
    retained = out / (label + '-filter.txt')
    retained.write_text(p['filter_graph'])
    p['retained_graph_sha256'] = sha(retained)
    return p

def lossless_args(p, path, audio_codec):
    a = p['args'].copy()
    for flag in ['-preset', '-crf', '-b:a', '-movflags', '-g', '-keyint_min', '-sc_threshold']:
        while flag in a:
            at = a.index(flag); del a[at:at + 2]
    a[a.index('-c:v') + 1] = 'ffv1'; a[a.index('-c:a') + 1] = audio_codec
    a[-1] = str(path); a[-1:-1] = ['-level', '3', '-coder', '1', '-context', '1']
    return a

def audio(path, first, last):
    data = tool(ffmpeg, ['-v', 'error', '-nostdin', '-threads', '1', '-i', path, '-map', '0:a:0', '-vn',
          '-af', f'atrim=start_sample={first}:end_sample={last}', '-ar', '48000', '-ac', '2', '-f', 'f32le', 'pipe:1'])
    assert len(data) == (last - first) * 8, ('audio coverage', len(data), last - first)
    return data

def video(path, first, last, fmt='rgb24', roi=False):
    vf = f'trim=start_frame={first}:end_frame={last}' + (',crop=260:75:0:0:exact=1' if roi else '')
    data = tool(ffmpeg, ['-v', 'error', '-nostdin', '-threads', '1', '-i', path, '-map', '0:v:0', '-an',
          '-vf', vf, '-fps_mode', 'passthrough', '-threads', '1', '-f', 'rawvideo', '-pix_fmt', fmt, 'pipe:1'])
    size = (260 * 75 if roi else 960 * 540) * (3 if fmt == 'rgb24' else 1)
    assert len(data) == (last - first) * size, ('video coverage', len(data), (last - first) * size)
    return data

def streamed_rgb(path, first, last):
    options=['-v','error','-nostdin','-threads','1','-i',path,'-map','0:v:0','-an','-vf',
        f'trim=start_frame={first}:end_frame={last}','-fps_mode','passthrough','-threads','1',
        '-f','rawvideo','-pix_fmt','rgb24','pipe:1']
    began=time.monotonic();child=subprocess.Popen([str(ffmpeg),*map(str,options)],stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    children.append(child);chunks=queue.Queue(maxsize=4);errors=[]
    def read_chunks():
        for chunk in iter(lambda:child.stdout.read(1024*1024),b''):chunks.put(chunk)
        chunks.put(None)
    threading.Thread(target=read_chunks,daemon=True).start()
    threading.Thread(target=lambda:errors.append(child.stderr.read()),daemon=True).start()
    h=hashlib.sha256();size=0;deadline=time.monotonic()+240
    while True:
        chunk=chunks.get(timeout=max(.001,deadline-time.monotonic()))
        if chunk is None:break
        h.update(chunk);size+=len(chunk)
    code=child.wait(timeout=max(.001,deadline-time.monotonic()))
    report['file_tools'].append({'tool':ffmpeg.name,'args':list(map(str,options)),'exit_code':code,
        'stdout_bytes':size,'streaming_rgb_sha256':h.hexdigest(),'maximum_queue_bytes':4*1024*1024,
        'seconds_correctness_only':time.monotonic()-began})
    assert code==0, b''.join(errors).decode(errors='replace')
    assert size==(last-first)*960*540*3, ('streamed RGB coverage',size,last-first)
    return {'sha256':h.hexdigest(),'bytes':size,'frames':last-first}

def metrics(a, b, dtype):
    x, y = np.frombuffer(a, dtype=dtype).astype(np.float64), np.frombuffer(b, dtype=dtype).astype(np.float64)
    assert x.shape == y.shape
    d = x - y
    return {'rms': float(np.sqrt(np.mean(d*d))), 'max': float(np.max(np.abs(d))), 'values': len(x)}

def profile(path):
    data = path.read_bytes(); at = data.index(b'options: ') + len(b'options: '); stop = data.index(b'\0', at)
    v = dict(x.split('=', 1) for x in data[at:stop].decode().split() if '=' in x)
    result = {'rc': v['rc'], 'crf': float(v['crf']), 'keyint': int(v['keyint']),
              'effective_keyint_min': int(v['keyint_min']), 'scenecut': int(v['scenecut'])}
    assert result == {'rc': 'crf', 'crf': 20., 'keyint': 15, 'effective_keyint_min': 8, 'scenecut': 0}
    return result

templates = {}
def marker_oracle(source, sfps, fps, clip, rendered, compiled):
    # Independent model of bundled FFmpeg's timestamp operations, not another
    # project compiled by the engine: nearest settb rescale, double setpts D2TS
    # truncation, nearest fps rescale, then LAST qualifying buffered input.
    if str(source) not in templates:
        data = tool(ffmpeg, ['-v','error','-nostdin','-threads','1','-i', source,'-map','0:v:0','-an',
            '-vf','scale=960:540:flags=bicubic,format=rgba,format=yuv420p,crop=260:75:0:0:exact=1',
            '-fps_mode','passthrough','-f','rawvideo','-pix_fmt','gray','pipe:1'])
        templates[str(source)] = np.frombuffer(data, np.uint8).reshape(-1, 75*260)
    frames = templates[str(source)]
    origin = rat(clip['retime']['render_source_origin'])
    speed = rat(clip['speed'])
    phase = (rat(clip['source_in']) - origin) / speed
    offset = nearest(phase * fps)
    matched = re.search(r'fps=[^,]+:round=near,settb=1/(\d+),trim=start=', compiled['filter_graph'])
    exact_ticks = int(matched.group(1)) if matched else None
    if exact_ticks:
        assert exact_ticks == math.lcm(sfps.numerator, origin.denominator) * speed.numerator
        assert origin * exact_ticks == int(origin * exact_ticks)
    ticks = []
    for k in range(len(frames)):
        if exact_ticks:
            source_tick = nearest(Fraction(k) / sfps * exact_ticks)
            retimed = (Fraction(source_tick) - origin * exact_ticks) * speed.denominator / speed.numerator
            assert retimed.denominator == 1
            pts = int(retimed)
            ticks.append(nearest(Fraction(pts, exact_ticks) * fps))
        else:
            avtb = nearest(Fraction(k) / sfps * 1000000)
            pts = math.trunc((float(avtb) - float(origin) / .000001) / float(speed))
            ticks.append(nearest(Fraction(pts, 1000000) * fps))
    expected = [max(k for k,t in enumerate(ticks) if t <= n + offset) for n in range(clip['duration'])]
    observed = np.frombuffer(video(rendered, clip['start'], clip['start'] + clip['duration'], 'gray', True), np.uint8).reshape(-1,75*260)
    max_rms, mismatches = 0., []
    for n, k in enumerate(expected):
        candidates = list(range(max(0,k-3), min(len(frames),k+4)))
        d = frames[candidates].astype(np.float64) - observed[n].astype(np.float64)
        rms = np.sqrt(np.mean(d*d, axis=1)); found = candidates[int(np.argmin(rms))]
        max_rms = max(max_rms,float(rms[candidates.index(k)]))
        if found != k: mismatches.append({'local_frame': n,'expected_source_frame': k,'nearest_template': found})
    return {'timestamp_policy': 'Exact reciprocal ticks, integer speed ratio; fps nearest; last qualifying input' if exact_ticks else 'AVTB nearest; setpts double truncation; fps nearest; last qualifying input',
            'actual_retime_tick_denominator': exact_ticks or 1000000, 'exact_retime_grid': bool(exact_ticks),
            'source_grid': rv(sfps), 'sequence_grid': rv(fps), 'render_source_origin': rv(origin),
            'final_video_phase_frames': offset, 'expected_source_frames': expected,
            'marker_max_rms': max_rms, 'mismatches': mismatches,
            'passed': not mismatches and max_rms <= 6}

def audio_oracle(media, clip, fps, full_float, end_frame):
    # A source-only clock/resampler control is independent of the compiled
    # project/mixer graph. Envelope values and source sample coordinates are
    # evaluated here from rational quantities, without normalizing amplitude.
    speed = rat(clip['speed']); origin = rat(clip['retime']['render_source_origin'])
    epoch = rat(media['timing']['origin']); sr = 48000
    rate = math.floor(sr * speed + Fraction(1,2))
    vf = f'asetpts=PTS-({epoch.numerator}/{epoch.denominator})/TB,aresample={sr}:first_pts=0,asetpts=N/SR/TB'
    vf += ',aresample=48000' if speed == 1 else f',asetrate={rate},aresample={sr}'
    vf += ',aformat=sample_fmts=fltp:channel_layouts=stereo'
    data = tool(ffmpeg,['-v','error','-nostdin','-copyts','-threads','1','-i',media['path'],
        '-map','0:a:0','-vn','-af',vf,'-ar',str(sr),'-ac','2','-f','f32le','pipe:1'])
    raw = np.frombuffer(data,'<f4').reshape(-1,2)
    s,e = sample(clip['start'],fps),sample(clip['start']+clip['duration'],fps)
    phase = (rat(clip['source_in'])-origin)/speed
    root_start = math.ceil((Fraction(clip['start'])/fps-phase)*sr)
    source_first = math.ceil(origin/speed*sr)+s-root_start
    piece = raw[source_first:source_first+e-s]
    assert len(piece)==e-s
    source_time = (np.arange(s,e,dtype=np.float64)/sr-float(Fraction(clip['start'])/fps))*float(speed)
    envelope = clip['retime']['envelope']
    keys = sorted((rat(k['time']),k['value']) for k in envelope['keyframes'] if k['property']=='volume')
    if keys:
        gain = np.interp(source_time,[float(k[0]) for k in keys],[k[1] for k in keys])
    else:
        gain = np.full(e-s,clip['volume'],dtype=np.float64)
    fade = np.ones(e-s,dtype=np.float64)
    fi,fo,fs,fe = [rat(envelope[n]) for n in ['fade_in','fade_out','fade_in_start','fade_out_end']]
    if fi: fade *= np.clip((source_time-float(fs))/float(fi),0,1)
    if fo: fade *= np.clip((float(fe)-source_time)/float(fo),0,1)
    if keys or fi or fo:
        selected = piece.astype(np.float64)*(gain*fade)[:,None]
    else:
        selected = (piece*np.float32(clip['volume'])).astype(np.float64)
    # The ordinary shared renderer already uses alimiter's default auto-level
    # at limit .97. These low-level fixtures do not trigger attenuation. Account
    # for its existing DBL auto-level multiplication; do not alter source levels.
    assert np.max(np.abs(selected)) < .97
    selected = (selected*(1/.97)).astype('<f4')
    expected = np.zeros((sample(end_frame,fps),2),dtype='<f4');expected[s:e]=selected
    observed = audio(full_float,0,len(expected));actual=np.frombuffer(observed,'<f4').reshape(-1,2)
    def pulses(v):
        idx=np.flatnonzero(np.max(np.abs(v),axis=1)>.001)
        groups=np.split(idx,np.flatnonzero(np.diff(idx)>128)+1)
        return [[int(g[0]),int(g[-1])+1] for g in groups if len(g)]
    expected_pulses,actual_pulses=pulses(expected),pulses(actual)
    lags=[]
    for lag in range(-3,4):
        a,b=(expected[-lag:],actual[:len(actual)+lag]) if lag<0 else (expected[:len(expected)-lag],actual[lag:]) if lag else (expected,actual)
        d=a.astype(np.float64)-b.astype(np.float64)
        lags.append({'sample_lag':lag,'rms':float(np.sqrt(np.mean(d*d)))})
    best=min(lags,key=lambda l:l['rms'])['sample_lag']
    probes=[]
    for begin,finish in expected_pulses:
        center=begin+int(np.argmax(np.abs(expected[begin:finish,0])))
        local=center-s
        if 0<=local<len(piece) and abs(float(piece[local,0]))>.001:
            probes.append({'global_sample':center,'source_sample':source_first+local,
                'expected_gain':float((gain*fade)[local]/.97),'observed_gain':float(actual[center,0]/piece[local,0]),
                'expected_sample':float(expected[center,0]),'observed_sample':float(actual[center,0])})
    return {'source_only_filter':vf,'source_only_float_sha256':digest(data),'source_sample_interval':[source_first,source_first+e-s],
        'expected_full_float_sha256':digest(expected.tobytes()),'observed_full_float_sha256':digest(observed),
        'independent_numeric_float_exact':expected.tobytes()==observed,
        'independent_numeric_metrics':metrics(expected.tobytes(),observed,np.float32),
        'numeric_metrics_scope':'Python independently evaluates canonical source envelopes; reported rounding is not a replacement epsilon for strict production bounded/full float bits.',
        'existing_shared_limiter_auto_level':1/.97,'source_levels_changed':False,
        'expected_pulse_sample_bounds':expected_pulses,'observed_pulse_sample_bounds':actual_pulses,
        'pulse_sample_bounds_exact':expected_pulses==actual_pulses,'fixed_lag_no_gain_or_phase_normalization':lags,
        'best_fixed_lag':best,'gain_probes':probes,'passed':best==0 and expected_pulses==actual_pulses}

proxy_source_controls = {}

def proxy_audio_oracle(media, clip, fps, full_float, end_frame, full_plan, bounded_plan):
    # A proxy is a distinct compressed source. Read its own clock and samples;
    # never align or normalize them to the program result or original footage.
    proxy = pathlib.Path(media['proxy'])
    assert str(proxy) in full_plan['args'] and str(proxy) in bounded_plan['args']
    metadata = json.loads(tool(ffprobe, ['-v','error','-show_streams','-show_format','-of','json',proxy]))
    streams = [s for s in metadata['streams'] if s['codec_type'] in ['audio','video']]
    starts = [Fraction(s['start_pts'])*Fraction(s['time_base']) for s in streams]
    epoch = min(starts)
    assert epoch == 0 and all(t == 0 for t in starts), 'Proxy must have a verified zero common origin'
    actual_media = copy.deepcopy(media)
    actual_media['path'] = str(proxy)
    actual_media['timing']['origin'] = rv(epoch)
    result = audio_oracle(actual_media, clip, fps, full_float, end_frame)
    result.update(input_kind='actual_normalized_proxy', input_sha256=sha(proxy),
                  input_probe=metadata, input_common_origin=rv(epoch), cache_limiter_applied=False)
    # Preserve the formerly required wrong-input comparison as a diagnostic.
    # AAC ringing changes threshold-crossing bounds; those are not a claimed
    # exact correspondence between original PCM and a lossy proxy.
    result['original_source_vs_proxy_program_diagnostic'] = audio_oracle(media, clip, fps, full_float, end_frame)
    key = (sha(media['path']), sha(proxy))
    if key not in proxy_source_controls:
        def normalized(path, origin):
            vf = f'asetpts=PTS-({origin.numerator}/{origin.denominator})/TB,aresample=48000:first_pts=0,asetpts=N/SR/TB,aformat=sample_fmts=fltp:channel_layouts=stereo'
            data = tool(ffmpeg, ['-v','error','-nostdin','-copyts','-threads','1','-i',path,
                '-map','0:a:0','-vn','-af',vf,'-ar','48000','-ac','2','-f','f32le','pipe:1'])
            return data, vf
        original_epoch = rat(media['timing']['origin'])
        original_data, original_filter = normalized(media['path'], original_epoch)
        proxy_data, proxy_filter = normalized(proxy, epoch)
        coverage = math.ceil(rat(media['duration'])*48000)
        original = np.frombuffer(original_data,'<f4').reshape(-1,2)
        compressed = np.frombuffer(proxy_data,'<f4').reshape(-1,2)
        original_frame_count = len(original)
        measured_audio_frames = math.ceil((rat(media['timing']['audio_end'])-original_epoch)*48000)
        assert len(original) >= measured_audio_frames and len(compressed) >= coverage
        # A mixed-rate source may end audio before its final video tick. Add
        # only that documented absent tail to the independent common clock.
        if len(original) < coverage:
            original = np.pad(original,((0,coverage-len(original)),(0,0)))
        original, compressed = original[:coverage], compressed[:coverage]
        source_offset = rat(media['timing']['audio_start'])-original_epoch
        source_offset_samples = math.ceil(source_offset*48000)
        lags = []
        for lag in sorted(set([-9600,-4800,4800,9600,*range(-3,4)])):
            a,b = (original[-lag:],compressed[:coverage+lag]) if lag<0 else (original[:coverage-lag],compressed[lag:]) if lag else (original,compressed)
            difference = a.astype(np.float64)-b.astype(np.float64)
            lags.append({'sample_lag':lag,'rms':float(np.sqrt(np.mean(difference*difference)))})
        best = min(lags,key=lambda x:x['rms'])['sample_lag']
        assert best == 0, 'Proxy source changed phase relative to the independently normalized original'
        assert not np.any(original[:source_offset_samples]), 'Original common-clock padding differs from measured stream offset'
        def pulse_bounds(v):
            indices = np.flatnonzero(np.max(np.abs(v),axis=1)>.001)
            groups = np.split(indices,np.flatnonzero(np.diff(indices)>128)+1)
            return [[int(g[0]),int(g[-1])+1] for g in groups if len(g)]
        proxy_source_controls[key] = {
            'original_sha256':key[0], 'proxy_sha256':key[1], 'coverage_stereo_samples':[0,coverage],
            'original_normalization_filter':original_filter, 'proxy_normalization_filter':proxy_filter,
            'original_full_decoded_frames':len(original_data)//8, 'proxy_full_decoded_frames':len(proxy_data)//8,
            'documented_absent_audio_tail_zero_frames':max(0,coverage-original_frame_count),
            'original_slice_float_sha256':digest(original.tobytes()), 'proxy_slice_float_sha256':digest(compressed.tobytes()),
            'unaligned_compression_metrics':metrics(original.tobytes(),compressed.tobytes(),np.float32),
            'sample_bits_exact':original.tobytes()==compressed.tobytes(),
            'original_pulse_threshold_bounds':pulse_bounds(original), 'proxy_pulse_threshold_bounds':pulse_bounds(compressed),
            'pulse_threshold_bounds_exact':pulse_bounds(original)==pulse_bounds(compressed),
            'original_measured_relative_audio_offset':rv(source_offset), 'original_offset_samples':source_offset_samples,
            'original_offset_padding_zero':True, 'proxy_common_origin':rv(epoch),
            'fixed_lag_no_gain_or_phase_normalization':lags, 'best_fixed_lag':best,
            'source_levels_changed':False, 'cache_limiter_applied':False,
            'offset_evidence_scope':'Measured original stream offset is preserved as normalized source padding; zero fixed-lag comparison includes +/-200 ms controls. AAC threshold ringing is reported separately and is not a sample-exact original/proxy claim.'}
    result['original_to_proxy_source_compression_diagnostic'] = proxy_source_controls[key]
    return result

cases = [
    {'label':'full-2x','speed':Fraction(2)}, {'label':'full-half','speed':Fraction(1,2)},
    {'label':'trim-2x','speed':Fraction(2),'trim':(90,420)},
    {'label':'trim-half','speed':Fraction(1,2),'trim':(90,420)},
    {'label':'fractional-7over6','speed':Fraction(7,6),'source_in':Fraction(7,13),'duration':420},
    {'label':'mixed-7over6','speed':Fraction(7,6),'source':'mixed','source_in':Fraction(7,13),'duration':419},
    {'label':'mixed-sequence-7over6','speed':Fraction(7,6),'source':'mixed','fps':Fraction(30000,1001),'source_in':Fraction(7,13),'duration':420},
    {'label':'envelope-trim-7over6','speed':Fraction(7,6),'trim':(31,451),'envelopes':True,'split':True},
    {'label':'envelope-return-1x','speed':Fraction(1),'via':Fraction(7,6),'trim':(31,451),'envelopes':True,'legacy':True},
]
if args.cases:
    labels = args.cases.split(',')
    assert all(any(c['label'] == l for c in cases) for l in labels), 'Unknown case label'
    cases = [c for c in cases if c['label'] in labels]

def check(record, property_name, value):
    record[property_name] = bool(value)
    if not value: record['failures'].append(property_name)

def verify_case(case):
    label = case['label']; fps = case.get('fps', Fraction(30)); speed = case['speed']
    free=shutil.disk_usage(out).free
    report.setdefault('free_space_before_cases',[]).append({'label':label,'free_bytes':free})
    assert free>=4*1024**3, 'Less than 4 GiB free; retain all prior evidence and stop fresh encodes'
    p = base_project(label, case.get('source','offset'), fps, case.get('duration',480), case.get('source_in',Fraction(0)),case.get('envelopes',False))
    base = write_project(label + '-before',p)
    current = rpc('load_project',path=str(base))
    if case.get('trim'):
        first,last = case['trim']
        current = rpc('edit',command={'type':'trim','id':'V1-retime','edge':'in','frame':first})
        current = rpc('edit',command={'type':'trim','id':'V1-retime','edge':'out','frame':last})
    before = copy.deepcopy(current)
    legacy=None
    if case.get('legacy'):
        if args.mode=='proxies':
            proxy_job=wait_job(rpc('proxy',mediaId=current['media'][0]['id']))
            current=rpc('set_proxy',mediaId=current['media'][0]['id'],path=proxy_job['path'])
        oldstart=before['clips'][0]['start'];oldend=oldstart+before['clips'][0]['duration']
        oldfirst=oldstart+before['clips'][0]['duration']//3;oldlast=min(oldend,oldfirst+150)
        lp=plan(label+'-legacy-full',None,args.mode=='proxies');lf=out/(label+'-legacy-float.mkv')
        tool(ffmpeg,lossless_args(lp,lf,'pcm_f32le'))
        lj=wait_job(rpc('render',height=540,useProxies=args.mode=='proxies',region={'start_frame':oldfirst,'end_frame':oldlast}))
        ln=out/(label+'-legacy-native.mp4');ln.write_bytes(pathlib.Path(lj['path']).read_bytes())
        legacy={'project':copy.deepcopy(before),'plan':lp,'float':lf,'native':ln,'native_job':lj}
    undo_before=copy.deepcopy(current)
    if case.get('via'):
        undo_before=rpc('edit',command={'type':'retime_clip','id':'V1-retime','speed':rv(case['via'])})
    current = rpc('edit',command={'type':'retime_clip','id':'V1-retime','speed':rv(speed)})
    record = {'label':label,'before_clips':before['clips'],'after_clips':current['clips'],'failures':[], 'verified_content':False}
    report['cases'].append(record)
    expected_span = Fraction(before['clips'][0]['duration']) / fps * rat(before['clips'][0]['speed'])
    expected_duration = math.floor(expected_span / speed * fps)
    check(record,'exact_selected_source_span',all(rat(c['retime']['source_span']) == expected_span for c in current['clips']))
    check(record,'duration_floor_correct',all(c['duration'] == expected_duration for c in current['clips']))
    check(record,'start_source_in_fixed',all(c['start'] == b['start'] and c['source_in'] == b['source_in'] for c,b in zip(current['clips'],before['clips'])))
    check(record,'linked_members_retime_together',all(rat(c['speed']) == speed for c in current['clips']))
    def captured_envelope(c):
        oldspeed=rat(c['speed'])
        return {'keyframes':[{'property':k['property'],'time':rv(Fraction(k['frame'])/fps*oldspeed),'value':k['value']} for k in c['keyframes']],
            'fade_in':rv(Fraction(c['fade_in'])/fps*oldspeed),'fade_out':rv(Fraction(c['fade_out'])/fps*oldspeed),
            'fade_in_start':rv(Fraction(c.get('fade_in_start',0))/fps*oldspeed),
            'fade_out_end':rv(Fraction(c.get('fade_out_end',c['duration']))/fps*oldspeed)}
    check(record,'canonical_envelope_captures_source_times',all(c['retime']['envelope']==captured_envelope(b) for c,b in zip(current['clips'],before['clips'])))
    check(record,'undo_real_state',rpc('undo')['clips'] == undo_before['clips'])
    check(record,'redo_real_state',rpc('redo')['clips'] == current['clips'])
    snapshot = out / (label + '-reopened.monocut'); rpc('save',path=str(snapshot))
    current2 = rpc('load_project',path=str(snapshot))
    check(record,'save_reopen_clips_exact',current2['clips'] == current['clips'])
    record['snapshot_sha256'] = sha(snapshot)
    roundtrip=[]
    for next_speed in [Fraction(7,6),Fraction(1),speed]:
        step=rpc('edit',command={'type':'retime_clip','id':'V1-retime','speed':rv(next_speed)})
        roundtrip.append({'speed':rv(next_speed),'duration':step['clips'][0]['duration'],'source_span':step['clips'][0]['retime']['source_span']})
        assert step['clips'][0]['duration']==math.floor(expected_span/next_speed*fps)
    check(record,'repeated_retime_retains_fractional_tail',step['clips']==current['clips'])
    record['repeated_retime_durations']=roundtrip
    if args.mode == 'proxies':
        job = wait_job(rpc('proxy',mediaId=current['media'][0]['id']))
        current = rpc('set_proxy',mediaId=current['media'][0]['id'],path=job['path'])
        record['proxy_job'] = job
    proxies = args.mode == 'proxies'
    start = current['clips'][0]['start']; end = start + expected_duration
    first = start + expected_duration // 3; last = min(end,first + 150)
    region = {'start_frame':first,'end_frame':last}; count = last - first
    s0,s1 = sample(first,fps),sample(last,fps); ns = s1-s0
    full = plan(label+'-full',None,proxies)
    full_flac = out / (label+'-full-flac.mkv'); fa = lossless_args(full,full_flac,'flac'); tool(ffmpeg,fa)
    float_graph = out / (label+'-full-float-filter.txt'); float_graph.write_text(full['filter_graph']+';[vout]nullsink')
    full_float = out / (label+'-full-float.mka'); ffa=fa.copy(); ffa[ffa.index('-filter_complex_script')+1]=str(float_graph)
    at=next(i for i in range(len(ffa)-1) if ffa[i:i+2]==['-map','[vout]']); del ffa[at:at+2]
    ffa[ffa.index('-c:a')+1]='pcm_f32le'; ffa[-1]=str(full_float); tool(ffmpeg,ffa)
    bounded = plan(label+'-bounded',region,proxies)
    lossless = out / (label+'-bounded-float.mkv'); la=lossless_args(bounded,lossless,'pcm_f32le'); tool(ffmpeg,la)
    native_job=wait_job(rpc('render',height=540,useProxies=proxies,region=region))
    native=pathlib.Path(native_job['path']); record['native_job']=native_job
    record['profile']=profile(native)
    record.update(region=region,global_stereo_sample_coverage=[s0,s1],local_sample_frames=ns,
                  native_sha256=sha(native),full_plan=full,full_reference_sha256=sha(full_flac),
                  full_float_sha256=sha(full_float),bounded_plan=bounded,bounded_float_sha256=sha(lossless))
    record['retained_media_bytes']={p.name:p.stat().st_size for p in [native,full_flac,full_float,lossless]}
    expected_gray,actual_gray=video(full_flac,first,last,'gray'),video(native,0,count,'gray')
    g=metrics(expected_gray,actual_gray,np.uint8)
    gx=np.frombuffer(expected_gray,np.uint8).reshape(count,-1); gy=np.frombuffer(actual_gray,np.uint8).reshape(count,-1)
    g['per_frame_max_rms']=max(float(np.sqrt(np.mean((x.astype(np.float64)-y)**2))) for x,y in zip(gx,gy))
    record['gray_all_frames']=g; check(record,'gray_5',g['rms']<=5 and g['per_frame_max_rms']<=5)
    del expected_gray,actual_gray,gx,gy
    pictures=[]
    for n in [0,count//2,count-1]:
        a,b=video(full_flac,first+n,first+n+1),video(native,n,n+1)
        rgb=metrics(a,b,np.uint8)
        roi=lambda x:np.frombuffer(x,np.uint8).reshape(540,960,3)[:75,:260,:].tobytes()
        m=metrics(roi(a),roi(b),np.uint8); pictures.append({'local_frame':n,'rgb':rgb,'marker':m})
    record['selected_rgb']=pictures;check(record,'rgb_marker_6',all(x['rgb']['rms']<=6 and x['marker']['rms']<=6 for x in pictures))
    a,b=streamed_rgb(full_flac,first,last),streamed_rgb(lossless,0,count)
    record['lossless_rgb_streams']={'full':a,'bounded':b};check(record,'lossless_all_rgb_exact',a==b)
    ordinary,native_audio=audio(full_flac,s0,s1),audio(native,0,ns)
    record['ordinary_audio']=metrics(ordinary,native_audio,np.float32)
    check(record,'ordinary_aac_003',record['ordinary_audio']['rms']<=.003)
    f,b=audio(full_float,s0,s1),audio(lossless,0,ns)
    record['float_metrics']=metrics(f,b,np.float32);record['full_float_slice_sha256']=digest(f);record['bounded_float_slice_sha256']=digest(b)
    check(record,'bounded_full_float_exact',f==b)
    matching=out/(label+'-matching-aac.m4a')
    tool(ffmpeg,['-v','error','-nostdin','-i',lossless,'-map','0:a:0','-vn','-c:a','aac','-b:a','160k','-ar','48000','-ac','2',matching])
    check(record,'matching_aac_decoded_exact',native_audio==audio(matching,0,ns))
    audio_clip=next(c for c in current['clips'] if c['track_id']=='A1')
    record['independent_audio_oracle']=proxy_audio_oracle(current['media'][0],audio_clip,fps,full_float,end,full,bounded) if proxies else audio_oracle(current['media'][0],audio_clip,fps,full_float,end)
    check(record,'independent_audio_phase_and_pulse_samples',record['independent_audio_oracle']['passed'])
    if legacy:
        a,b=streamed_rgb(legacy['float'],0,end),streamed_rgb(full_flac,0,end)
        check(record,'return_1x_legacy_all_rgb_exact',a==b)
        a,b=audio(legacy['float'],0,sample(end,fps)),audio(full_float,0,sample(end,fps))
        record['return_1x_legacy_float_metrics']=metrics(a,b,np.float32)
        check(record,'return_1x_legacy_full_float_exact',a==b)
        a,b=audio(legacy['native'],0,ns),native_audio
        check(record,'return_1x_legacy_native_aac_exact',a==b)
        record['legacy_control']={'plan':legacy['plan'],'float_sha256':sha(legacy['float']),
            'native_sha256':sha(legacy['native']),'profile':profile(legacy['native']),
            'negative_inherited_fade_start':legacy['project']['clips'][0].get('fade_in_start')}
    if not case.get('envelopes'):
        # Compare originals' real baked source frame/time markers, even when the
        # native preview uses a proxy. Codec pixel fidelity remains a separate
        # ordinary marker <=6 gate above.
        m=current['media'][0]; source=pathlib.Path(m['path']);sfps=rat(m['fps'])
        record['independent_marker_oracle']=marker_oracle(source,sfps,fps,current['clips'][0],full_flac,full)
        check(record,'independent_source_marker_mapping',record['independent_marker_oracle']['passed'])
    if case.get('split'):
        unsplit=copy.deepcopy(current); split_frame=start+expected_duration//2
        splitp=rpc('edit',command={'type':'split','ids':['V1-retime'],'frame':split_frame})
        split_snapshot=out/(label+'-split-reopened.monocut');rpc('save',path=str(split_snapshot))
        check(record,'split_saved_reopened_exact',rpc('load_project',path=str(split_snapshot))['clips']==splitp['clips'])
        sp=plan(label+'-split-full',None,proxies);split_file=out/(label+'-split-float.mkv')
        tool(ffmpeg,lossless_args(sp,split_file,'pcm_f32le'))
        a,b=audio(full_float,0,sample(end,fps)),audio(split_file,0,sample(end,fps))
        record['split_float_metrics']=metrics(a,b,np.float32)
        check(record,'canonical_split_full_float_exact',a==b)
        a,b=streamed_rgb(full_flac,0,end),streamed_rgb(split_file,0,end)
        check(record,'canonical_split_all_rgb_exact',a==b)
        record['split_metadata']=splitp['clips'];record['split_snapshot_sha256']=sha(split_snapshot)
    record['verified_content']=not record['failures'];save()
    print(json.dumps({'label':label,'mode':args.mode,'verified_content':record['verified_content'],'failures':record['failures']}),flush=True)

try:
    initial=write_project('driver-initial',base_project('driver-initial'))
    env={**os.environ,'MONO_CUT_FFMPEG':str(ffmpeg),'MONO_CUT_FFPROBE':str(ffprobe),'MONO_CUT_MEASURE_PARENT_PID':str(os.getpid())}
    stderr=(out/'driver-stderr.log').open('wb')
    driver=subprocess.Popen([str(driver_path),str(initial),str(cache),str(font)],stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,stderr=stderr,text=True,encoding='utf-8',env=env,
        creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    threading.Thread(target=read_stdout,args=(driver.stdout,),daemon=True).start()
    ready=messages.get(timeout=180);assert ready['event']=='ready';assert ready['driver_executable_sha256']==sha(driver_path)
    report['ready']=ready
    for case in cases:
        try: verify_case(case)
        except BaseException as error:
            report['failures'].append({'label':case['label'],'error':str(error) or repr(error)});save()
            print(json.dumps({'label':case['label'],'failed':str(error)}),flush=True)
    report['verified_content']=not report['failures'] and len(report['cases'])==len(cases) and all(c['verified_content'] for c in report['cases'])
except BaseException as error:
    report['failures'].append(str(error) or repr(error))
finally:
    for child in children:
        if child.poll() is None: child.kill();child.wait(timeout=10)
    if driver and driver.poll() is None:
        try:
            report['final_native_telemetry']=rpc('telemetry');rpc('assets',previewPath=None,sourcePath=None,successorPreviewPath=None)
            rpc('shutdown');driver.stdin.close();driver.wait(timeout=30)
        except Exception as error:
            report['failures'].append('ordered driver cleanup: '+str(error));driver.kill();driver.wait(timeout=10)
    report['driver_closed']=bool(driver and driver.poll() is not None)
    report['recursive_partial_files_after']=[p.relative_to(cache).as_posix() for p in cache.rglob('*') if p.is_file() and re.search(r'\.mono-(?:audio-)?part-|\.json\.tmp$',p.name)]
    report['sources_after']=sources();report['inputs_after']=inputs()
    report['sources_unchanged']=report['sources_before']==report['sources_after'];report['inputs_unchanged']=report['inputs_before']==report['inputs_after']
    report['helper_unchanged']=report['helper_sha256']==sha(__file__)
    report['tools_after']={p.name:sha(p) for p in [driver_path,ffmpeg,ffprobe,font]}
    report['tools_unchanged']=report['tools']==report['tools_after']
    report['retained_output_bytes']=sum(p.stat().st_size for p in out.rglob('*') if p.is_file())
    report['free_space_after_bytes']=shutil.disk_usage(out).free
    report['cleanup_passed']=report.get('final_native_telemetry',{}).get('active_managed_children')==0 and report['driver_closed'] and not report['recursive_partial_files_after'] and not any(c.poll() is None for c in children)
    if not all(report[k] for k in ['sources_unchanged','inputs_unchanged','helper_unchanged','tools_unchanged','cleanup_passed']):
        report['failures'].append('hash/cleanup guard failed')
    report['verified_content']=report['verified_content'] and not report['failures']
    save()
print(json.dumps({'verified_content':report['verified_content'],'failures':report['failures'],'cleanup_passed':report['cleanup_passed']}))
sys.exit(0 if report['verified_content'] else 1)
