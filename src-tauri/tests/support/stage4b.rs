#![allow(dead_code)]
//! Deterministic, silent actual-media support for the explicitly requested region gate.
use mono_cut_lib::{
    edit,
    media::{self, MediaContext},
    model::*,
    processes::ProcessRegistry,
    storage,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub fn hash(path: &Path) -> String {
    let mut f = fs::File::open(path).unwrap();
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b).unwrap();
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    format!("{:x}", h.finalize())
}
pub fn disk(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                disk(&p)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}
pub fn context(dir: &Path) -> MediaContext {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    let tool = |env: &str, name: &str| {
        std::env::var_os(env)
            .map(PathBuf::from)
            .unwrap_or(resources.join(name))
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
        processes: Arc::new(ProcessRegistry::default()),
    }
}
pub fn directory() -> PathBuf {
    let dir = std::env::var_os("MONO_CUT_REGION_BENCH_DIR")
        .map(PathBuf::from)
        .expect(
            "Set MONO_CUT_REGION_BENCH_DIR to a directory outside the published source checkout",
        );
    fs::create_dir_all(&dir).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    assert!(
        !dir.canonicalize().unwrap().starts_with(root),
        "Keep synthetic media and private timing artifacts outside the source checkout"
    );
    dir
}
pub fn write(dir: &Path, name: &str, value: &Value) {
    fs::write(dir.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
pub fn cleanup(ctx: &MediaContext, job_id: &str) {
    assert!(!job_id.is_empty());
    let prefix = format!("{job_id}-");
    for entry in fs::read_dir(&ctx.cache_dir).unwrap().flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && entry.file_name().to_string_lossy().starts_with(&prefix)
        {
            fs::remove_file(entry.path()).unwrap();
        }
    }
}
pub fn snapshot_sources() -> Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let names = [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src/audio_clock.rs",
        "src/video_seek.rs",
        "src/model.rs",
        "src/edit.rs",
        "src/media.rs",
        "src/render.rs",
        "src/jobs.rs",
        "src/processes.rs",
        "src/preview.rs",
        "src/lib.rs",
        "tests/region_benchmark.rs",
        "tests/region_reference.rs",
        "tests/region_reference_control.rs",
        "tests/support/stage4b.rs",
        "../src/App.tsx",
        "../src/Monitor.tsx",
        "../src/monitorLifecycle.ts",
        "../src/ProgramMedia.tsx",
        "../src/programMediaSlots.ts",
        "../src/programTransport.ts",
        "../src/editorShortcuts.ts",
        "../src/GuidedTour.tsx",
        "../src/types.ts",
        "../src/previewBridge.ts",
        "../src/previewScheduler.ts",
        "../src/previewModel.ts",
        "../src/previewAssets.ts",
        "../src/programPreview.ts",
        "../src/previewRegion.ts",
        "examples/preview_region_driver.rs",
        "../scripts/measure-preview-regions.mjs",
        "../scripts/measure-preview-continuity.mjs",
        "tests/continuity_reference.rs",
    ];
    Value::Object(
        names
            .into_iter()
            .filter_map(|n| {
                let p = root.join(n);
                p.is_file().then(|| {
                    (
                        n.into(),
                        json!({"sha256":hash(&p),"bytes":fs::metadata(p).unwrap().len()}),
                    )
                })
            })
            .collect(),
    )
}
pub fn preserve_previous(dir: &Path) -> Value {
    let Some(old) = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(|root| root.join("work/stage-4a-preview-benchmark/final-run"))
    else {
        return json!({"available":false});
    };
    let report_path = old.join("benchmark-results.json");
    if !report_path.exists() {
        return json!({"available":false});
    }
    let report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    let mut files = serde_json::Map::new();
    for p in [report_path, old.join("synthetic1080p12s.mp4")] {
        if p.exists() {
            files.insert(
                p.file_name().unwrap().to_string_lossy().into(),
                json!({"sha256":hash(&p),"bytes":fs::metadata(&p).unwrap().len()}),
            );
        }
    }
    for case in report["cases"].as_array().unwrap() {
        if let Some(name) = case["output"].as_str() {
            let p = old.join("cache").join(name);
            if p.exists() {
                let actual = hash(&p);
                assert_eq!(
                    Some(actual.as_str()),
                    case["output_sha256"].as_str(),
                    "Retained earlier stress output changed"
                );
                files.insert(
                    name.into(),
                    json!({"sha256":actual,"bytes":fs::metadata(&p).unwrap().len()}),
                );
            }
        }
    }
    let evidence = json!({"available":true,"retained_files":files,"earlier_report":report,
        "limitations":"Four simultaneous video tracks with portions offcanvas and no source audio. Earlier timings are retained builder measurements, not native-display observations."});
    write(dir, "retained-stress-evidence.json", &evidence);
    evidence
}
fn source(
    ctx: &MediaContext,
    dir: &Path,
    name: &str,
    fps: &str,
    frequency: u32,
    phase: &str,
) -> PathBuf {
    let path = dir.join(name);
    if path.exists() {
        return path;
    }
    let font = ctx
        .font
        .to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'");
    // Source markers are baked into actual 1080p media; frame number and source
    // time advance at each source's own exact cadence. Audio is PCM, never played.
    let vf = format!("drawbox=x=0:y=0:w=520:h=150:color=black:t=fill,drawtext=fontfile='{font}':text='F%{{n}} T%{{pts\\:flt}}':fontcolor=white:fontsize=64:x=24:y=30");
    let audio = format!("aevalsrc=if(lt(mod(t+{phase}\\,1)\\,0.025)\\,0.22*sin(2*PI*{frequency}*t)\\,0):s=48000:d=16");
    let args = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("testsrc2=size=1920x1080:rate={fps}:duration=16"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        audio,
        "-vf".into(),
        vf,
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-crf".into(),
        "18".into(),
        "-g".into(),
        "15".into(),
        "-threads".into(),
        "2".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        "-ac".into(),
        "2".into(),
        "-t".into(),
        "16".into(),
        path.to_string_lossy().into(),
    ];
    media::run_ffmpeg(ctx, &args).unwrap();
    path
}
fn track(name: &str, kind: &str) -> Track {
    Track {
        id: name.into(),
        name: name.into(),
        kind: kind.into(),
        muted: false,
        hidden: false,
        locked: false,
    }
}
fn add(
    p: &mut Project,
    mid: &str,
    tid: &str,
    name: &str,
    start: i64,
    source_in: Rational,
    duration: i64,
) {
    edit::apply(
        p,
        EditCommand::AddClip {
            media_id: mid.into(),
            track_id: tid.into(),
            start,
            source_in: Some(source_in),
            duration: Some(duration),
        },
    )
    .unwrap();
    let c = p.clips.last_mut().unwrap();
    c.id = name.into();
    c.name = name.into();
}
pub fn fixture(ctx: &MediaContext, dir: &Path) -> Project {
    media::ensure_cache(ctx).unwrap();
    let begun = Instant::now();
    let paths = [
        source(ctx, dir, "source-1080p30.mkv", "30", 440, "0"),
        source(
            ctx,
            dir,
            "source-1080p-30000-1001.mkv",
            "30000/1001",
            660,
            "0.25",
        ),
    ];
    let mut p = Project::new(
        "Stage 4B ordinary two-video two-audio workload".into(),
        1920,
        1080,
        Rational::new(30, 1),
    );
    p.id = "stage4b-ordinary-v1".into();
    for (i, path) in paths.iter().enumerate() {
        let mut m = media::probe(ctx, path).unwrap();
        m.id = format!("stage4b-source-{i}");
        p.media.push(m);
    }
    p.tracks = vec![
        track("V1", "video"),
        track("V2", "video"),
        track("A1", "audio"),
        track("A2", "audio"),
    ];
    for i in 0..5 {
        let s = i * 360;
        // Linked AV uses identical source offset and varispeed mapping.
        for (tid, source_index) in [
            ("V1", i % 2),
            ("V2", (i + 1) % 2),
            ("A1", i % 2),
            ("A2", (i + 1) % 2),
        ] {
            add(
                &mut p,
                &format!("stage4b-source-{source_index}"),
                tid,
                &format!("{tid}-{i}"),
                s,
                Rational::new(i, 4),
                360,
            );
            let c = p.clips.last_mut().unwrap();
            c.speed = if i == 2 {
                Rational::new(11, 10)
            } else {
                Rational::one()
            };
            if tid == "V1" {
                c.volume = 0.;
                c.linked_id = Some(format!("pair-{i}"));
            }
            if tid == "V2" {
                c.volume = 0.;
                c.opacity = 0.9;
                c.transform.scale = 0.35;
                c.transform.x = 500.;
                c.transform.y = -240.;
                c.fade_in = 15;
                c.fade_out = 15;
            }
            if tid == "A1" {
                c.volume = 0.55;
                c.linked_id = Some(format!("pair-{i}"));
            }
            if tid == "A2" {
                c.volume = 0.35;
                c.fade_in = 15;
                c.fade_out = 15;
                c.keyframes = vec![
                    Keyframe {
                        property: "volume".into(),
                        frame: 0,
                        value: 0.15,
                    },
                    Keyframe {
                        property: "volume".into(),
                        frame: 180,
                        value: 0.35,
                    },
                    Keyframe {
                        property: "volume".into(),
                        frame: 359,
                        value: 0.2,
                    },
                ];
            }
        }
    }
    // Incoming duration expands before the overlap, retaining exact 60 s extent.
    for (tid, i) in [("V1", 2), ("V2", 1)] {
        edit::apply(
            &mut p,
            EditCommand::UpdateClip {
                id: format!("{tid}-{i}"),
                patch: json!({"duration":375}),
            },
        )
        .unwrap();
        if tid == "V1" {
            edit::apply(
                &mut p,
                EditCommand::UpdateClip {
                    id: format!("A1-{i}"),
                    patch: json!({"duration":375}),
                },
            )
            .unwrap();
        }
        edit::apply(
            &mut p,
            EditCommand::CrossDissolve {
                id: format!("{tid}-{i}"),
                frames: 15,
            },
        )
        .unwrap();
    }
    for c in &mut p.clips {
        if c.id == "V1-0" || c.id == "A1-0" {
            c.fade_in = 15;
        }
        if c.id == "V1-4" || c.id == "A1-4" {
            c.fade_out = 30;
        }
    }
    edit::apply(
        &mut p,
        EditCommand::AddTitle {
            track_id: "V2".into(),
            start: 750,
            duration: 300,
            text: "Region / 30 fps".into(),
        },
    )
    .unwrap();
    let title = p.clips.last_mut().unwrap();
    title.id = "stage4b-title".into();
    title.name = "Region title".into();
    title.fade_in = 15;
    title.fade_out = 15;
    title.transform.y = 300.;
    p.in_point = Some(300);
    p.out_point = Some(1500); // Preview must ignore the user's export range.
    p.validate().unwrap();
    assert_eq!(p.length(), 1800);
    assert_eq!(p.clips.len(), 21);
    assert_eq!(p.tracks.len(), 4);
    storage::write_atomic(&dir.join("ordinary-60s.monocut"), &p).unwrap();
    let inputs:Vec<_>=paths.iter().zip(p.media.iter()).map(|(path,m)|json!({"filename":path.file_name().unwrap().to_string_lossy(),"bytes":fs::metadata(path).unwrap().len(),"sha256":hash(path),"fps":m.fps,"duration":m.duration,"timing":m.timing,"width":m.width,"height":m.height,"has_audio":m.has_audio})).collect();
    write(
        dir,
        "fixture-manifest.json",
        &json!({"recipe":"stage4b-ordinary-v1","sequence_frames":1800,"sequence_fps":p.fps,"sequence_dimensions":[1920,1080],"sample_rate":48000,"preview_dimensions":[960,540],"preview_codec":"H.264 CRF20 / AAC160kbps","proxy":"engine normalized1280x720 H.264CRF23; prepared separately","video_tracks":2,"audio_tracks":2,"clips":21,"title_range_frames":[750,1050],"source_generation_or_lookup_seconds":begun.elapsed().as_secs_f64(),"sources":inputs,"project_sha256":hash(&dir.join("ordinary-60s.monocut")),"no_native_display_or_audible_output":true}),
    );
    p
}
pub fn settings(codec: &str) -> ExportSettings {
    ExportSettings {
        width: 960,
        height: 540,
        fps: Rational::new(30, 1),
        codec: codec.into(),
        crf: if codec == "h264" { 20 } else { 22 },
        audio_bitrate: 160,
        sample_rate: 48000,
    }
}
pub fn run(ctx: &MediaContext, args: &[String]) {
    media::run_ffmpeg(ctx, args).unwrap();
}
pub fn decoded(ctx: &MediaContext, path: &Path, args: &[String]) -> Vec<u8> {
    let mut command = media::command(&ctx.ffmpeg);
    command
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(args)
        .arg("pipe:1");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
pub fn frame(ctx: &MediaContext, path: &Path, index: i64) -> Vec<u8> {
    decoded(
        ctx,
        path,
        &[
            "-vf".into(),
            format!("select=eq(n\\,{index})"),
            "-frames:v".into(),
            "1".into(),
            "-an".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgb24".into(),
        ],
    )
}
pub fn video_metrics(a: &[u8], b: &[u8]) -> Value {
    assert_eq!(a.len(), b.len());
    assert!(!a.is_empty());
    let mut square = 0.;
    let mut abs = 0.;
    let mut max = 0u8;
    let mut hist = [0usize; 256];
    for (&a, &b) in a.iter().zip(b) {
        let d = a.abs_diff(b);
        square += (d as f64).powi(2);
        abs += d as f64;
        max = max.max(d);
        hist[d as usize] += 1;
    }
    let mut n = 0;
    let mut p99 = 0;
    for (d, count) in hist.iter().enumerate() {
        n += count;
        if n * 100 >= a.len() * 99 {
            p99 = d;
            break;
        }
    }
    json!({"samples":a.len(),"rms":(square/a.len()as f64).sqrt(),"mae":abs/a.len()as f64,"max":max,"p99_absolute":p99})
}
pub fn audio(ctx: &MediaContext, path: &Path, start_sample: i64, end_sample: i64) -> Vec<f32> {
    let bytes = decoded(
        ctx,
        path,
        &[
            "-vn".into(),
            "-af".into(),
            format!("atrim=start_sample={start_sample}:end_sample={end_sample}"),
            "-ac".into(),
            "2".into(),
            "-ar".into(),
            "48000".into(),
            "-f".into(),
            "f32le".into(),
        ],
    );
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
pub fn audio_metrics(a: &[f32], b: &[f32]) -> Value {
    assert_eq!(a.len(), b.len());
    let mut square = 0.;
    let mut signal = 0.;
    let mut max = 0f64;
    for (&a, &b) in a.iter().zip(b) {
        let d = a as f64 - b as f64;
        square += d * d;
        signal += (a as f64).powi(2);
        max = max.max(d.abs());
    }
    let rms = (square / a.len().max(1) as f64).sqrt();
    let signal_rms = (signal / a.len().max(1) as f64).sqrt();
    json!({"interleaved_samples":a.len(),"rms":rms,"max":max,"signal_rms":signal_rms,"snr_db":if rms>0.{20.*(signal_rms/rms).log10()}else{999.}})
}
pub fn wait(jobs: &mono_cut_lib::jobs::JobManager, id: &str, bound: Duration) -> Job {
    let start = Instant::now();
    loop {
        let j = jobs.list().into_iter().find(|j| j.id == id).unwrap();
        if j.status != "running" {
            assert_eq!(j.status, "complete", "Job {j:?}");
            return j;
        }
        assert!(start.elapsed() < bound, "job timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
