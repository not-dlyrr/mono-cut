//! Reproducible, silent engine benchmark. Explicitly ignored in ordinary tests.
//! Run with MONO_CUT_BENCH_DIR pointing outside the published source checkout.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    processes::ProcessRegistry,
    storage::{self, ProjectStore},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut h = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    format!("{:x}", h.finalize())
}
fn disk(path: &Path) -> u64 {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}
fn stopping(ctx: &MediaContext) -> bool {
    ctx.cache_dir
        .parent()
        .map(|dir| dir.join("stop.request").exists())
        .unwrap_or(false)
}
#[derive(Clone, Copy, Default)]
struct Memory {
    self_rss: u64,
    ffmpeg_rss: u64,
    ffmpeg_count: usize,
}

#[cfg(windows)]
fn memory() -> Memory {
    #[repr(C)]
    struct Entry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        name: [u16; 260],
    }
    #[repr(C)]
    struct Counters {
        size: u32,
        faults: u32,
        peak: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut Entry) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut Entry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, bytes: u32) -> i32;
    }
    unsafe fn rss(pid: u32) -> u64 {
        let handle = OpenProcess(0x0400 | 0x0010, 0, pid);
        if handle == 0 {
            return 0;
        }
        let mut counters: Counters = std::mem::zeroed();
        counters.size = std::mem::size_of::<Counters>() as u32;
        let success = GetProcessMemoryInfo(handle, &mut counters, counters.size);
        CloseHandle(handle);
        if success == 0 {
            0
        } else {
            counters.working as u64
        }
    }
    unsafe {
        let own = std::process::id();
        let mut result = Memory {
            self_rss: rss(own),
            ..Memory::default()
        };
        let snapshot = CreateToolhelp32Snapshot(2, 0);
        if snapshot == -1 {
            return result;
        }
        let mut entry: Entry = std::mem::zeroed();
        entry.size = std::mem::size_of::<Entry>() as u32;
        let mut found = Process32FirstW(snapshot, &mut entry);
        while found != 0 {
            let end = entry.name.iter().position(|c| *c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&entry.name[..end]);
            if entry.parent == own && name.eq_ignore_ascii_case("ffmpeg.exe") {
                result.ffmpeg_count += 1;
                result.ffmpeg_rss += rss(entry.pid);
            }
            found = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        result
    }
}
#[cfg(target_os = "linux")]
fn memory() -> Memory {
    let own = std::process::id();
    let rss = |pid: u32| -> u64 {
        fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0)
            * 1024
    };
    let mut result = Memory {
        self_rss: rss(own),
        ..Memory::default()
    };
    if let Ok(children) = fs::read_to_string(format!("/proc/{own}/task/{own}/children")) {
        for pid in children
            .split_whitespace()
            .filter_map(|p| p.parse::<u32>().ok())
        {
            if fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|n| n.trim() == "ffmpeg")
                .unwrap_or(false)
            {
                result.ffmpeg_count += 1;
                result.ffmpeg_rss += rss(pid);
            }
        }
    }
    result
}
#[cfg(not(any(windows, target_os = "linux")))]
fn memory() -> Memory {
    Memory::default()
}
fn context(dir: &Path) -> MediaContext {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    let tool = |variable: &str, name: &str| {
        std::env::var_os(variable)
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
fn project(ctx: &MediaContext, source: &Path, clip_frames: i64) -> Project {
    let mut p = Project::new(
        "Preview reuse baseline".into(),
        1920,
        1080,
        Rational::new(30, 1),
    );
    let m = media::probe(ctx, source).unwrap();
    let mid = m.id.clone();
    p.media.push(m);
    p.tracks.clear();
    for track_index in 0..4 {
        let tid = id();
        p.tracks.push(Track {
            id: tid.clone(),
            name: format!("Video{}", track_index + 1),
            kind: "video".into(),
            muted: false,
            hidden: false,
            locked: false,
        });
        for clip_index in 0..5 {
            edit::apply(
                &mut p,
                EditCommand::AddClip {
                    media_id: mid.clone(),
                    track_id: tid.clone(),
                    start: clip_index * clip_frames,
                    source_in: None,
                    duration: Some(clip_frames),
                },
            )
            .unwrap();
            let cid = p.clips.last().unwrap().id.clone();
            edit::apply(&mut p,EditCommand::UpdateClip{id:cid,patch:json!({"transform":{"scale":0.5,"x":(track_index%2) as f64*960.,"y":(track_index/2) as f64*540.}})}).unwrap();
        }
    }
    p.validate().unwrap();
    assert_eq!(p.length(), 5 * clip_frames);
    assert_eq!(p.clips.len(), 20);
    assert_eq!(p.tracks.len(), 4);
    p
}
fn snapshot_sources() -> Value {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let names = [
        "Cargo.toml",
        "Cargo.lock",
        "src/model.rs",
        "src/edit.rs",
        "src/media.rs",
        "src/render.rs",
        "src/jobs.rs",
        "src/processes.rs",
        "src/preview.rs",
        "src/lib.rs",
    ];
    Value::Object(
        names
            .into_iter()
            .filter_map(|name| {
                let path = root.join(name);
                path.is_file().then(|| {
                    (
                        name.to_string(),
                        json!({"sha256":hash(&path),"bytes":fs::metadata(path).unwrap().len()}),
                    )
                })
            })
            .collect(),
    )
}
fn save_report(dir: &Path, report: &Value) {
    fs::write(
        dir.join("benchmark-results.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}
fn wait_case(
    ctx: &MediaContext,
    jobs: &JobManager,
    p: &Project,
    label: &str,
    proxies: bool,
    bound: Duration,
) -> Value {
    let before = memory();
    let disk_before = disk(&ctx.cache_dir);
    let spawned_before = ctx.processes.total_spawn_count();
    let start = Instant::now();
    let job = jobs.preview(p.clone(), 540, proxies).unwrap();
    let reply = start.elapsed();
    let mut peak = before;
    let mut progress = job.progress;
    let mut graph_seen = false;
    let mut part_seen = false;
    let mut heartbeat = Instant::now();
    let mut cancelled_at = None;
    let mut cancel_reason = None;
    let final_job = loop {
        let current = jobs.list().into_iter().find(|j| j.id == job.id).unwrap();
        progress = progress.max(current.progress);
        let usage = memory();
        peak.self_rss = peak.self_rss.max(usage.self_rss);
        peak.ffmpeg_rss = peak.ffmpeg_rss.max(usage.ffmpeg_rss);
        peak.ffmpeg_count = peak.ffmpeg_count.max(usage.ffmpeg_count);
        for entry in fs::read_dir(&ctx.cache_dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            graph_seen |= name == format!("{}-filters.txt", job.id);
            part_seen |= name.starts_with(&format!(".mono-part-{}", job.id));
        }
        if current.status != "running" {
            break current;
        }
        if cancelled_at.is_none() && (start.elapsed() >= bound || stopping(ctx)) {
            cancel_reason = Some(if stopping(ctx) {
                "stop-file"
            } else {
                "timeout"
            });
            cancelled_at = Some(start.elapsed().as_secs_f64());
            jobs.cancel(&job.id).unwrap();
        }
        if heartbeat.elapsed() >= Duration::from_secs(10) {
            eprintln!("BENCH {label}: elapsed={:.3}s progress={progress:.4} ffmpegRSS={}MiB selfRSS={}MiB",start.elapsed().as_secs_f64(),usage.ffmpeg_rss/1048576,usage.self_rss/1048576);
            heartbeat = Instant::now();
        }
        assert!(
            cancelled_at.is_none() || start.elapsed() < bound + Duration::from_secs(10),
            "Cancellation exceeded its bound"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let after = memory();
    let output = final_job.path.as_ref().map(PathBuf::from);
    let terminal_seconds = if job.status != "running" {
        reply.as_secs_f64()
    } else {
        start.elapsed().as_secs_f64()
    };
    let value = json!({"label":label,"sequence_frames":p.length(),"height":540,"use_proxies":proxies,"api_reply_status":job.status,"job_status":final_job.status,"error":final_job.error,"api_reply_seconds":reply.as_secs_f64(),"request_to_terminal_seconds":terminal_seconds,"completion_observed_seconds":start.elapsed().as_secs_f64(),"cancel_requested_seconds":cancelled_at,"cancel_reason":cancel_reason,"max_progress":progress,"managed_spawns":ctx.processes.total_spawn_count()-spawned_before,"ffmpeg_seen":peak.ffmpeg_count>0,"peak_ffmpeg_count":peak.ffmpeg_count,"graph_seen":graph_seen,"partial_seen":part_seen,"output":output.as_ref().map(|p|p.file_name().unwrap_or_default().to_string_lossy().into_owned()),"output_sha256":output.as_ref().filter(|p|p.is_file()).map(|p|hash(p)),"output_bytes":output.as_ref().and_then(|p|fs::metadata(p).ok().map(|m|m.len())),"disk_before_bytes":disk_before,"disk_after_bytes":disk(&ctx.cache_dir),"self_rss_before_bytes":before.self_rss,"self_rss_after_bytes":after.self_rss,"self_rss_peak_bytes":peak.self_rss,"ffmpeg_rss_peak_bytes":peak.ffmpeg_rss,"protected_assets":{"files":ctx.processes.protected_assets().file_count,"pins":ctx.processes.protected_assets().pin_count,"bytes":ctx.processes.protected_assets().bytes}});
    eprintln!("BENCH RESULT {}", value);
    value
}
fn edit_project(p: &Project, value: f64) -> Project {
    let mut edited = p.clone();
    let cid = edited.clips[0].id.clone();
    edit::apply(
        &mut edited,
        EditCommand::UpdateClip {
            id: cid,
            patch: json!({"brightness":value}),
        },
    )
    .unwrap();
    edited
}
fn edited_pixel_delta(ctx: &MediaContext, baseline: &str, edited: &str) -> f64 {
    let decode = |name: &str| {
        let path = ctx.cache_dir.join(name);
        let _pin = ctx.processes.pin_file(&path).unwrap();
        let mut process = media::command(&ctx.ffmpeg);
        process
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-an",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "pipe:1",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = ctx.processes.spawn(&mut process).unwrap();
        let mut output = child.take_stdout().unwrap();
        let mut errors = child.take_stderr().unwrap();
        let drain = std::thread::spawn(move || {
            let mut bytes = vec![];
            errors.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut frame = vec![];
        output.read_to_end(&mut frame).unwrap();
        let status = child.wait().unwrap();
        let errors = drain.join().unwrap();
        assert!(status.success(), "{}", String::from_utf8_lossy(&errors));
        assert_eq!(frame.len(), 960 * 540);
        frame
    };
    let a = decode(baseline);
    let b = decode(edited);
    let mut delta = 0.;
    for y in 180..300 {
        for x in 300..600 {
            delta += b[y * 960 + x] as f64 - a[y * 960 + x] as f64;
        }
    }
    delta / (120. * 300.)
}

#[test]
#[ignore = "explicit silent 1080p performance benchmark; may run for several minutes"]
fn review_workload_cold_reuse_edits_and_burst_cancellation() {
    let dir = std::env::var_os("MONO_CUT_BENCH_DIR")
        .map(PathBuf::from)
        .expect("Set MONO_CUT_BENCH_DIR to a disposable directory outside the source checkout");
    fs::create_dir_all(&dir).unwrap();
    let ctx = context(&dir);
    media::ensure_cache(&ctx).unwrap();
    let mode = std::env::var("MONO_CUT_BENCH_MODE").unwrap_or_else(|_| "all".into());
    assert!(["small", "large", "all"].contains(&mode.as_str()));
    let source = dir.join("synthetic1080p12s.mp4");
    let generation = Instant::now();
    if !source.exists() {
        media::run_ffmpeg(
            &ctx,
            &[
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=1920x1080:rate=30:duration=12".into(),
                "-an".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "ultrafast".into(),
                "-crf".into(),
                "30".into(),
                "-threads".into(),
                "2".into(),
                source.to_string_lossy().into(),
            ],
        )
        .unwrap();
    }
    let ffmpeg_version = media::command(&ctx.ffmpeg)
        .arg("-version")
        .output()
        .unwrap();
    let mut report = json!({"started_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),"mode":mode,"benchmark_executable_sha256":hash(&std::env::current_exe().unwrap()),"logical_processors":std::thread::available_parallelism().ok().map(|n|n.get()),"source_hashes_before":snapshot_sources(),"ffmpeg_sha256":hash(&ctx.ffmpeg),"ffprobe_sha256":hash(&ctx.ffprobe),"ffmpeg_version":String::from_utf8_lossy(&ffmpeg_version.stdout).lines().next(),"fixture":{"bytes":fs::metadata(&source).unwrap().len(),"sha256":hash(&source),"generation_or_existing_lookup_seconds":generation.elapsed().as_secs_f64(),"duration_seconds":12,"width":1920,"height":1080,"fps":30,"audio":false,"encoding":"libx264 ultrafast CRF30 threads2","note":"Exact review offsets x=0/960,y=0/540 at scale0.5 are preserved; under centered transform semantics portions extend offcanvas."},"cases":[]});
    save_report(&dir, &report);
    let jobs = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    for (prefix, clip_frames, bound) in [
        ("small", 18, Duration::from_secs(90)),
        ("large", 360, Duration::from_secs(480)),
    ] {
        if stopping(&ctx) {
            break;
        }
        if mode != "all" && mode != prefix {
            continue;
        }
        let p = project(&ctx, &source, clip_frames);
        let project_path = dir.join(format!("{prefix}-sequence.monocut"));
        storage::write_atomic(&project_path, &p).unwrap();
        let mut store = ProjectStore::new(dir.join(format!("{prefix}-recovery.json")));
        let p = store.open(&project_path).unwrap();
        let cold = wait_case(
            &ctx,
            &jobs,
            &p,
            &format!("{prefix}-cold-original"),
            false,
            bound,
        );
        let complete = cold["job_status"] == "complete";
        let baseline_output = cold["output"].clone();
        let baseline_hash = cold["output_sha256"].clone();
        report["cases"].as_array_mut().unwrap().push(cold);
        save_report(&dir, &report);
        if stopping(&ctx) {
            break;
        }
        if !complete {
            continue;
        }
        if stopping(&ctx) {
            break;
        }
        for label in ["identical-repeat", "metadata-only"] {
            let case_project = if label == "metadata-only" {
                let mut q = p.clone();
                q.name = "Display-only project rename".into();
                q.tracks[0].name = "Display-only track rename".into();
                q.clips[0].name = "Display-only clip rename".into();
                q.markers.push(Marker {
                    id: id(),
                    frame: 1,
                    name: "Metadata marker".into(),
                });
                q
            } else {
                p.clone()
            };
            let case = wait_case(
                &ctx,
                &jobs,
                &case_project,
                &format!("{prefix}-{label}"),
                false,
                bound,
            );
            assert_eq!(
                case["managed_spawns"], 0,
                "An equivalent request must not spawn any media child"
            );
            assert_eq!(case["output"], baseline_output);
            assert_eq!(case["output_sha256"], baseline_hash);
            report["cases"].as_array_mut().unwrap().push(case);
            save_report(&dir, &report);
        }
        let edited = store
            .apply(
                EditCommand::UpdateClip {
                    id: p.clips[0].id.clone(),
                    patch: json!({"brightness": 0.05}),
                },
                &ctx,
            )
            .unwrap();
        let mut case = wait_case(
            &ctx,
            &jobs,
            &edited,
            &format!("{prefix}-actual-brightness-edit"),
            false,
            bound,
        );
        if case["job_status"] == "complete" {
            assert_ne!(case["output"], baseline_output);
            assert_ne!(case["output_sha256"], baseline_hash);
            let delta = edited_pixel_delta(
                &ctx,
                baseline_output.as_str().unwrap(),
                case["output"].as_str().unwrap(),
            );
            assert!(
                delta > 4.,
                "Brightness edit did not change visible footage: luma delta {delta}"
            );
            case["independent_first_frame_luma_delta"] = json!(delta);
        }
        report["cases"].as_array_mut().unwrap().push(case);
        save_report(&dir, &report);
        if stopping(&ctx) {
            break;
        }
        let restored = store.undo().unwrap();
        assert_eq!(restored, p);
        let undo = wait_case(
            &ctx,
            &jobs,
            &restored,
            &format!("{prefix}-undo-after-rendered-edit"),
            false,
            bound,
        );
        assert_eq!(undo["managed_spawns"], 0);
        assert_eq!(undo["output"], baseline_output);
        assert_eq!(undo["output_sha256"], baseline_hash);
        report["cases"].as_array_mut().unwrap().push(undo);
        save_report(&dir, &report);
        if prefix == "small" {
            let mut burst = vec![];
            let mut last = None;
            for value in [0.1, 0.2, 0.3] {
                let start = Instant::now();
                let job = jobs.preview(edit_project(&p, value), 540, false).unwrap();
                burst.push(json!({"value":value,"job_id":job.id,"api_reply_seconds":start.elapsed().as_secs_f64()}));
                last = Some(job.id);
                std::thread::sleep(Duration::from_millis(300));
            }
            let stop = Instant::now();
            jobs.cancel(last.as_ref().unwrap()).unwrap();
            while jobs
                .list()
                .iter()
                .any(|j| j.kind == "preview" && j.status == "running")
            {
                assert!(stop.elapsed() < Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(20));
            }
            report["burst"] = json!({"requests":burst,"last_cancel_to_stopped_seconds":stop.elapsed().as_secs_f64(),"managed_active_after":ctx.processes.active_count(),"remaining_partials":fs::read_dir(&ctx.cache_dir).unwrap().flatten().filter(|e|e.file_name().to_string_lossy().contains("mono-part")).count()});
            save_report(&dir, &report);
        }
        if prefix == "large" && std::env::var("MONO_CUT_BENCH_PROXY").as_deref() == Ok("1") {
            let begin = Instant::now();
            let proxy = jobs.proxy(p.media[0].clone()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(180);
            let completed = loop {
                let current = jobs.list().into_iter().find(|j| j.id == proxy.id).unwrap();
                if current.status != "running" {
                    break current;
                }
                if Instant::now() > deadline || stopping(&ctx) {
                    jobs.cancel(&proxy.id).unwrap();
                }
                assert!(Instant::now() < deadline + Duration::from_secs(10));
                std::thread::sleep(Duration::from_millis(100));
            };
            let mut proxied = p.clone();
            if let Some(path) = completed.path.as_ref() {
                proxied.media[0].proxy = Some(path.clone());
                proxied.media[0].proxy_timing_version = Some(NORMALIZED_TIMING_VERSION);
            }
            report["proxy_generation"] = json!({"seconds":begin.elapsed().as_secs_f64(),"status":completed.status,"bytes":completed.path.as_ref().and_then(|p|fs::metadata(p).ok().map(|m|m.len()))});
            save_report(&dir, &report);
            if completed.status == "complete" && !stopping(&ctx) {
                let proxy_pin = ctx
                    .processes
                    .pin_file(completed.path.as_ref().unwrap())
                    .unwrap();
                let case = wait_case(
                    &ctx,
                    &jobs,
                    &proxied,
                    "large-warm-proxy-original-project",
                    true,
                    bound,
                );
                report["cases"].as_array_mut().unwrap().push(case);
                save_report(&dir, &report);
                drop(proxy_pin);
            }
        }
    }
    jobs.set_playback_assets(None, None).unwrap();
    jobs.shutdown();
    drop(jobs);
    report["source_hashes_after"] = snapshot_sources();
    report["source_hashes_unchanged"] =
        json!(report["source_hashes_before"] == report["source_hashes_after"]);
    report["final_memory"] = json!({"self_rss_bytes":memory().self_rss,"ffmpeg_rss_bytes":memory().ffmpeg_rss,"active_managed_children":ctx.processes.active_count(),"cache_bytes":disk(&ctx.cache_dir),"protected_assets":ctx.processes.protected_assets().pin_count});
    save_report(&dir, &report);
    eprintln!(
        "BENCH report={}",
        dir.join("benchmark-results.json").display()
    );
}
