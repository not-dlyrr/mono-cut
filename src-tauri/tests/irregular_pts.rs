//! Actual, silently decoded media for source-specific seek safety. The attempted
//! negative generator offsets below produce missing initial packet PTS, while
//! the resulting admitted source's observed common/stream origins are zero.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    preview, render,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const SAMPLE_RATE: u32 = 44_100;
// The declared Stage4B native encoder/reference/timing profile is 540p. The
// source itself remains160x90; its exact lossless comparison uses that size.
const PREVIEW_HEIGHT: u32 = 540;
fn context(dir: &Path) -> MediaContext {
    let media_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    let tool = |env: &str, name: &str| {
        std::env::var_os(env)
            .map(PathBuf::from)
            .unwrap_or(media_dir.join(name))
    };
    MediaContext {
        ffmpeg: tool(
            "MONO_CUT_FFMPEG",
            if cfg!(windows) {
                "ffmpeg.exe"
            } else {
                "ffmpeg"
            },
        ),
        ffprobe: tool(
            "MONO_CUT_FFPROBE",
            if cfg!(windows) {
                "ffprobe.exe"
            } else {
                "ffprobe"
            },
        ),
        font: tool("MONO_CUT_FONT", "Inter.ttf"),
        cache_dir: dir.join("cache"),
        processes: Default::default(),
    }
}
fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn source_hashes() -> Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut records = serde_json::Map::new();
    for entry in fs::read_dir(root.join("src")).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            records.insert(
                format!("src/{}", p.file_name().unwrap().to_string_lossy()),
                json!(hash(&p)),
            );
        }
    }
    records.insert(
        "tests/irregular_pts.rs".into(),
        json!(hash(&root.join("tests/irregular_pts.rs"))),
    );
    Value::Object(records)
}
fn write(dir: &Path, name: &str, value: &Value) {
    fs::write(dir.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn generate(ctx: &MediaContext, path: &Path, irregular: bool) {
    let (video_offset, audio_offset) = if irregular {
        ("-2.125", "-2.031")
    } else {
        ("0", "0")
    };
    media::run_ffmpeg(ctx, &[
        "-copyts".into(), "-f".into(), "lavfi".into(), "-i".into(), "testsrc2=size=160x90:rate=24:duration=12".into(),
        // Moderate, nonzero stereo cues exercise phase/sample coverage while
        // fitting the unchanged160kbps AAC comparison limit. The retained
        // independent counterexample keeps its original loud PCM unchanged.
        "-f".into(), "lavfi".into(), "-i".into(), "aevalsrc=0.02*sin(2*PI*1137*t)+if(lt(mod(t\\,0.11)\\,0.002)\\,0.012\\,0)|0.02*sin(2*PI*853*t)+if(lt(mod(t\\,0.17)\\,0.003)\\,0.012\\,0):s=96000:d=12".into(),
        "-filter_complex".into(), format!("[0:v]setpts=PTS+({video_offset})/TB[v];[1:a]asetpts=PTS+({audio_offset})/TB[a]"), "-map".into(), "[v]".into(), "-map".into(), "[a]".into(),
        "-c:v".into(), "libx264".into(), "-crf".into(), "18".into(), "-preset".into(), "veryfast".into(), "-g".into(), "900".into(), "-keyint_min".into(), "900".into(), "-sc_threshold".into(), "0".into(), "-bf".into(), "3".into(), "-threads".into(), "2".into(),
        "-c:a".into(), "pcm_f32le".into(), "-fps_mode".into(), "passthrough".into(), "-avoid_negative_ts".into(), "disabled".into(), path.to_string_lossy().into()
    ]).unwrap();
}
fn packet_info(ctx: &MediaContext, path: &Path) -> Value {
    let out = media::command(&ctx.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_packets",
            "-show_entries",
            "packet=pts,dts,flags",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn project(ctx: &MediaContext, source: &Path) -> Project {
    let mut p = Project::new(
        "Irregular packet clock".into(),
        160,
        90,
        Rational::new(24000, 1001),
    );
    p.sample_rate = SAMPLE_RATE;
    let m = media::probe(ctx, source).unwrap();
    let mid = m.id.clone();
    p.media.push(m);
    let tid = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: mid,
            track_id: tid,
            start: 37,
            source_in: Some(Rational::new(123, 997)),
            duration: Some(180),
        },
    )
    .unwrap();
    // Fixed-duration legacy renderer fixture; preserve its exact source mapping.
    p.clips[0].speed=Rational::new(17,23); p.clips[0].render_offset=None;
    p.validate().unwrap();
    p
}
fn samples(frame: i64, fps: Rational) -> usize {
    let n = frame as i128 * fps.den as i128 * SAMPLE_RATE as i128;
    ((n + fps.num as i128 - 1) / fps.num as i128) as usize
}
fn decoded(ctx: &MediaContext, path: &Path, kind: &str) -> Vec<u8> {
    let args = match kind {
        "rgb" => vec![
            "-map",
            "0:v:0",
            "-an",
            "-pix_fmt",
            "rgb24",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
        ],
        "gray" => vec![
            "-map",
            "0:v:0",
            "-an",
            "-pix_fmt",
            "gray",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
        ],
        _ => vec![
            "-map", "0:a:0", "-vn", "-ac", "2", "-ar", "44100", "-f", "f32le",
        ],
    };
    let out = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(args)
        .arg("pipe:1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn rms(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    (a.iter()
        .zip(b)
        .map(|(x, y)| (*x as f64 - *y as f64).powi(2))
        .sum::<f64>()
        / a.len() as f64)
        .sqrt()
}
fn audio_rms(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    (a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(x, y)| {
            (f32::from_le_bytes(x.try_into().unwrap()) as f64
                - f32::from_le_bytes(y.try_into().unwrap()) as f64)
                .powi(2)
        })
        .sum::<f64>()
        / (a.len() / 4) as f64)
        .sqrt()
}
fn roi(frame: &[u8], width: usize, height: usize) -> Vec<u8> {
    (0..height.min(75))
        .flat_map(|y| {
            frame[y * width * 3..y * width * 3 + width.min(260) * 3]
                .iter()
                .copied()
        })
        .collect()
}
fn encoded_profile(path: &Path, p: &Project) -> Value {
    let bytes = fs::read(path).unwrap();
    let marker = b"options: ";
    let start = bytes
        .windows(marker.len())
        .position(|v| v == marker)
        .expect("Native MP4 missing actual x264 encoding options")
        + marker.len();
    let end = bytes[start..]
        .iter()
        .position(|b| *b == 0)
        .map(|n| start + n)
        .unwrap_or((start + 4096).min(bytes.len()));
    let options = String::from_utf8_lossy(&bytes[start..end]);
    let field = |name: &str| {
        options
            .split_whitespace()
            .find_map(|f| f.strip_prefix(&format!("{name}=")))
            .expect("Actual encoder option missing")
    };
    assert_eq!(field("crf").parse::<f64>().unwrap(), 20.);
    assert_eq!(
        field("keyint").parse::<u32>().unwrap(),
        preview::encoding_profile(p).gop_frames
    );
    assert_eq!(field("scenecut"), "0");
    assert_eq!(field("rc"), "crf");
    json!({"crf":20,"gop_frames":field("keyint").parse::<u32>().unwrap(),"effective_keyint_min":field("keyint_min").parse::<u32>().unwrap(),"scene_cut_threshold":0,"rate_control":field("rc"),"width":preview::settings(p,PREVIEW_HEIGHT).unwrap().width,"height":PREVIEW_HEIGHT})
}
fn wait(manager: &JobManager, id: &str) -> Job {
    let began = Instant::now();
    loop {
        let job = manager.list().into_iter().find(|j| j.id == id).unwrap();
        if job.status != "running" {
            assert_eq!(job.status, "complete", "Native preview failed: {job:?}");
            return job;
        }
        assert!(
            began.elapsed() < Duration::from_secs(90),
            "Preview timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn settings(p: &Project, native_size: bool) -> ExportSettings {
    let mut s = if native_size {
        preview::settings(p, PREVIEW_HEIGHT).unwrap()
    } else {
        ExportSettings {
            width: 160,
            height: 90,
            fps: p.fps,
            codec: "ffv1".into(),
            crf: 0,
            audio_bitrate: 160,
            sample_rate: SAMPLE_RATE,
        }
    };
    s.codec = "ffv1".into();
    s
}
fn encode_full(ctx: &MediaContext, p: &Project, dir: &Path, native_size: bool) -> PathBuf {
    let tag = if native_size {
        "full-native-size"
    } else {
        "full"
    };
    let output = dir.join(format!("{tag}.mkv"));
    let plan = render::compile(ctx, p, &settings(p, native_size), false, &output, tag).unwrap();
    media::run_ffmpeg(ctx, &plan.args).unwrap();
    output
}
fn decoded_span(ctx: &MediaContext, plan: &render::RenderPlan, dir: &Path, tag: &str) -> Value {
    let needle = "[0:0]";
    assert!(plan.filter_graph.contains(needle));
    let graph = plan
        .filter_graph
        .replacen(needle, &format!("{needle}showinfo,"), 1);
    let graph_path = dir.join(format!("{tag}-counted-filters.txt"));
    fs::write(&graph_path, graph).unwrap();
    let mut args = plan.args.clone();
    let graph_index = args
        .iter()
        .position(|a| a == "-filter_complex_script")
        .unwrap()
        + 1;
    args[graph_index] = graph_path.to_string_lossy().into();
    let level = args.iter().position(|a| a == "-loglevel").unwrap() + 1;
    args[level] = "info".into();
    let output = dir.join(format!("{tag}-counted.mkv"));
    *args.last_mut().unwrap() = output.to_string_lossy().into();
    let result = media::command(&ctx.ffmpeg).args(&args).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::write(dir.join(format!("{tag}-decoded-input.log")), &result.stderr).unwrap();
    let log = String::from_utf8_lossy(&result.stderr);
    let pts = log
        .lines()
        .filter(|s| s.contains("showinfo") && s.contains(" n:"))
        .filter_map(|s| {
            s.split_once("pts_time:")
                .and_then(|(_, tail)| tail.split_whitespace().next())
                .and_then(|v| v.parse::<f64>().ok())
        })
        .collect::<Vec<_>>();
    assert!(!pts.is_empty());
    json!({"decoded_input_frames":pts.len(),"first_decoded_pts_seconds":pts[0],"last_decoded_pts_seconds":pts.last(),"minimum_decoded_pts_seconds":pts.iter().copied().fold(f64::INFINITY,f64::min),"maximum_decoded_pts_seconds":pts.iter().copied().fold(f64::NEG_INFINITY,f64::max),"decoded_pts_regressions":pts.windows(2).filter(|v|v[1]<v[0]).count(),"instrumented_output_sha256":hash(&output),"note":"Measured before source filters; includes reconstructed initial missing PTS/GOP preroll/read-ahead. Initial reconstructed PTS may precede later stamped packets on a discontinuous clock; frame count reports work even when final PTS alone understates it. These are observed decoding spans, not a universal hard bound."})
}
fn check(
    ctx: &MediaContext,
    p: &Project,
    dir: &Path,
    irregular: bool,
    retained_full: Option<&Path>,
) -> Value {
    let sources_before = source_hashes();
    let saved = serde_json::to_value(p).unwrap();
    let full_path = encode_full(ctx, p, dir, false);
    let full_rgb = decoded(ctx, &full_path, "rgb");
    let full_audio = decoded(ctx, &full_path, "audio");
    assert_eq!(full_rgb.len(), p.length() as usize * 160 * 90 * 3);
    assert_eq!(full_audio.len(), samples(p.length(), p.fps) * 8);
    if let Some(retained) = retained_full {
        assert_eq!(
            full_rgb,
            decoded(ctx, retained, "rgb"),
            "Corrected full renderer changed the untouched original reference"
        );
    }
    let native_full_path = encode_full(ctx, p, dir, true);
    let native_full_rgb = decoded(ctx, &native_full_path, "rgb");
    let native_full_gray = decoded(ctx, &native_full_path, "gray");
    let native_full_audio = decoded(ctx, &native_full_path, "audio");
    let ns = settings(p, true);
    let (nw, nh) = (ns.width as usize, ns.height as usize);
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let mut records = vec![];
    let mut adjoining_video = vec![];
    let mut adjoining_audio = vec![];
    let mut adjoining_native_audio = vec![];
    let mut native_audio_failures = vec![];
    for (start, end) in [
        (37, 38),
        (78, 79),
        (121, 124),
        (78, 198),
        (216, 217),
        (78, 138),
        (138, 198),
    ] {
        let region = PreviewRegion {
            start_frame: start,
            end_frame: end,
        };
        let tag = format!("region-{start}-{end}");
        let path = dir.join(format!("{tag}.mkv"));
        let began = Instant::now();
        let plan = render::compile_region(ctx, p, &settings(p, false), false, &path, &tag, &region)
            .unwrap();
        let first_input = plan.args.iter().position(|a| a == "-i").unwrap();
        let has_seek = plan.args[..first_input].iter().any(|a| a == "-ss");
        if irregular {
            assert!(
                !has_seek,
                "Unsafe irregular initial PTS still used an input-side video seek"
            );
        } else if start >= 78 {
            assert!(has_seek, "Regular source lost its fast bounded video seek");
        }
        let compile_seconds = began.elapsed().as_secs_f64();
        let plan_record = json!({"working_region":plan.working_region,"input_ranges":plan.input_ranges,"audio_prefix_fallbacks":plan.audio_prefix_fallbacks,"video_prefix_fallbacks":plan.video_prefix_fallbacks,"video_seek_checks":plan.video_seek_checks,"args":plan.args});
        media::run_ffmpeg(ctx, &plan.args).unwrap();
        let lossless_seconds = began.elapsed().as_secs_f64();
        let decoded_input_span = decoded_span(ctx, &plan, dir, &tag);
        let actual_rgb = decoded(ctx, &path, "rgb");
        let expected_rgb = &full_rgb[start as usize * 160 * 90 * 3..end as usize * 160 * 90 * 3];
        assert_eq!(
            actual_rgb, expected_rgb,
            "Irregular/regular region altered picture phase {start}..{end}"
        );
        assert!(
            actual_rgb.iter().any(|v| *v > 0),
            "Requested contributing footage became black"
        );
        let actual_audio = decoded(ctx, &path, "audio");
        let expected_audio = &full_audio[samples(start, p.fps) * 8..samples(end, p.fps) * 8];
        let pcm_rms = audio_rms(&actual_audio, expected_audio);
        assert!(
            pcm_rms <= 0.0003,
            "Lossless PCM changed phase or coverage: {pcm_rms}"
        );
        if (start, end) == (78, 138) || (start, end) == (138, 198) {
            adjoining_video.extend_from_slice(&actual_rgb);
            adjoining_audio.extend_from_slice(&actual_audio);
        }
        let began = Instant::now();
        let job = manager
            .preview_region(p.clone(), PREVIEW_HEIGHT, false, region.clone())
            .unwrap();
        let job = wait(&manager, &job.id);
        let native_seconds = began.elapsed().as_secs_f64();
        let native_path = PathBuf::from(job.path.as_ref().unwrap());
        let native_rgb = decoded(ctx, &native_path, "rgb");
        let native_gray = decoded(ctx, &native_path, "gray");
        assert_eq!(
            native_rgb.len(),
            (end - start) as usize * nw * nh * 3,
            "Native frame count"
        );
        assert!(
            native_rgb.iter().any(|v| *v > 0),
            "Native preview returned black footage"
        );
        let rgb_expected =
            &native_full_rgb[start as usize * nw * nh * 3..end as usize * nw * nh * 3];
        let gray_expected = &native_full_gray[start as usize * nw * nh..end as usize * nw * nh];
        let max_gray_rms = native_gray
            .chunks_exact(nw * nh)
            .zip(gray_expected.chunks_exact(nw * nh))
            .map(|(a, b)| rms(a, b))
            .fold(0f64, f64::max);
        assert!(
            max_gray_rms <= 5.,
            "Native gray RMS {max_gray_rms} exceeded unchanged limit5"
        );
        let mut selected_rgb = vec![];
        let mut selected_roi = vec![];
        for index in [
            0,
            (end - start - 1) as usize / 2,
            (end - start - 1) as usize,
        ] {
            let a = &native_rgb[index * nw * nh * 3..(index + 1) * nw * nh * 3];
            let b = &rgb_expected[index * nw * nh * 3..(index + 1) * nw * nh * 3];
            let rr = rms(a, b);
            let mr = rms(&roi(a, nw, nh), &roi(b, nw, nh));
            assert!(
                rr <= 6. && mr <= 6.,
                "Native RGB/marker RMS {rr}/{mr} exceeded unchanged limit6 at {start}+{index}"
            );
            selected_rgb.push(rr);
            selected_roi.push(mr);
        }
        let native_audio = decoded(ctx, &native_path, "audio");
        let required = (samples(end, p.fps) - samples(start, p.fps)) * 8;
        assert!(
            native_audio.len() >= required,
            "Native AAC omitted requested samples"
        );
        let audio_error = audio_rms(
            &native_audio[..required],
            &native_full_audio[samples(start, p.fps) * 8..samples(end, p.fps) * 8],
        );
        if audio_error > 0.003 {
            native_audio_failures.push(json!({"region":region,"rms":audio_error,"limit":0.003}));
        }
        if (start, end) == (78, 138) || (start, end) == (138, 198) {
            adjoining_native_audio.extend_from_slice(&native_audio[..required]);
        }
        let codec_control = if retained_full.is_some() {
            let control = dir.join(format!("{tag}-independent-aac.mp4"));
            let gop = preview::encoding_profile(p).gop_frames;
            media::run_ffmpeg(
                ctx,
                &[
                    "-i".into(),
                    path.to_string_lossy().into(),
                    "-map".into(),
                    "0:v:0".into(),
                    "-map".into(),
                    "0:a:0".into(),
                    "-c:v".into(),
                    "libx264".into(),
                    "-preset".into(),
                    "veryfast".into(),
                    "-crf".into(),
                    "20".into(),
                    "-g".into(),
                    gop.to_string(),
                    "-keyint_min".into(),
                    gop.to_string(),
                    "-sc_threshold".into(),
                    "0".into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    "160k".into(),
                    "-ar".into(),
                    SAMPLE_RATE.to_string(),
                    control.to_string_lossy().into(),
                ],
            )
            .unwrap();
            let control_audio = decoded(ctx, &control, "audio");
            assert!(control_audio.len() >= required);
            let native_control_error =
                audio_rms(&native_audio[..required], &control_audio[..required]);
            json!({"sha256":hash(&control),"native_vs_independent_aac_rms":native_control_error,"independent_aac_vs_full_flac_rms":audio_rms(&control_audio[..required],expected_audio),"interpretation":"The original.003AAC-vs-FLAC gate remains separately reported. An exact encoder-only match isolates compression for that row; nonzero residual remains unclassified and does not claim a native audio pass."})
        } else {
            Value::Null
        };
        records.push(json!({"region":region,"lossless_rgb_exact":true,"lossless_audio_rms":pcm_rms,"stereo_sample_frames":required/8,"native_gray_max_rms":max_gray_rms,"native_selected_rgb_rms":selected_rgb,"native_selected_marker_rms":selected_roi,"native_audio_rms":audio_error,"native_audio_gate_passed":audio_error<=0.003,"native_codec_control":codec_control,"encoded_profile":encoded_profile(&native_path,p),"lossless_sha256":hash(&path),"native_sha256":hash(&native_path),"compile_seconds":compile_seconds,"lossless_seconds":lossless_seconds,"native_seconds":native_seconds,"decoded_input_span":decoded_input_span,"plan":plan_record}));
    }
    assert_eq!(
        adjoining_video,
        &full_rgb[78 * 160 * 90 * 3..198 * 160 * 90 * 3]
    );
    let reference_join = &full_audio[samples(78, p.fps) * 8..samples(198, p.fps) * 8];
    let join_error = audio_rms(&adjoining_audio, reference_join);
    assert!(join_error <= 0.0003);
    let native_join_error = audio_rms(
        &adjoining_native_audio,
        &native_full_audio[samples(78, p.fps) * 8..samples(198, p.fps) * 8],
    );
    assert_eq!(
        saved,
        serde_json::to_value(p).unwrap(),
        "Rendering changed saved clip/export coordinates"
    );
    manager.shutdown();
    assert_eq!(ctx.processes.active_count(), 0);
    let sources_after = source_hashes();
    assert_eq!(
        sources_before, sources_after,
        "Source changed during reference gate"
    );
    json!({"source_sha256":hash(Path::new(&p.media[0].path)),"observed_timing":p.media[0].timing,"project_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(p).unwrap())),"full_sha256":hash(&full_path),"full_native_size_sha256":hash(&native_full_path),"saved_coordinates_unchanged":true,"adjoining_frames":120,"adjoining_stereo_samples":adjoining_audio.len()/8,"adjoining_rgb_exact":true,"adjoining_audio_rms":join_error,"adjoining_native_audio_rms":native_join_error,"native_audio_limit":0.003,"native_audio_gate_passed":native_audio_failures.is_empty()&&native_join_error<=0.003,"native_audio_gate_failures":native_audio_failures,"source_hashes_before":sources_before,"source_hashes_after":sources_after,"records":records,"active_children_after":ctx.processes.active_count(),"verification":"Silent file decode/native job helper only; no native display or speaker output."})
}
fn check_normalized_proxy(ctx: &MediaContext, p: &Project, dir: &Path) -> Value {
    let sources_before = source_hashes();
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let began = Instant::now();
    let proxy = manager.proxy(p.media[0].clone()).unwrap();
    let proxy = wait(&manager, &proxy.id);
    let preparation_seconds = began.elapsed().as_secs_f64();
    let proxy_path = PathBuf::from(proxy.path.unwrap());
    let mut pp = p.clone();
    pp.media[0].proxy = Some(proxy_path.to_string_lossy().into());
    pp.media[0].proxy_timing_version = Some(NORMALIZED_TIMING_VERSION);
    assert!(media::proxy_is_current(&pp.media[0]));
    let region = PreviewRegion {
        start_frame: 121,
        end_frame: 124,
    };
    let full_path = dir.join("proxy-full.mkv");
    let full_plan = render::compile(
        ctx,
        &pp,
        &settings(&pp, false),
        true,
        &full_path,
        "proxy-full",
    )
    .unwrap();
    media::run_ffmpeg(ctx, &full_plan.args).unwrap();
    let full = decoded(ctx, &full_path, "rgb");
    let path = dir.join("proxy-region.mkv");
    let plan = render::compile_region(
        ctx,
        &pp,
        &settings(&pp, false),
        true,
        &path,
        "proxy-region",
        &region,
    )
    .unwrap();
    assert!(
        plan.video_prefix_fallbacks.is_empty(),
        "Normalized proxy incorrectly inherited original unsafe-clock policy"
    );
    assert!(plan.video_seek_checks[0].using_proxy);
    assert_eq!(
        plan.video_seek_checks[0].inspection.clock,
        mono_cut_lib::video_seek::InitialClock::SampledPtsPresent
    );
    assert_eq!(plan.video_seek_checks[0].inspection.missing_pts, 0);
    let first_input = plan.args.iter().position(|a| a == "-i").unwrap();
    assert!(plan.args[..first_input].iter().any(|a| a == "-ss"));
    media::run_ffmpeg(ctx, &plan.args).unwrap();
    assert_eq!(
        decoded(ctx, &path, "rgb"),
        full[121 * 160 * 90 * 3..124 * 160 * 90 * 3]
    );
    let original = render::compile_region(
        ctx,
        &pp,
        &settings(&pp, false),
        false,
        &dir.join("original-disabled-proxy.mkv"),
        "original-disabled-proxy",
        &region,
    )
    .unwrap();
    assert_eq!(original.video_prefix_fallbacks.len(), 1);
    assert!(!original.video_seek_checks[0].using_proxy);
    assert_ne!(
        original.video_seek_checks[0].inspection.identity_key,
        plan.video_seek_checks[0].inspection.identity_key
    );
    let native_full_path = dir.join("proxy-full-native-size.mkv");
    let native_full = render::compile(
        ctx,
        &pp,
        &settings(&pp, true),
        true,
        &native_full_path,
        "proxy-native-full",
    )
    .unwrap();
    media::run_ffmpeg(ctx, &native_full.args).unwrap();
    let reference = decoded(ctx, &native_full_path, "rgb");
    let native = manager
        .preview_region(pp.clone(), PREVIEW_HEIGHT, true, region.clone())
        .unwrap();
    let native = wait(&manager, &native.id);
    let native_path = PathBuf::from(native.path.unwrap());
    let actual = decoded(ctx, &native_path, "rgb");
    let size = preview::settings(&pp, PREVIEW_HEIGHT).unwrap();
    let bytes = (size.width * size.height * 3) as usize;
    assert_eq!(actual.len(), 3 * bytes);
    let errors = (0..3)
        .map(|i| {
            rms(
                &actual[i * bytes..(i + 1) * bytes],
                &reference[(121 + i) * bytes..(122 + i) * bytes],
            )
        })
        .collect::<Vec<_>>();
    assert!(errors.iter().all(|v| *v <= 6.));
    manager.shutdown();
    assert_eq!(ctx.processes.active_count(), 0);
    assert_eq!(sources_before, source_hashes());
    json!({"proxy_sha256":hash(&proxy_path),"proxy_preparation_seconds":preparation_seconds,"source_sha256":hash(Path::new(&p.media[0].path)),"proxy_video_seek_checks":plan.video_seek_checks,"original_with_proxy_disabled_checks":original.video_seek_checks,"original_with_proxy_disabled_fallbacks":original.video_prefix_fallbacks,"proxy_prefix_fallbacks":plan.video_prefix_fallbacks,"lossless_rgb_exact":true,"native_rgb_rms":errors,"native_sha256":hash(&native_path),"encoded_profile":encoded_profile(&native_path,p),"region":region,"active_children_after":0})
}
#[test]
fn generated_initial_missing_pts_and_regular_clock_keep_full_picture_phase() {
    let temp = TempDir::new().unwrap();
    let external = std::env::var_os("MONO_CUT_IRREGULAR_GENERATED_EVIDENCE").map(PathBuf::from);
    let root = external.as_deref().unwrap_or(temp.path());
    fs::create_dir_all(root).unwrap();
    for irregular in [true, false] {
        let dir = root.join(if irregular { "irregular" } else { "regular" });
        fs::create_dir_all(&dir).unwrap();
        let ctx = context(&dir);
        let source = dir.join("source.mkv");
        generate(&ctx, &source, irregular);
        let packets = packet_info(&ctx, &source);
        let packets = packets["packets"].as_array().unwrap();
        assert_eq!(packets.len(), 288);
        let missing = packets
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.get("pts").is_none().then_some(i))
            .collect::<Vec<_>>();
        if irregular {
            assert_eq!(missing, (0..50).chain([52]).collect::<Vec<_>>());
        } else {
            assert!(missing.is_empty());
        }
        let p = project(&ctx, &source);
        let timing = p.media[0].timing.as_ref().unwrap();
        assert_eq!(timing.origin, Rational::zero());
        assert_eq!(timing.video_start, Some(Rational::zero()));
        assert_eq!(timing.audio_start, Some(Rational::zero()));
        let mut result = check(&ctx, &p, &dir, irregular, None);
        if irregular {
            // `check` exercises shutdown ownership, so use a new registry when
            // opening another helper session against the same stamped caches.
            let proxy_ctx = context(&dir);
            result["normalized_proxy_switch"] = check_normalized_proxy(&proxy_ctx, &p, &dir);
        }
        write(&dir, "results.json", &result);
        assert_eq!(
            result["native_audio_gate_passed"], true,
            "Generated moderate audio exceeded unchanged native.003 limit: {}",
            result["native_audio_gate_failures"]
        );
        eprintln!("VERIFIED generated {} source: {} packets, {} missing PTS, observed origin0; seven lossless/native region comparisons, exact adjoining frame/sample coverage, regular fast seek preserved.",if irregular{"irregular"}else{"regular"},packets.len(),missing.len());
    }
}
#[test]
#[ignore = "Retained counterexample evidence; set MONO_CUT_IRREGULAR_EVIDENCE and MONO_CUT_IRREGULAR_RETAINED outside checkout"]
fn retained_counterexample_picture_phase_and_pcm_match_untouched_full() {
    let dir = PathBuf::from(
        std::env::var_os("MONO_CUT_IRREGULAR_EVIDENCE")
            .expect("Private evidence directory required"),
    );
    fs::create_dir_all(&dir).unwrap();
    let retained = PathBuf::from(
        std::env::var_os("MONO_CUT_IRREGULAR_RETAINED")
            .expect("Preserved independent fixture directory required"),
    );
    let original_project = retained.join("negative-minimal-project.json");
    let source = retained.join("longgop-negative.mkv");
    assert_eq!(
        hash(&original_project),
        "9c9d4a0e64910d08bcb34d5a10f22588a08ae3cfef79b23c798bfeb3b65f8406"
    );
    assert_eq!(
        hash(&source),
        "66219b1d7b14c28054641b8f04fb98e70c46b52a7ab141770a70f8afe2e64ee0"
    );
    let mut p: Project = serde_json::from_slice(&fs::read(&original_project).unwrap()).unwrap();
    p.media[0].path = source.to_string_lossy().into();
    p.validate().unwrap();
    let ctx = context(&dir);
    let before = Instant::now();
    let result = check(
        &ctx,
        &p,
        &dir,
        true,
        Some(&retained.join("minimal-neutral-full.mkv")),
    );
    let result = json!({"untouched_source_sha256":hash(&source),"untouched_project_sha256":hash(&original_project),"total_seconds":before.elapsed().as_secs_f64(),"results":result});
    write(&dir, "retained-counterexample-results.json", &result);
    eprintln!(
        "VERIFIED untouched counterexample against its original full reference: {}",
        result
    );
}
