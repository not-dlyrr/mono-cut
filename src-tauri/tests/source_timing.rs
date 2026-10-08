//! Real decoded-media regressions for the shared source presentation-time origin.
//! Fixtures, projects and output files are temporary; these tests never play audio.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    render,
    storage::ProjectStore,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const WIDTH: usize = 160;
const HEIGHT: usize = 90;
const SAMPLE_RATE: i64 = 48_000;
// AAC priming is decoded by FFmpeg; the remaining transient/ringing tolerance is
// 256 samples (5.33 ms), far below the 9,600-sample regression being tested.
const AAC_CUE_TOLERANCE: i64 = 256;

fn context(dir: &Path) -> MediaContext {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    MediaContext {
        ffmpeg: std::env::var_os("MONO_CUT_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffmpeg")),
        ffprobe: std::env::var_os("MONO_CUT_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("ffprobe")),
        cache_dir: dir.join("cache"),
        font: std::env::var_os("MONO_CUT_FONT")
            .map(PathBuf::from)
            .unwrap_or(resources.join("Inter.ttf")),
        processes: Default::default(),
    }
}

fn fixture(ctx: &MediaContext, path: &Path, video_start: Rational, audio_start: Rational) {
    let video = format!(
        "[0:v]geq=lum='40+mod(N,30)*5':cb=128:cr=128,setpts=PTS+({}/{})/TB[v]",
        video_start.num, video_start.den
    );
    let audio = format!(
        "[1:a]asetpts=PTS+({}/{})/TB[a]",
        audio_start.num, audio_start.den
    );
    media::run_ffmpeg(
        ctx,
        &vec![
            "-copyts".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=c=black:size=160x90:rate=30:duration=2".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "aevalsrc=if(between(t\\,0.5\\,0.52)\\,0.6\\,0):s=48000:d=2".into(),
            "-filter_complex".into(),
            format!("{video};{audio}"),
            "-map".into(),
            "[v]".into(),
            "-map".into(),
            "[a]".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            "-avoid_negative_ts".into(),
            "disabled".into(),
            "-map_metadata".into(),
            "-1".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
    .expect("Synthetic timing fixtures require FFmpeg with FFV1 and PCM support");
}

fn wait(manager: &JobManager, id: &str) -> Job {
    let start = Instant::now();
    loop {
        let job = manager.list().into_iter().find(|job| job.id == id).unwrap();
        if job.status != "running" {
            assert_eq!(job.status, "complete", "Media job failed: {job:?}");
            return job;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "Job timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn settings(codec: &str) -> ExportSettings {
    ExportSettings {
        width: WIDTH as u32,
        height: HEIGHT as u32,
        fps: Rational::new(30, 1),
        codec: codec.into(),
        crf: 18,
        audio_bitrate: 192,
        sample_rate: SAMPLE_RATE as u32,
    }
}

fn decoded(ctx: &MediaContext, path: &Path, args: &[String]) -> Vec<u8> {
    let output = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-copyts", "-i"])
        .arg(path)
        .args(args)
        .arg("pipe:1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn pcm(ctx: &MediaContext, path: &Path, origin: Rational) -> Vec<f32> {
    // Raw PCM cannot represent a late stream's initial PTS. Explicitly placing
    // decoded samples on the independently supplied source clock exposes that gap.
    let args = vec![
        "-map".into(),
        "0:a:0".into(),
        "-af".into(),
        format!(
            "asetpts=PTS-({}/{})/TB,aresample=48000:first_pts=0",
            origin.num, origin.den
        ),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        SAMPLE_RATE.to_string(),
        "-f".into(),
        "f32le".into(),
    ];
    decoded(ctx, path, &args)
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}

fn cue(samples: &[f32]) -> i64 {
    samples
        .iter()
        .position(|sample| sample.abs() > 0.2)
        .expect("Decoded audio pulse is missing") as i64
}

fn assert_cue(samples: &[f32], expected: i64, tolerance: i64, label: &str) -> i64 {
    let actual = cue(samples);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: expected pulse sample {expected}, decoded {actual} (tolerance {tolerance})"
    );
    actual
}

fn frame_codes(ctx: &MediaContext, path: &Path) -> Vec<f64> {
    let args = vec![
        "-map".into(),
        "0:v:0".into(),
        "-vf".into(),
        "scale=160:90".into(),
        "-fps_mode".into(),
        "passthrough".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
    ];
    let bytes = decoded(ctx, path, &args);
    assert_eq!(bytes.len() % (WIDTH * HEIGHT * 3), 0);
    bytes
        .chunks_exact(WIDTH * HEIGHT * 3)
        .map(|frame| {
            let mut sum = 0.;
            for y in 30..50 {
                for x in 60..80 {
                    sum += frame[(y * WIDTH + x) * 3] as f64;
                }
            }
            sum / 400.
        })
        .collect()
}

fn assert_frames(
    codes: &[f64],
    original: &[f64],
    leading_frames: usize,
    total: usize,
    label: &str,
) {
    assert_eq!(codes.len(), total, "{label}: output frame count");
    assert!(
        codes[..leading_frames].iter().all(|code| *code < 3.),
        "{label}: video appeared before its shared source start"
    );
    for (frame, expected) in original.iter().enumerate() {
        let actual = codes[leading_frames + frame];
        assert!(
            (actual - expected).abs() <= 4.,
            "{label}: source frame {frame} moved or changed: expected {expected:.2}, decoded {actual:.2}"
        );
    }
    // The renderer leaves the neutral background after a shorter stream ends.
    assert!(
        codes[leading_frames + original.len()..]
            .iter()
            .all(|code| *code < 3.),
        "{label}: video extended beyond its shared source duration"
    );
}

fn project(
    ctx: &MediaContext,
    source: &Path,
    source_in: Rational,
    start: i64,
    frames: i64,
) -> Project {
    let mut p = Project::new("Source timing".into(), 160, 90, Rational::new(30, 1));
    p.media.push(media::probe(ctx, source).unwrap());
    let media_id = p.media[0].id.clone();
    let track_id = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id,
            track_id,
            start,
            source_in: Some(source_in),
            duration: Some(frames),
        },
    )
    .unwrap();
    p
}

fn export(ctx: &MediaContext, p: &Project, path: &Path, proxies: bool, codec: &str) {
    // Compile directly to exercise the same graph while allowing proxy FFV1
    // comparisons. The user-facing export API deliberately uses originals.
    let plan = render::compile(ctx, p, &settings(codec), proxies, path, &id()).unwrap();
    media::run_ffmpeg(ctx, &plan.args).unwrap();
}

#[test]
fn relative_stream_starts_survive_original_prepared_proxy_preview_and_export() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let cases = [
        ("audio-late", Rational::zero(), Rational::new(1, 5)),
        ("aligned", Rational::zero(), Rational::zero()),
        ("video-late", Rational::new(1, 5), Rational::zero()),
        ("common-nonzero", Rational::new(3, 1), Rational::new(16, 5)),
    ];
    for (name, video_start, audio_start) in cases {
        let source = dir.path().join(format!("{name}.mkv"));
        fixture(&ctx, &source, video_start, audio_start);
        let origin = if video_start.value() < audio_start.value() {
            video_start
        } else {
            audio_start
        };
        let video_offset = video_start.add(Rational::new(-origin.num, origin.den));
        let audio_offset = audio_start.add(Rational::new(-origin.num, origin.den));
        let expected_cue = audio_offset
            .add(Rational::new(1, 2))
            .frames_floor(Rational::new(SAMPLE_RATE, 1));
        let total = if video_start == audio_start { 60 } else { 66 };
        let p = project(&ctx, &source, Rational::zero(), 0, total);
        let timing = p.media[0]
            .timing
            .as_ref()
            .expect("Probe must preserve stream origins");
        assert_eq!(timing.origin, origin, "{name}: exact source origin");
        assert_eq!(
            timing.video_start,
            Some(video_start),
            "{name}: exact video start"
        );
        assert_eq!(
            timing.audio_start,
            Some(audio_start),
            "{name}: exact audio start"
        );
        assert_eq!(
            p.media[0].duration,
            Rational::from_frames(total, p.fps),
            "{name}: duration relative to shared origin"
        );
        let reference_audio = pcm(&ctx, &source, origin);
        assert_cue(
            &reference_audio,
            expected_cue,
            4,
            &format!("{name} original"),
        );
        let original_codes = frame_codes(&ctx, &source);
        assert_eq!(original_codes.len(), 60);
        let leading = video_offset.frames_floor(p.fps) as usize;
        let lossless = dir.path().join(format!("{name}-lossless.mkv"));
        export(&ctx, &p, &lossless, false, "ffv1");
        let lossless_pcm = pcm(&ctx, &lossless, Rational::zero());
        assert_eq!(
            lossless_pcm.len(),
            total as usize * 1600,
            "{name}: exact decoded sequence audio duration"
        );
        let lossless_cue = assert_cue(&lossless_pcm, expected_cue, 4, &format!("{name} FFV1/FLAC"));
        assert_frames(
            &frame_codes(&ctx, &lossless),
            &original_codes,
            leading,
            total as usize,
            &format!("{name} export"),
        );

        let stale_source = ctx.cache_dir.join(format!(
            "{}-source-v3.mp4",
            media::cache_key(&source).unwrap()
        ));
        fs::write(
            &stale_source,
            b"obsolete cache with independently reset stream timestamps",
        )
        .unwrap();
        let prepared = PathBuf::from(media::prepare_source(&ctx, &p.media[0]).unwrap());
        assert_ne!(
            prepared, stale_source,
            "Old source caches must be invalidated"
        );
        let prepared_cue = assert_cue(
            &pcm(&ctx, &prepared, Rational::zero()),
            expected_cue,
            AAC_CUE_TOLERANCE,
            &format!("{name} prepared"),
        );
        assert_frames(
            &frame_codes(&ctx, &prepared),
            &original_codes,
            leading,
            total as usize,
            &format!("{name} prepared"),
        );
        // Render the normalized prepared file as newly probed media. Its timing
        // metadata must start at zero, so a common source origin is not applied twice.
        let mut prepared_project = p.clone();
        let mut prepared_media = media::probe(&ctx, &prepared).unwrap();
        prepared_media.id = p.media[0].id.clone();
        assert_eq!(
            prepared_media.timing.as_ref().unwrap().origin,
            Rational::zero()
        );
        prepared_project.media[0] = prepared_media;
        let prepared_export = dir.path().join(format!("{name}-prepared-export.mkv"));
        export(&ctx, &prepared_project, &prepared_export, false, "ffv1");
        assert_cue(
            &pcm(&ctx, &prepared_export, Rational::zero()),
            expected_cue,
            AAC_CUE_TOLERANCE,
            &format!("{name} prepared re-export"),
        );
        assert_frames(
            &frame_codes(&ctx, &prepared_export),
            &original_codes,
            leading,
            total as usize,
            &format!("{name} prepared re-export"),
        );

        let proxy_job = manager.proxy(p.media[0].clone()).unwrap();
        let proxy_job = wait(&manager, &proxy_job.id);
        let proxy = PathBuf::from(proxy_job.path.unwrap());
        let proxy_cue = assert_cue(
            &pcm(&ctx, &proxy, Rational::zero()),
            expected_cue,
            AAC_CUE_TOLERANCE,
            &format!("{name} proxy"),
        );
        assert_frames(
            &frame_codes(&ctx, &proxy),
            &original_codes,
            leading,
            total as usize,
            &format!("{name} proxy"),
        );
        let project_file = dir.path().join(format!("{name}.monocut"));
        fs::write(&project_file, serde_json::to_vec(&p).unwrap()).unwrap();
        let mut store = ProjectStore::new(dir.path().join(format!("{name}-recovery.json")));
        store.open(&project_file).unwrap();
        let proxied = store.set_proxy(&p.media[0].id, proxy).unwrap();
        assert_eq!(proxied.media[0].proxy_timing_version, Some(1));
        let proxy_export = dir.path().join(format!("{name}-proxy-export.mkv"));
        export(&ctx, &proxied, &proxy_export, true, "ffv1");
        assert_cue(
            &pcm(&ctx, &proxy_export, Rational::zero()),
            expected_cue,
            AAC_CUE_TOLERANCE,
            &format!("{name} proxy re-export"),
        );
        assert_frames(
            &frame_codes(&ctx, &proxy_export),
            &original_codes,
            leading,
            total as usize,
            &format!("{name} proxy re-export"),
        );
        let mut preview_cues = vec![];
        for use_proxies in [false, true] {
            let job = manager.preview(proxied.clone(), 180, use_proxies).unwrap();
            let job = wait(&manager, &job.id);
            let path = PathBuf::from(job.path.unwrap());
            let actual = assert_cue(
                &pcm(&ctx, &path, Rational::zero()),
                expected_cue,
                AAC_CUE_TOLERANCE,
                &format!("{name} preview proxies={use_proxies}"),
            );
            assert_frames(
                &frame_codes(&ctx, &path),
                &original_codes,
                leading,
                total as usize,
                &format!("{name} preview proxies={use_proxies}"),
            );
            preview_cues.push(actual);
        }
        eprintln!("VERIFIED source timing {name}: original cue {} / FFV1-FLAC {lossless_cue} / prepared {prepared_cue} / proxy {proxy_cue} / previews {preview_cues:?}; {total} output frames; {leading} leading black frames; AAC tolerance {AAC_CUE_TOLERANCE} samples.", cue(&reference_audio));
    }
}

#[test]
fn trims_splits_and_existing_speed_mapping_use_the_shared_source_clock() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("offset.mkv");
    fixture(&ctx, &source, Rational::new(3, 1), Rational::new(16, 5));
    let trimmed = project(&ctx, &source, Rational::new(3, 10), 0, 45);
    let trimmed_path = dir.path().join("trimmed.mkv");
    export(&ctx, &trimmed, &trimmed_path, false, "ffv1");
    assert_cue(
        &pcm(&ctx, &trimmed_path, Rational::zero()),
        19_200,
        4,
        "0.300-second shared source trim",
    );
    let positioned = project(&ctx, &source, Rational::new(3, 10), 30, 45);
    let positioned_path = dir.path().join("positioned.mkv");
    export(&ctx, &positioned, &positioned_path, false, "ffv1");
    let positioned_pcm = pcm(&ctx, &positioned_path, Rational::zero());
    assert_eq!(
        positioned_pcm.len(),
        75 * 1600,
        "Exact positioned sequence audio duration"
    );
    assert_cue(&positioned_pcm, 67_200, 4, "Trim at timeline 1.000 seconds");
    let mut split = positioned.clone();
    let clip_id = split.clips[0].id.clone();
    // Place the split exactly at the pulse onset; either side resetting its
    // stream start would shift, duplicate or lose this cue.
    edit::apply(
        &mut split,
        EditCommand::Split {
            ids: vec![clip_id],
            frame: 42,
        },
    )
    .unwrap();
    assert_eq!(split.clips.len(), 2);
    let split_path = dir.path().join("split.mkv");
    export(&ctx, &split, &split_path, false, "ffv1");
    let split_pcm = pcm(&ctx, &split_path, Rational::zero());
    assert_cue(&split_pcm, 67_200, 4, "Split at pulse onset");
    assert_eq!(positioned_pcm.len(), split_pcm.len());
    let rms = (positioned_pcm
        .iter()
        .zip(&split_pcm)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / positioned_pcm.len() as f64)
        .sqrt();
    assert!(
        rms < 0.000_01,
        "Split changed or duplicated decoded audio: RMS {rms}"
    );
    let before_codes = frame_codes(&ctx, &positioned_path);
    let after_codes = frame_codes(&ctx, &split_path);
    assert_eq!(before_codes.len(), after_codes.len());
    assert!(
        before_codes
            .iter()
            .zip(&after_codes)
            .all(|(a, b)| (a - b).abs() <= 1.),
        "Split moved a decoded video frame"
    );
    let prepared_path = PathBuf::from(media::prepare_source(&ctx, &positioned.media[0]).unwrap());
    let mut prepared_project = positioned.clone();
    let mut prepared_media = media::probe(&ctx, &prepared_path).unwrap();
    prepared_media.id = positioned.media[0].id.clone();
    prepared_project.media[0] = prepared_media;
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let proxy_job = manager.proxy(positioned.media[0].clone()).unwrap();
    let proxy_job = wait(&manager, &proxy_job.id);
    let mut proxy_project = positioned.clone();
    proxy_project.media[0].proxy = proxy_job.path;
    proxy_project.media[0].proxy_timing_version = Some(1);
    let cached_variants = [
        ("prepared", prepared_project, false),
        ("proxy", proxy_project, true),
    ];
    for (name, cached_project, use_proxies) in &cached_variants {
        let path = dir.path().join(format!("positioned-{name}.mkv"));
        export(&ctx, cached_project, &path, *use_proxies, "ffv1");
        assert_cue(
            &pcm(&ctx, &path, Rational::zero()),
            67_200,
            AAC_CUE_TOLERANCE,
            &format!("Common nonzero origin, 0.300 trim, 1.000 timeline offset through {name}"),
        );
        let codes = frame_codes(&ctx, &path);
        assert_eq!(codes.len(), before_codes.len());
        assert!(
            codes
                .iter()
                .zip(&before_codes)
                .all(|(a, b)| (a - b).abs() <= 4.),
            "{name}: source trim changed decoded video frames"
        );
    }
    let mut fast = project(&ctx, &source, Rational::new(3, 10), 30, 45);
    let clip_id = fast.clips[0].id.clone();
    // Keep the historical fixed-duration renderer fixture independent of retiming.
    fast.clips[0].speed=Rational::new(2,1); fast.clips[0].render_offset=None;
    edit::apply(
        &mut fast,
        EditCommand::UpdateClip {
            id: clip_id,
            patch: json!({"duration":24}),
        },
    )
    .unwrap();
    let fast_path = dir.path().join("speed.mkv");
    export(&ctx, &fast, &fast_path, false, "ffv1");
    let fast_pcm = pcm(&ctx, &fast_path, Rational::zero());
    assert_eq!(
        fast_pcm.len(),
        54 * 1600,
        "Exact 2x positioned sequence audio duration"
    );
    assert_cue(&fast_pcm, 57_600, 8, "Existing 2x rate mapping");
    let fast_codes = frame_codes(&ctx, &fast_path);
    for (name, cached_project, use_proxies) in &cached_variants {
        let mut cached_fast = cached_project.clone();
        let clip_id = cached_fast.clips[0].id.clone();
        cached_fast.clips[0].speed=Rational::new(2,1); cached_fast.clips[0].render_offset=None;
        edit::apply(
            &mut cached_fast,
            EditCommand::UpdateClip {
                id: clip_id,
                patch: json!({"duration":24}),
            },
        )
        .unwrap();
        let path = dir.path().join(format!("speed-{name}.mkv"));
        export(&ctx, &cached_fast, &path, *use_proxies, "ffv1");
        let cached_pcm = pcm(&ctx, &path, Rational::zero());
        assert_eq!(
            cached_pcm.len(),
            fast_pcm.len(),
            "{name}: exact 2x decoded sequence audio duration"
        );
        assert_cue(
            &cached_pcm,
            57_600,
            AAC_CUE_TOLERANCE,
            &format!("Existing 2x rate through {name}"),
        );
        let codes = frame_codes(&ctx, &path);
        assert_eq!(codes.len(), fast_codes.len());
        for (frame, (cached, original)) in codes.iter().zip(&fast_codes).enumerate() {
            assert!((cached-original).abs() <= 4., "{name}: 2x source mapping changed decoded video at sequence frame {frame}: cached code {cached:.3}, original code {original:.3}");
        }
    }
    eprintln!("VERIFIED shared source edits with common3s origin: trim pulse sample19200; timeline offset/split/prepared/proxy pulse67200; split PCM RMS {rms:.8}; original/prepared/proxy 2x rate pulse57600; exact decoded audio durations; no decoded split, cached trim or speed frame shift.");
}

