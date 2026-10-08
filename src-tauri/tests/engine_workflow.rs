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
fn sample(ctx: &MediaContext, path: &Path, fps: &str) {
    media::run_ffmpeg(ctx,&vec!["-f".into(),"lavfi".into(),"-i".into(),format!("testsrc2=size=160x90:rate={fps}:duration=2.1"),"-f".into(),"lavfi".into(),"-i".into(),"aevalsrc=if(between(t\\,0.5\\,0.52)\\,0.6\\,0):s=48000:d=2.1".into(),"-vf".into(),"geq=lum='if(lt(X,40)*lt(Y,40),mod(N,20)*10+16,lum(X,Y))':cb='if(lt(X,20)*lt(Y,20),128,cb(X,Y))':cr='if(lt(X,20)*lt(Y,20),128,cr(X,Y))'".into(),"-c:v".into(),"libx264".into(),"-crf".into(),"16".into(),"-c:a".into(),"pcm_s16le".into(),"-shortest".into(),path.to_string_lossy().to_string()]).expect("Real-media fixture generation requires FFmpeg with libx264");
}
fn settings(codec: &str) -> ExportSettings {
    ExportSettings {
        width: 160,
        height: 90,
        fps: Rational::new(30, 1),
        codec: codec.into(),
        crf: 22,
        audio_bitrate: 160,
        sample_rate: 48000,
    }
}
fn wait(manager: &JobManager, id: &str) -> Job {
    let start = Instant::now();
    loop {
        let j = manager.list().into_iter().find(|j| j.id == id).unwrap();
        if j.status != "running" {
            assert!(
                j.status == "complete" || j.status == "cancelled",
                "Job failed: {:?}",
                j
            );
            return j;
        }
        assert!(start.elapsed() < Duration::from_secs(90), "Job timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn decoded(ctx: &MediaContext, path: &Path, args: &[&str]) -> Vec<u8> {
    let output = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-i"])
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
fn frames(ctx: &MediaContext, path: &Path) -> i64 {
    let output = media::command(&ctx.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap()
}
fn setup(dir: &Path, ctx: &MediaContext, source: &Path) -> ProjectStore {
    let mut store = ProjectStore::new(dir.join("recovery.json"));
    let p = store.import(vec![source.to_path_buf()], ctx).unwrap();
    store
        .apply(
            EditCommand::AddClip {
                media_id: p.media[0].id.clone(),
                track_id: p.tracks[0].id.clone(),
                start: 3,
                source_in: Some(Rational::new(1, 5)),
                duration: Some(45),
            },
            ctx,
        )
        .unwrap();
    store
}

#[test]
fn exact_rational_editing_and_rejected_edits_are_transactional() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let mut store = ProjectStore::new(dir.path().join("recovery.json"));
    let p = store
        .new_project("NTSC".into(), 1920, 1080, Rational::new(30000, 1001))
        .unwrap();
    assert_eq!(Rational::from_frames(30000, p.fps), Rational::new(1001, 1));
    let track = p.tracks[0].id.clone();
    let p = store
        .apply(
            EditCommand::AddTitle {
                track_id: track,
                start: 0,
                duration: 120,
                text: "Mono Cut".into(),
            },
            &ctx,
        )
        .unwrap();
    let id = p.clips[0].id.clone();
    store.apply(EditCommand::UpdateClip{id:id.clone(),patch:json!({"keyframes":[{"property":"opacity","frame":0,"value":0.0},{"property":"opacity","frame":90,"value":1.0}]})},&ctx).unwrap();
    let p = store
        .apply(
            EditCommand::Split {
                ids: vec![id.clone()],
                frame: 30,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(p.clips.len(), 2);
    assert_eq!(p.clips[1].source_in, Rational::new(1001, 1000));
    assert_eq!(p.clips[0].duration, 30);
    assert_eq!(p.clips[1].start, 30);
    assert!((edit::evaluate(&p.clips[1], "opacity", 0., 1.) - 1. / 3.).abs() < 1e-9);
    let before = store.get();
    assert!(store
        .apply(
            EditCommand::Move {
                ids: vec![id.clone()],
                delta: -1,
                track_id: None
            },
            &ctx
        )
        .is_err());
    assert_eq!(store.get(), before);
    assert!(store
        .apply(
            EditCommand::Move {
                ids: vec![id.clone()],
                delta: i64::MIN,
                track_id: None
            },
            &ctx
        )
        .is_err());
    assert_eq!(store.get(), before);
    let p = store.undo().unwrap();
    assert_eq!(p.clips.len(), 1);
    assert_eq!(store.redo().unwrap(), before);
    let p = store
        .apply(
            EditCommand::Delete {
                ids: vec![id],
                ripple: true,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(p.clips[0].start, 0);
    assert_eq!(p.clips[0].duration, 90);
    assert!(store.recovery_available());
    let mut recovered = ProjectStore::new(dir.path().join("recovery.json"));
    assert_eq!(recovered.recover().unwrap(), p);
    let bad = dir.path().join("bad.monocut");
    let mut malformed = serde_json::to_value(p).unwrap();
    malformed["fps"]["num"] = json!(i64::MIN);
    fs::write(&bad, serde_json::to_vec(&malformed).unwrap()).unwrap();
    assert!(store.open(&bad).is_err());
}

#[test]
fn real_media_workflow_accuracy_sync_proxies_cancellation_and_reopen() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("mixed-ntsc.mkv");
    sample(&ctx, &source, "30000/1001");
    let mut store = setup(dir.path(), &ctx, &source);
    let p = store.get();
    assert_eq!(p.media[0].fps, Rational::new(30000, 1001));
    assert!(Path::new(p.media[0].thumbnail.as_ref().unwrap()).is_file());
    assert_eq!(p.media[0].waveform.len(), 1024);
    assert!(p.media[0].waveform.iter().any(|v| *v > 0.1));
    let source_ready = media::prepare_source(&ctx, &p.media[0]).unwrap();
    assert!(Path::new(&source_ready).is_file());
    let project_path = dir.path().join("project.monocut");
    store.save(&project_path).unwrap();
    let serialized = fs::read_to_string(&project_path).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert!(!Path::new(raw["media"][0]["path"].as_str().unwrap()).is_absolute());
    let mut reopened = ProjectStore::new(dir.path().join("reopen-recovery.json"));
    assert_eq!(reopened.open(&project_path).unwrap().clips, p.clips);
    let hydrated = reopened.hydrate_assets(&ctx).unwrap();
    assert!(hydrated.media[0].thumbnail.is_some());
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let lossless = dir.path().join("accurate.mkv");
    let timer = Instant::now();
    let job = manager
        .export(p.clone(), lossless.clone(), settings("ffv1"))
        .unwrap();
    assert_eq!(wait(&manager, &job.id).status, "complete");
    let render_ms = timer.elapsed().as_millis();
    assert_eq!(frames(&ctx, &lossless), 48);
    let audio = decoded(
        &ctx,
        &lossless,
        &["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le"],
    );
    let samples: Vec<f32> = audio
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let first = samples
        .iter()
        .position(|v| v.abs() > 0.2)
        .expect("Audio pulse missing");
    assert!(
        (first as i64 - 19200).abs() <= 4,
        "Audio should start at 0.4 s / sample 19200, got {first}"
    );
    let actual = decoded(
        &ctx,
        &lossless,
        &[
            "-ss",
            "0.1",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ],
    );
    let reference = decoded(
        &ctx,
        &source,
        &[
            "-vf",
            "trim=start=0.2,setpts=PTS-STARTPTS,fps=30,scale=160:90,format=yuv420p",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ],
    );
    assert_eq!(actual.len(), reference.len());
    let rms = (actual
        .iter()
        .zip(&reference)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / actual.len() as f64)
        .sqrt();
    // The neutral 20x20 frame barcode is N*10 in luma. Checking it independently
    // catches a one-source-frame trim error without penalizing RGB/YUV chroma conversion.
    let barcode = |pixels: &[u8]| -> f64 {
        let mut sum = 0.;
        for y in 8..28 {
            for x in 8..28 {
                sum += pixels[(y * 160 + x) * 3] as f64;
            }
        }
        sum / 400.
    };
    let actual_code = barcode(&actual);
    let expected_code = barcode(&reference);
    assert!((actual_code-expected_code).abs()<=3.,"Source frame barcode mismatch: actual {actual_code}, expected {expected_code}; RGB RMS {rms}");
    let proxy = manager.proxy(p.media[0].clone()).unwrap();
    let proxy = wait(&manager, &proxy.id);
    store
        .set_proxy(&p.media[0].id, PathBuf::from(proxy.path.unwrap()))
        .unwrap();
    let proxied = store.get();
    let proxy_plan = render::compile(
        &ctx,
        &proxied,
        &settings("h264"),
        true,
        &dir.path().join("proxy-preview.mp4"),
        "proxy-plan",
    )
    .unwrap();
    assert!(proxy_plan.args.iter().any(|a| a.ends_with("-proxy-v3.mp4")));
    let start = Instant::now();
    let preview = manager.preview(proxied.clone(), 180, true).unwrap();
    let preview = wait(&manager, &preview.id);
    let preview_ms = start.elapsed().as_millis();
    assert_eq!(frames(&ctx, Path::new(preview.path.as_ref().unwrap())), 48);
    let export = dir.path().join("consistent.mp4");
    let h264 = ExportSettings {
        width: 320,
        height: 180,
        ..settings("h264")
    };
    let render_export =
        render::compile(&ctx, &proxied, &h264, false, &export, "same-export").unwrap();
    let render_preview = render::compile(
        &ctx,
        &proxied,
        &h264,
        true,
        &dir.path().join("same-preview.mp4"),
        "same-preview",
    )
    .unwrap();
    // The aligned fixture selects absolute source indexes 0/1. Normalized
    // proxies expose v:0/a:0; compare the same processing graph after mapping
    // those input labels, retaining every timing and effect expression.
    assert_eq!(
        render_export
            .filter_graph
            .replace("[0:0]", "[0:v:0]")
            .replace("[0:1]", "[0:a:0]"),
        render_preview.filter_graph,
        "Aligned proxy and original must use the same processing graph"
    );
    let exported = manager
        .export(proxied.clone(), export.clone(), h264)
        .unwrap();
    wait(&manager, &exported.id);
    let a = decoded(
        &ctx,
        &export,
        &[
            "-ss",
            "0.5",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ],
    );
    let b = decoded(
        &ctx,
        Path::new(preview.path.as_ref().unwrap()),
        &[
            "-ss",
            "0.5",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ],
    );
    let diff = (a
        .iter()
        .zip(&b)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / a.len() as f64)
        .sqrt();
    assert!(diff < 12., "Preview/proxy vs export RGB RMS {diff}");
    let cancelled = dir.path().join("cancelled.mp4");
    let slow = ExportSettings {
        width: 3840,
        height: 2160,
        ..settings("h264")
    };
    let job = manager
        .export(proxied.clone(), cancelled.clone(), slow)
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    manager.cancel(&job.id).unwrap();
    assert_eq!(wait(&manager, &job.id).status, "cancelled");
    assert!(!cancelled.exists());
    assert!(!fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("mono-part")));
    fs::rename(&source, dir.path().join("replacement.mkv")).unwrap();
    let missing = reopened.open(&project_path).unwrap();
    assert!(missing.media[0].missing);
    let relinked = reopened
        .apply(
            EditCommand::Relink {
                media_id: missing.media[0].id.clone(),
                path: dir
                    .path()
                    .join("replacement.mkv")
                    .to_string_lossy()
                    .to_string(),
            },
            &ctx,
        )
        .unwrap();
    assert!(!relinked.media[0].missing);
    assert_eq!(relinked.clips, p.clips);
    eprintln!("VERIFIED: 30000/1001 source on 30 fps sequence; 48 output frames; source trim RGB RMS {rms:.3}; audio pulse sample {first}; proxy preview/export RGB RMS {diff:.3}; 160x90 FFV1 render {render_ms} ms; 320x180 cached preview render {preview_ms} ms; save/reopen/missing/relink/undo/cancel complete.");
}

#[test]
fn effects_keyframes_titles_and_explicit_frame_rate_conversion() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("source.mkv");
    sample(&ctx, &source, "24");
    let mut store = setup(dir.path(), &ctx, &source);
    let p = store.get();
    let clip = p.clips[0].id.clone();
    store.apply(EditCommand::UpdateClip{id:clip.clone(),patch:json!({"speed":{"num":5,"den":4},"duration":36,"fade_in":6,"fade_out":6,"brightness":0.05,"contrast":1.1,"saturation":0.8,"transform":{"scale":0.8,"rotation":12.0,"crop_left":0.05,"crop_right":0.05},"keyframes":[{"property":"opacity","frame":0,"value":0.2},{"property":"opacity","frame":35,"value":1.0},{"property":"volume","frame":0,"value":0.5},{"property":"volume","frame":35,"value":0.8},{"property":"x","frame":0,"value":-10.0},{"property":"x","frame":35,"value":10.0},{"property":"scale","frame":0,"value":0.8},{"property":"scale","frame":35,"value":1.0}]})},&ctx).unwrap();
    store
        .apply(
            EditCommand::AddTrack {
                kind: "video".into(),
                name: "Title".into(),
            },
            &ctx,
        )
        .unwrap();
    let p2 = store.get();
    if ctx.font.is_file() {
        store
            .apply(
                EditCommand::AddTitle {
                    track_id: p2.tracks.last().unwrap().id.clone(),
                    start: 6,
                    duration: 24,
                    text: "Mono Cut : 100% 'open'\nTitle".into(),
                },
                &ctx,
            )
            .unwrap();
    }
    let mut s = settings("ffv1");
    s.fps = Rational::new(60, 1);
    let out = dir.path().join("effects.mkv");
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let job = manager.export(store.get(), out.clone(), s).unwrap();
    wait(&manager, &job.id);
    assert_eq!(frames(&ctx, &out), 78);
    let pcm = decoded(
        &ctx,
        &out,
        &["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le"],
    );
    let pcm: Vec<f32> = pcm
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let pulse = pcm.iter().position(|v| v.abs() > 0.2).unwrap();
    assert!(
        (pulse as i64 - 16320).abs() <= 8,
        "Speed-adjusted pulse should align at sample 16320, got {pulse}"
    );
    let p = store
        .apply(EditCommand::Unlink { ids: vec![clip] }, &ctx)
        .unwrap();
    assert!(p.clips.iter().any(|c| c.track_id == p.tracks[1].id));
    assert_eq!(p.clips[0].volume, 0.);
    let ids = p
        .clips
        .iter()
        .filter(|c| c.media_id.is_some())
        .map(|c| c.id.clone())
        .collect();
    store.apply(EditCommand::Link { ids }, &ctx).unwrap();
    let linked = store.get();
    let ids = vec![linked.clips[0].id.clone()];
    let moved = store
        .apply(
            EditCommand::Move {
                ids,
                delta: 3,
                track_id: None,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(moved.clips[0].start, 6);
    assert_eq!(
        moved
            .clips
            .iter()
            .find(|c| c.track_id == moved.tracks[1].id)
            .unwrap()
            .start,
        6
    );
    assert_eq!(p.media[0].fps, Rational::new(24, 1));
    eprintln!("VERIFIED: speed/crop/rotation/color/fades/animated opacity-volume-position-scale compile and render; 24 fps input to explicit 60 fps output, 78 frames; audio extraction and linked movement. Inter title test included: {}",ctx.font.is_file());
}

#[test]
fn shutdown_stops_source_preparation_and_encoder_jobs_without_partial_files() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("long-silent-source.mp4");
    media::run_ffmpeg(
        &ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "color=c=white:s=640x360:r=30:d=120".into(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "ultrafast".into(),
            source.to_string_lossy().to_string(),
        ],
    )
    .unwrap();
    let media = media::probe(&ctx, &source).unwrap();
    let worker_ctx = ctx.clone();
    let worker = std::thread::spawn(move || media::prepare_source(&worker_ctx, &media));
    let started = Instant::now();
    while ctx.processes.active_count() == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Source preparation did not start"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    ctx.processes.shutdown();
    let error = worker.join().unwrap().unwrap_err();
    assert!(
        error.contains("closing"),
        "Expected shutdown cancellation, got {error}"
    );
    assert_eq!(ctx.processes.active_count(), 0);
    assert!(fs::read_dir(&ctx.cache_dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("part")));
    assert!(
        media::run_ffmpeg(&ctx, &vec!["-version".into()])
            .unwrap_err()
            .contains("closing"),
        "Shutdown must reject new processes"
    );

    let ctx = context(dir.path());
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let mut p = Project::new("Shutdown export".into(), 640, 360, Rational::new(30, 1));
    let add = EditCommand::AddTitle {
        track_id: p.tracks[0].id.clone(),
        start: 0,
        duration: 9000,
        text: "Mono Cut".into(),
    };
    edit::apply(&mut p, add).unwrap();
    let output = dir.path().join("cancelled-export.mp4");
    let mut export_settings = settings("h264");
    export_settings.width = 640;
    export_settings.height = 360;
    let job = manager.export(p, output.clone(), export_settings).unwrap();
    let started = Instant::now();
    while ctx.processes.active_count() == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Encoder did not start"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    manager.shutdown();
    let finished = manager
        .list()
        .into_iter()
        .find(|entry| entry.id == job.id)
        .unwrap();
    assert_eq!(finished.status, "cancelled");
    assert_eq!(ctx.processes.active_count(), 0);
    assert!(!output.exists());
    assert!(fs::read_dir(dir.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mono-part-")));
    eprintln!("VERIFIED: shutdown kills/reaps active source preparation and encoder children, clears partial files, reports job cancellation, and prevents new child launches.");
}

#[test]
fn rotated_and_anamorphic_sources_preserve_display_fit_with_proxies() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    for (name, sar, rotation, dimensions, expected_picture) in [
        ("portrait", "1", true, (90, 160), (100, 180)),
        ("anamorphic", "2", false, (320, 90), (320, 90)),
        ("rotated-anamorphic", "2", true, (90, 320), (50, 180)),
    ] {
        let encoded = dir.path().join(format!("{name}-encoded.mp4"));
        media::run_ffmpeg(
            &ctx,
            &vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "color=c=white:s=160x90:r=30:d=1".into(),
                "-vf".into(),
                format!("setsar={sar}"),
                "-c:v".into(),
                "libx264".into(),
                "-crf".into(),
                "12".into(),
                encoded.to_string_lossy().to_string(),
            ],
        )
        .unwrap();
        let source = if rotation {
            let source = dir.path().join(format!("{name}.mp4"));
            media::run_ffmpeg(
                &ctx,
                &vec![
                    "-display_rotation".into(),
                    "90".into(),
                    "-i".into(),
                    encoded.to_string_lossy().to_string(),
                    "-c".into(),
                    "copy".into(),
                    source.to_string_lossy().to_string(),
                ],
            )
            .unwrap();
            source
        } else {
            encoded
        };
        let m = media::import(&ctx, &source).unwrap();
        assert_eq!(
            (m.width, m.height),
            dimensions,
            "{name}: metadata must describe display geometry"
        );
        let prepared = media::prepare_source(&ctx, &m).unwrap();
        let proxy_job = manager.proxy(m.clone()).unwrap();
        let proxy = wait(&manager, &proxy_job.id).path.unwrap();
        for path in [&prepared, &proxy] {
            let output = media::command(&ctx.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=width,height,sample_aspect_ratio",
                    "-of",
                    "json",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(output.status.success());
            let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                metadata["streams"][0]["sample_aspect_ratio"], "1:1",
                "{name}: caches must use square pixels"
            );
        }
        let mut p = Project::new(name.into(), 320, 180, Rational::new(30, 1));
        p.media.push(m);
        let add = EditCommand::AddClip {
            media_id: p.media[0].id.clone(),
            track_id: p.tracks[0].id.clone(),
            start: 0,
            source_in: None,
            duration: Some(30),
        };
        edit::apply(&mut p, add).unwrap();
        p.media[0].proxy = Some(proxy);
        let mut rendered = vec![];
        for use_proxy in [false, true] {
            let output = dir.path().join(format!("{name}-{use_proxy}.mkv"));
            let mut render_settings = settings("ffv1");
            render_settings.width = 320;
            render_settings.height = 180;
            let plan =
                render::compile(&ctx, &p, &render_settings, use_proxy, &output, &id()).unwrap();
            media::run_ffmpeg(&ctx, &plan.args).unwrap();
            let pixels = decoded(
                &ctx,
                &output,
                &["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24"],
            );
            assert_eq!(pixels.len(), 320 * 180 * 3);
            let mut min_x = 320usize;
            let mut max_x = 0;
            let mut min_y = 180usize;
            let mut max_y = 0;
            for y in 0..180 {
                for x in 0..320 {
                    let offset = (y * 320 + x) * 3;
                    if pixels[offset..offset + 3]
                        .iter()
                        .all(|channel| *channel > 180)
                    {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                        min_y = min_y.min(y);
                        max_y = max_y.max(y);
                    }
                }
            }
            // Keep the geometry expectation unchanged while distinguishing
            // compositor pixels from architecture-specific RGB conversion.
            let scalar = media::command(&ctx.ffmpeg)
                .args(["-v", "error", "-cpuflags", "0", "-i"])
                .arg(&output)
                .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap();
            assert!(scalar.status.success());
            assert_eq!(scalar.stdout.len(), pixels.len());
            let luma = decoded(
                &ctx,
                &output,
                &[
                    "-frames:v",
                    "1",
                    "-vf",
                    "extractplanes=y",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "gray",
                ],
            );
            assert_eq!(luma.len(), 320 * 180);
            let bounds = |data: &[u8], channels: usize, cutoff: u8| {
                let mut bx0 = 320usize;
                let mut bx1 = 0usize;
                let mut by0 = 180usize;
                let mut by1 = 0usize;
                for y in 0..180 {
                    for x in 0..320 {
                        let offset = (y * 320 + x) * channels;
                        if data[offset..offset + channels].iter().all(|v| *v > cutoff) {
                            bx0 = bx0.min(x);
                            bx1 = bx1.max(x);
                            by0 = by0.min(y);
                            by1 = by1.max(y);
                        }
                    }
                }
                (bx0 <= bx1 && by0 <= by1)
                    .then(|| (bx0, bx1, by0, by1, bx1 - bx0 + 1, by1 - by0 + 1))
            };
            let cutoffs: Vec<_> = [32u8, 64, 96, 127, 160, 180, 200, 220]
                .into_iter()
                .map(|cutoff| {
                    (
                        cutoff,
                        bounds(&pixels, 3, cutoff),
                        bounds(&scalar.stdout, 3, cutoff),
                        bounds(&luma, 1, cutoff),
                    )
                })
                .collect();
            let left: usize = (320 - expected_picture.0) / 2;
            let right = left + expected_picture.0 - 1;
            let edges: Vec<_> = (left.saturating_sub(3)..=left + 3)
                .chain(right.saturating_sub(3)..=(right + 3).min(319))
                .map(|x| {
                    let offset = (90 * 320 + x) * 3;
                    (
                        x,
                        &pixels[offset..offset + 3],
                        &scalar.stdout[offset..offset + 3],
                        luma[90 * 320 + x],
                    )
                })
                .collect();
            eprintln!("FIT_DIAGNOSTIC {name} proxy={use_proxy}: thresholds=(cutoff, defaultRGB, scalarRGB, rawY) {cutoffs:?}; midpoint edges=(x, defaultRGB, scalarRGB, rawY) {edges:?}; graph={}", plan.filter_graph);
            assert_eq!(
                (max_x - min_x + 1, max_y - min_y + 1),
                expected_picture,
                "{name} proxy={use_proxy}: picture must fit without aspect distortion"
            );
            rendered.push(pixels);
        }
        let rms = (rendered[0]
            .iter()
            .zip(&rendered[1])
            .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
            .sum::<f64>()
            / rendered[0].len() as f64)
            .sqrt();
        assert!(
            rms < 3.,
            "{name}: original/proxy geometry differs, RGB RMS {rms}"
        );
        eprintln!("VERIFIED: {name} display {}x{}, fitted white picture {}x{}, square-pixel source/proxy caches, original/proxy RGB RMS {rms:.3}.", dimensions.0, dimensions.1, expected_picture.0, expected_picture.1);
    }
}

#[test]
fn shortening_sequence_clamps_export_range_and_undo_restores_it() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let mut store = ProjectStore::new(dir.path().join("recovery.json"));
    let p = store.get();
    let p = store
        .apply(
            EditCommand::AddTitle {
                track_id: p.tracks[0].id.clone(),
                start: 0,
                duration: 120,
                text: "Range test".into(),
            },
            &ctx,
        )
        .unwrap();
    let clip = p.clips[0].id.clone();
    let marked = store
        .apply(
            EditCommand::SetRange {
                in_point: Some(10),
                out_point: Some(120),
            },
            &ctx,
        )
        .unwrap();
    let trimmed = store
        .apply(
            EditCommand::Trim {
                id: clip.clone(),
                edge: "out".into(),
                frame: 90,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!((trimmed.in_point, trimmed.out_point), (Some(10), Some(90)));
    assert_eq!(store.undo().unwrap(), marked);
    assert_eq!(store.redo().unwrap(), trimmed);

    store
        .apply(
            EditCommand::SetRange {
                in_point: Some(80),
                out_point: Some(90),
            },
            &ctx,
        )
        .unwrap();
    let trimmed = store
        .apply(
            EditCommand::Trim {
                id: clip.clone(),
                edge: "out".into(),
                frame: 60,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!((trimmed.in_point, trimmed.out_point), (Some(59), Some(60)));
    assert!(store
        .apply(
            EditCommand::SetRange {
                in_point: Some(70),
                out_point: Some(90)
            },
            &ctx
        )
        .is_err());
    assert_eq!(
        store.get(),
        trimmed,
        "Explicit invalid ranges remain rejected"
    );
    let empty = store
        .apply(
            EditCommand::Delete {
                ids: vec![clip],
                ripple: false,
            },
            &ctx,
        )
        .unwrap();
    assert!(empty.clips.is_empty());
    assert_eq!((empty.in_point, empty.out_point), (None, None));
    assert_eq!(store.undo().unwrap(), trimmed);
    eprintln!("VERIFIED: trim clamps export in/out to sequence end, delete clears empty range, undo restores ranges, explicit invalid ranges rejected.");
}

#[test]
fn hydrated_open_failure_preserves_project_recovery_and_edit_history() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let source = dir.path().join("source.mkv");
    sample(&ctx, &source, "30");
    let mut store = setup(dir.path(), &ctx, &source);
    let earlier = store.get();
    let before = store
        .apply(
            EditCommand::Rename {
                name: "Current edit".into(),
            },
            &ctx,
        )
        .unwrap();
    let recovery = dir.path().join("recovery.json");
    let recovery_before = fs::read(&recovery).unwrap();

    let corrupt = dir.path().join("corrupt.mkv");
    fs::write(&corrupt, b"existing file without a media container").unwrap();
    let mut candidate = before.clone();
    candidate.id = id();
    candidate.name = "Candidate project".into();
    candidate.media[0].thumbnail = None;
    candidate.media[0].waveform.clear();
    let mut broken_media = candidate.media[0].clone();
    broken_media.id = id();
    broken_media.name = "Corrupt media".into();
    broken_media.path = corrupt.to_string_lossy().to_string();
    candidate.media.push(broken_media);
    let document = dir.path().join("candidate.monocut");
    mono_cut_lib::storage::write_atomic(&document, &candidate).unwrap();

    assert!(store.open_hydrated(&document, &ctx).is_err());
    assert_eq!(
        store.get(),
        before,
        "Failed open must retain the visible current project"
    );
    assert_eq!(
        fs::read(&recovery).unwrap(),
        recovery_before,
        "Failed open must retain the previous autosave"
    );
    assert_eq!(
        store.undo().unwrap(),
        earlier,
        "Failed open must retain undo history"
    );
    assert_eq!(store.redo().unwrap(), before);

    let unhydrated = store.open(&document).unwrap();
    let recovery_before = fs::read(&recovery).unwrap();
    assert!(store.hydrate_assets(&ctx).is_err());
    assert_eq!(
        store.get(),
        unhydrated,
        "Failed hydration must not attach partial assets"
    );
    assert_eq!(fs::read(&recovery).unwrap(), recovery_before);

    fs::remove_file(&corrupt).unwrap();
    let missing = store.open_hydrated(&document, &ctx).unwrap();
    assert!(!missing.media[0].missing);
    assert!(missing.media[0].thumbnail.is_some());
    assert!(
        missing.media[1].missing,
        "Missing files remain reopenable for relinking"
    );
    eprintln!("VERIFIED: failed hydrated open and asset hydration preserve project/autosave/history; missing-media open remains supported.");
}

#[test]
fn pointer_moves_preserve_linked_audio_and_other_selected_lanes() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let source = dir.path().join("source.mkv");
    sample(&ctx, &source, "30");
    let mut store = setup(dir.path(), &ctx, &source);
    let p = store.get();
    let video_id = p.clips[0].id.clone();
    let video_track = p.tracks[0].id.clone();
    let audio_track = p.tracks[1].id.clone();
    let p = store
        .apply(
            EditCommand::Unlink {
                ids: vec![video_id.clone()],
            },
            &ctx,
        )
        .unwrap();
    let audio_id = p
        .clips
        .iter()
        .find(|c| c.track_id == audio_track)
        .unwrap()
        .id
        .clone();
    store
        .apply(
            EditCommand::Link {
                ids: vec![video_id.clone(), audio_id.clone()],
            },
            &ctx,
        )
        .unwrap();
    let p = store
        .apply(
            EditCommand::AddTrack {
                kind: "video".into(),
                name: "Overlay".into(),
            },
            &ctx,
        )
        .unwrap();
    let overlay_track = p.tracks.last().unwrap().id.clone();
    let p = store
        .apply(
            EditCommand::AddTrack {
                kind: "video".into(),
                name: "Destination".into(),
            },
            &ctx,
        )
        .unwrap();
    let destination = p.tracks.last().unwrap().id.clone();
    let p = store
        .apply(
            EditCommand::AddTitle {
                track_id: overlay_track.clone(),
                start: 3,
                duration: 45,
                text: "Overlay".into(),
            },
            &ctx,
        )
        .unwrap();
    let title_id = p.clips.last().unwrap().id.clone();
    let selected = vec![video_id.clone(), title_id.clone()];

    // Old pointer clients included the current lane even for a horizontal drag.
    let p = store
        .apply(
            EditCommand::Move {
                ids: selected.clone(),
                delta: 3,
                track_id: Some(video_track.clone()),
            },
            &ctx,
        )
        .unwrap();
    for (clip_id, lane) in [
        (&video_id, &video_track),
        (&audio_id, &audio_track),
        (&title_id, &overlay_track),
    ] {
        let c = p.clips.iter().find(|c| &c.id == clip_id).unwrap();
        assert_eq!(&c.track_id, lane);
        assert_eq!(c.start, 6);
    }
    let p = store
        .apply(
            EditCommand::Move {
                ids: selected,
                delta: 3,
                track_id: Some(destination.clone()),
            },
            &ctx,
        )
        .unwrap();
    for (clip_id, lane) in [
        (&video_id, &destination),
        (&audio_id, &audio_track),
        (&title_id, &overlay_track),
    ] {
        let c = p.clips.iter().find(|c| &c.id == clip_id).unwrap();
        assert_eq!(&c.track_id, lane);
        assert_eq!(c.start, 9);
    }
    assert!(store
        .apply(
            EditCommand::Move {
                ids: vec![video_id],
                delta: 0,
                track_id: Some(audio_track)
            },
            &ctx
        )
        .is_err());
    assert_eq!(
        store.get(),
        p,
        "Cross-kind lane changes must be rejected transactionally"
    );
    eprintln!("VERIFIED: horizontal pointer moves retain linked audio/selected lanes; cross-lane moves affect only the anchor lane; cross-kind changes are rejected.");
}

#[test]
fn cross_dissolve_blends_two_opaque_clips_without_a_black_dip() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let mut paths = vec![];
    for color in ["red", "blue"] {
        let path = dir.path().join(format!("{color}.mkv"));
        media::run_ffmpeg(
            &ctx,
            &vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!("color=c={color}:s=160x90:r=30:d=1"),
                "-c:v".into(),
                "ffv1".into(),
                path.to_string_lossy().to_string(),
            ],
        )
        .unwrap();
        paths.push(path);
    }
    let mut store = ProjectStore::new(dir.path().join("recovery.json"));
    let p = store.import(paths, &ctx).unwrap();
    let track = p.tracks[0].id.clone();
    for (i, m) in p.media.iter().enumerate() {
        store
            .apply(
                EditCommand::AddClip {
                    media_id: m.id.clone(),
                    track_id: track.clone(),
                    start: i as i64 * 30,
                    source_in: None,
                    duration: Some(30),
                },
                &ctx,
            )
            .unwrap();
    }
    let incoming = store.get().clips[1].id.clone();
    let p = store
        .apply(
            EditCommand::CrossDissolve {
                id: incoming,
                frames: 12,
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(p.clips[1].start, 18);
    assert_eq!(p.clips[0].fade_out, 12);
    assert_eq!(p.clips[1].fade_in, 12);
    let output = dir.path().join("dissolve.mkv");
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let job = manager.export(p, output.clone(), settings("ffv1")).unwrap();
    wait(&manager, &job.id);
    let pixels = decoded(
        &ctx,
        &output,
        &[
            "-ss",
            "0.8",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ],
    );
    let center = (45 * 160 + 80) * 3;
    let red = pixels[center];
    let blue = pixels[center + 2];
    assert!(
        (red as i16 - 127).abs() <= 8 && (blue as i16 - 127).abs() <= 8,
        "Midpoint should have half red and half blue, got R{red} B{blue}"
    );
    eprintln!(
        "VERIFIED: 12-frame cross dissolve midpoint R{red}/B{blue}, no compounded black dip."
    );
}