#[test]
fn old_v1_projects_reprobe_unknown_timing_and_keep_edits_through_recovery_and_relink() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("legacy-source.mkv");
    fixture(&ctx, &source, Rational::new(3, 1), Rational::new(16, 5));
    let mut p = project(&ctx, &source, Rational::new(3, 10), 30, 45);
    edit::apply(
        &mut p,
        EditCommand::AddBin {
            name: "Keep this bin".into(),
        },
    )
    .unwrap();
    let bin_id = p.bins[0].id.clone();
    let media_id = p.media[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AssignBin {
            media_id: media_id.clone(),
            bin_id: Some(bin_id.clone()),
        },
    )
    .unwrap();
    edit::apply(
        &mut p,
        EditCommand::Marker {
            frame: 42,
            name: "Keep this marker".into(),
        },
    )
    .unwrap();
    let expected_clips = p.clips.clone();
    let expected_markers = p.markers.clone();
    let legacy_path = dir.path().join("legacy.monocut");
    let mut legacy = serde_json::to_value(&p).unwrap();
    let stale_proxy = dir.path().join("old-proxy.mp4");
    fs::copy(&source, &stale_proxy).unwrap();
    legacy["media"][0]["proxy"] = json!(stale_proxy.to_string_lossy());
    legacy["media"][0].as_object_mut().unwrap().remove("timing");
    legacy["media"][0]
        .as_object_mut()
        .unwrap()
        .remove("proxy_timing_version");
    assert_eq!(legacy["version"], json!(1));
    fs::write(&legacy_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
    let legacy_recovery = dir.path().join("legacy-recovery.json");
    fs::write(
        &legacy_recovery,
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let mut legacy_recovered_store = ProjectStore::new(legacy_recovery);
    assert!(legacy_recovered_store.recovery_available());
    let legacy_recovered = legacy_recovered_store.recover_hydrated(&ctx).unwrap();
    assert_eq!(legacy_recovered.clips, expected_clips);
    assert_eq!(legacy_recovered.media[0].id, media_id);
    assert_eq!(
        legacy_recovered.media[0].timing.as_ref().unwrap().origin,
        Rational::new(3, 1)
    );
    let recovery_path = dir.path().join("recovery.json");
    let mut store = ProjectStore::new(recovery_path.clone());
    let unknown = store.open(&legacy_path).unwrap();
    assert!(unknown.media[0].timing.is_none());
    assert!(
        unknown.media[0].proxy.is_none(),
        "A legacy proxy without normalized timing metadata must be invalidated"
    );
    assert_eq!(unknown.clips, expected_clips);
    let hydrated = store.hydrate_assets(&ctx).unwrap();
    let timing = hydrated.media[0]
        .timing
        .clone()
        .expect("Existing legacy media must be reprobed");
    assert_eq!(timing.origin, Rational::new(3, 1));
    assert_eq!(timing.audio_start, Some(Rational::new(16, 5)));
    assert_eq!(hydrated.media[0].id, media_id);
    assert_eq!(hydrated.media[0].bin_id, Some(bin_id.clone()));
    assert_eq!(hydrated.clips, expected_clips);
    assert_eq!(hydrated.markers, expected_markers);
    let saved_path = dir.path().join("migrated.monocut");
    store.save(&saved_path).unwrap();
    let mut reopened = ProjectStore::new(dir.path().join("reopened-recovery.json"));
    let reopened_project = reopened.open_hydrated(&saved_path, &ctx).unwrap();
    assert_eq!(reopened_project.media[0].timing.as_ref(), Some(&timing));
    assert_eq!(reopened_project.clips, expected_clips);
    let mut recovered = ProjectStore::new(recovery_path);
    assert!(recovered.recovery_available());
    let recovered_project = recovered.recover().unwrap();
    assert_eq!(recovered_project.media[0].timing.as_ref(), Some(&timing));
    assert_eq!(recovered_project.clips, expected_clips);
    let migrated_export = dir.path().join("migrated.mkv");
    export(&ctx, &reopened_project, &migrated_export, false, "ffv1");
    assert_cue(
        &pcm(&ctx, &migrated_export, Rational::zero()),
        67_200,
        4,
        "Legacy migrated timeline",
    );

    // A missing legacy source still opens, accepts ordinary edits, saves and
    // recovers. Relinking discovers timing while retaining identifiers and trims.
    let replacement = dir.path().join("replacement.mkv");
    fs::rename(&source, &replacement).unwrap();
    let missing = store.open_hydrated(&legacy_path, &ctx).unwrap();
    assert!(missing.media[0].missing);
    assert!(missing.media[0].timing.is_none());
    assert_eq!(missing.clips, expected_clips);
    let edited_missing = store
        .apply(
            EditCommand::Rename {
                name: "Missing but editable".into(),
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(edited_missing.clips, expected_clips);
    store.save(&dir.path().join("missing.monocut")).unwrap();
    let recovered_missing = store.recover().unwrap();
    assert!(recovered_missing.media[0].missing);
    let relinked = store
        .apply(
            EditCommand::Relink {
                media_id: media_id.clone(),
                path: replacement.to_string_lossy().into_owned(),
            },
            &ctx,
        )
        .unwrap();
    assert!(!relinked.media[0].missing);
    assert_eq!(relinked.media[0].id, media_id);
    assert_eq!(relinked.media[0].bin_id, Some(bin_id));
    assert_eq!(relinked.media[0].timing.as_ref(), Some(&timing));
    assert_eq!(relinked.clips, expected_clips);
    assert_eq!(relinked.markers, expected_markers);
    store.save(&saved_path).unwrap();
    let final_reopened = reopened.open_hydrated(&saved_path, &ctx).unwrap();
    assert_eq!(final_reopened.media[0].timing.as_ref(), Some(&timing));
    assert_eq!(final_reopened.clips, expected_clips);
    eprintln!("VERIFIED additive version-1 timing migration: unknown legacy metadata reprobed; IDs/bins/markers/trims preserved across save, reopen, recovery, missing-source editing and relink; migrated decoded pulse sample 67200.");
}

#[test]
fn normalized_proxy_padding_does_not_cover_other_video_tracks() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let underlay = dir.path().join("underlay.png");
    media::run_ffmpeg(
        &ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=c=0x777777:size=160x90:rate=30".into(),
            "-frames:v".into(),
            "1".into(),
            "-update".into(),
            "1".into(),
            underlay.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    for (name, video_start, audio_start) in [
        ("leading-gap", Rational::new(1, 5), Rational::zero()),
        ("trailing-gap", Rational::zero(), Rational::new(1, 5)),
    ] {
        let source = dir.path().join(format!("{name}.mkv"));
        fixture(&ctx, &source, video_start, audio_start);
        let mut p = Project::new("Layered timing".into(), 160, 90, Rational::new(30, 1));
        p.media.push(media::probe(&ctx, &underlay).unwrap());
        p.media.push(media::probe(&ctx, &source).unwrap());
        let lower_media = p.media[0].id.clone();
        let lower_track = p.tracks[0].id.clone();
        edit::apply(
            &mut p,
            EditCommand::AddClip {
                media_id: lower_media,
                track_id: lower_track,
                start: 0,
                source_in: None,
                duration: Some(66),
            },
        )
        .unwrap();
        edit::apply(
            &mut p,
            EditCommand::AddTrack {
                kind: "video".into(),
                name: "Upper video".into(),
            },
        )
        .unwrap();
        let upper_media = p.media[1].id.clone();
        let upper_track = p.tracks.last().unwrap().id.clone();
        edit::apply(
            &mut p,
            EditCommand::AddClip {
                media_id: upper_media,
                track_id: upper_track,
                start: 0,
                source_in: None,
                duration: Some(66),
            },
        )
        .unwrap();
        let original_path = dir.path().join(format!("{name}-original.mkv"));
        export(&ctx, &p, &original_path, false, "ffv1");
        let original_codes = frame_codes(&ctx, &original_path);
        assert_eq!(original_codes.len(), 66);
        let exposed = if video_start == Rational::zero() {
            &original_codes[60..]
        } else {
            &original_codes[..6]
        };
        assert!(
            exposed.iter().all(|code| (*code - 119.).abs() <= 3.),
            "Original {name} must reveal the opaque lower track: {exposed:?}"
        );
        let job = manager.proxy(p.media[1].clone()).unwrap();
        let job = wait(&manager, &job.id);
        p.media[1].proxy = job.path;
        p.media[1].proxy_timing_version = Some(1);
        let proxy_path = dir.path().join(format!("{name}-proxied.mkv"));
        export(&ctx, &p, &proxy_path, true, "ffv1");
        let proxy_codes = frame_codes(&ctx, &proxy_path);
        assert_eq!(proxy_codes.len(), original_codes.len());
        for (frame, (original, proxy)) in original_codes.iter().zip(&proxy_codes).enumerate() {
            assert!((original-proxy).abs() <= 4., "{name}: normalized proxy padding covered lower-track content at frame {frame}; original {original:.2}, proxy {proxy:.2}");
        }
        eprintln!("VERIFIED normalized proxy layering {name}: six missing-video frames reveal lower track; all 66 original/proxy decoded frame codes agree within 4 RGB values.");
    }
}

#[test]
fn mixed_frame_rate_source_caches_preserve_the_last_audio_cue() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("mixed-rate-tail.mkv");
    media::run_ffmpeg(
        &ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=c=0x777777:size=160x90:rate=24000/1001:duration=2".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "aevalsrc=if(between(t\\,2.01\\,2.025)\\,0.6\\,0):s=48000:d=2.035".into(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            source.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    let p = project(&ctx, &source, Rational::zero(), 0, 61);
    assert_eq!(p.media[0].fps, Rational::new(24000, 1001));
    assert_cue(
        &pcm(&ctx, &source, Rational::zero()),
        96_480,
        4,
        "Mixed-rate original tail",
    );
    let original_export = dir.path().join("mixed-rate-original.mkv");
    export(&ctx, &p, &original_export, false, "ffv1");
    assert_cue(
        &pcm(&ctx, &original_export, Rational::zero()),
        96_480,
        4,
        "Mixed-rate 30fps FFV1/FLAC",
    );
    assert_eq!(frame_codes(&ctx, &original_export).len(), 61);
    let prepared = PathBuf::from(media::prepare_source(&ctx, &p.media[0]).unwrap());
    let prepared_cue = assert_cue(
        &pcm(&ctx, &prepared, Rational::zero()),
        96_480,
        AAC_CUE_TOLERANCE,
        "Mixed-rate prepared tail",
    );
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let proxy = manager.proxy(p.media[0].clone()).unwrap();
    let proxy = wait(&manager, &proxy.id);
    let proxy_path = PathBuf::from(proxy.path.clone().unwrap());
    let proxy_cue = assert_cue(
        &pcm(&ctx, &proxy_path, Rational::zero()),
        96_480,
        AAC_CUE_TOLERANCE,
        "Mixed-rate proxy tail",
    );
    let mut proxied = p.clone();
    proxied.media[0].proxy = proxy.path;
    proxied.media[0].proxy_timing_version = Some(1);
    let mut preview_cues = vec![];
    for use_proxies in [false, true] {
        let preview = manager.preview(proxied.clone(), 180, use_proxies).unwrap();
        let preview = wait(&manager, &preview.id);
        let preview_path = PathBuf::from(preview.path.unwrap());
        preview_cues.push(assert_cue(
            &pcm(&ctx, &preview_path, Rational::zero()),
            96_480,
            AAC_CUE_TOLERANCE,
            "Mixed-rate preview tail",
        ));
        assert_eq!(frame_codes(&ctx, &preview_path).len(), 61);
    }
    eprintln!("VERIFIED mixed-rate 24000/1001 source / 30fps sequence tail cue: original/FFV1 sample96480; prepared {prepared_cue}; proxy {proxy_cue}; previews {preview_cues:?}; 61 sequence output frames.");
}
