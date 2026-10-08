//! Silent JSON-lines port for measuring the actual engine from the production
//! TypeScript preview controller. It deliberately does not start Tauri or audio.
use mono_cut_lib::{
    jobs::{JobManager, PreviewIntent},
    media::MediaContext,
    model::*,
    preview,
    storage::ProjectStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
#[path = "../tests/support/stage4b.rs"]
mod fixture;

fn emit(lock: &Mutex<()>, value: Value) {
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value).unwrap();
    writeln!(out).unwrap();
    out.flush().unwrap();
}
fn text<'a>(v: &'a Value, name: &str) -> Result<&'a str, String> {
    v[name]
        .as_str()
        .ok_or_else(|| format!("Missing string {name}"))
}
fn integer(v: &Value, name: &str) -> Result<u64, String> {
    v[name]
        .as_u64()
        .ok_or_else(|| format!("Missing integer {name}"))
}
fn region(v: &Value) -> Result<Option<PreviewRegion>, String> {
    match v.get("region") {
        None | Some(Value::Null) => Ok(None),
        Some(r) => serde_json::from_value(r.clone())
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}
fn identity(
    jobs: &JobManager,
    p: &Project,
    height: u32,
    proxies: bool,
    r: Option<&PreviewRegion>,
) -> Result<String, String> {
    match r {
        Some(r) => jobs.preview_region_identity(p, height, proxies, r),
        None => jobs.preview_identity(p, height, proxies),
    }
}
#[derive(Default, Clone, Copy)]
struct Usage {
    own_rss: u64,
    ffmpeg_rss: u64,
    ffmpeg_count: usize,
    decoder_rss: u64,
    decoder_count: usize,
    combined_rss: u64,
    combined_count: usize,
}
fn decoder_parent() -> Option<u32> {
    std::env::var("MONO_CUT_MEASURE_PARENT_PID")
        .ok()
        .and_then(|p| p.parse().ok())
}
fn update_peak(peak: &mut Usage, now: Usage) {
    peak.own_rss = peak.own_rss.max(now.own_rss);
    peak.ffmpeg_rss = peak.ffmpeg_rss.max(now.ffmpeg_rss);
    peak.ffmpeg_count = peak.ffmpeg_count.max(now.ffmpeg_count);
    peak.decoder_rss = peak.decoder_rss.max(now.decoder_rss);
    peak.decoder_count = peak.decoder_count.max(now.decoder_count);
    peak.combined_rss = peak.combined_rss.max(now.combined_rss);
    peak.combined_count = peak.combined_count.max(now.combined_count);
}
#[cfg(windows)]
fn usage() -> Usage {
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
        fn Process32FirstW(s: isize, e: *mut Entry) -> i32;
        fn Process32NextW(s: isize, e: *mut Entry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(h: isize) -> i32;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(p: isize, c: *mut Counters, n: u32) -> i32;
    }
    unsafe fn rss(pid: u32) -> u64 {
        let p = OpenProcess(0x0400 | 0x0010, 0, pid);
        if p == 0 {
            return 0;
        }
        let mut c: Counters = std::mem::zeroed();
        c.size = std::mem::size_of::<Counters>() as u32;
        let ok = GetProcessMemoryInfo(p, &mut c, c.size);
        CloseHandle(p);
        if ok == 0 {
            0
        } else {
            c.working as u64
        }
    }
    unsafe {
        let pid = std::process::id();
        let mut result = Usage {
            own_rss: rss(pid),
            ..Usage::default()
        };
        let s = CreateToolhelp32Snapshot(2, 0);
        if s == -1 {
            return result;
        }
        let mut e: Entry = std::mem::zeroed();
        e.size = std::mem::size_of::<Entry>() as u32;
        let mut found = Process32FirstW(s, &mut e);
        while found != 0 {
            let n = e.name.iter().position(|c| *c == 0).unwrap_or(260);
            if String::from_utf16_lossy(&e.name[..n]).eq_ignore_ascii_case("ffmpeg.exe") {
                if e.parent == pid {
                    result.ffmpeg_count += 1;
                    result.ffmpeg_rss += rss(e.pid);
                } else if Some(e.parent) == decoder_parent() {
                    result.decoder_count += 1;
                    result.decoder_rss += rss(e.pid);
                }
            }
            found = Process32NextW(s, &mut e);
        }
        CloseHandle(s);
        result.combined_rss = result.ffmpeg_rss + result.decoder_rss;
        result.combined_count = result.ffmpeg_count + result.decoder_count;
        result
    }
}
#[cfg(target_os = "linux")]
fn usage() -> Usage {
    let pid = std::process::id();
    let rss = |pid: u32| {
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
    let mut result = Usage {
        own_rss: rss(pid),
        ..Usage::default()
    };
    for parent in [Some(pid), decoder_parent()].into_iter().flatten() {
        if let Ok(children) = fs::read_to_string(format!("/proc/{parent}/task/{parent}/children")) {
            for child in children
                .split_whitespace()
                .filter_map(|c| c.parse::<u32>().ok())
            {
                if fs::read_to_string(format!("/proc/{child}/comm"))
                    .map(|c| c.trim() == "ffmpeg")
                    .unwrap_or(false)
                {
                    if parent == pid {
                        result.ffmpeg_count += 1;
                        result.ffmpeg_rss += rss(child);
                    } else {
                        result.decoder_count += 1;
                        result.decoder_rss += rss(child);
                    }
                }
            }
        }
    }
    result.combined_rss = result.ffmpeg_rss + result.decoder_rss;
    result.combined_count = result.ffmpeg_count + result.decoder_count;
    result
}
#[cfg(not(any(windows, target_os = "linux")))]
fn usage() -> Usage {
    Usage::default()
}
struct Host {
    ctx: MediaContext,
    jobs: JobManager,
    store: Mutex<ProjectStore>,
    peak: Mutex<Usage>,
}
impl Host {
    fn call(&self, v: Value) -> Result<Value, String> {
        let height = v["height"].as_u64().unwrap_or(540) as u32;
        let proxies = v["useProxies"].as_bool().unwrap_or(false);
        let requested = region(&v)?;
        let project = || {
            self.store
                .lock()
                .map_err(|_| "Project state unavailable".to_string())
                .map(|s| s.get())
        };
        match text(&v, "op")? {
            "get_project" => Ok(json!(project()?)),
            "load_project" => Ok(json!(self
                .store
                .lock()
                .map_err(|_| "Project state unavailable")?
                .open_hydrated(&PathBuf::from(text(&v, "path")?), &self.ctx)?)),
            "edit" => {
                let command: EditCommand =
                    serde_json::from_value(v["command"].clone()).map_err(|e| e.to_string())?;
                Ok(json!(self
                    .store
                    .lock()
                    .map_err(|_| "Project state unavailable")?
                    .apply(command, &self.ctx)?))
            }
            "undo" => Ok(json!(self
                .store
                .lock()
                .map_err(|_| "Project state unavailable")?
                .undo()?)),
            "redo" => Ok(json!(self
                .store
                .lock()
                .map_err(|_| "Project state unavailable")?
                .redo()?)),
            "save" => Ok(json!(self
                .store
                .lock()
                .map_err(|_| "Project state unavailable")?
                .save(&PathBuf::from(text(&v, "path")?))?)),
            "session" => Ok(json!(self.jobs.begin_preview_session())),
            "cancel_intent" => {
                self.jobs
                    .cancel_preview_intent(integer(&v, "session")?, integer(&v, "revision")?)?;
                Ok(Value::Null)
            }
            "identity" => {
                let p = project()?;
                let key = identity(&self.jobs, &p, height, proxies, requested.as_ref())?;
                let program_key = self.jobs.preview_identity(&p, height, proxies)?;
                // The cache key contains region compatibility and source stamps.
                let cached_path =
                    preview::cached_region(&self.ctx, &p, height, &key, requested.as_ref())
                        .map(|p| p.to_string_lossy().into_owned());
                Ok(
                    json!({"key":key,"program_key":program_key,"cached_path":cached_path,"region":requested}),
                )
            }
            "intent" => {
                // Match the Tauri command: hold the current store through
                // identity validation and publication, including delayed calls.
                let store = self.store.lock().map_err(|_| "Project state unavailable")?;
                let key = identity(
                    &self.jobs,
                    &store.get(),
                    height,
                    proxies,
                    requested.as_ref(),
                )?;
                if key != text(&v, "expectedKey")? {
                    return Err(
                        "PREVIEW_IDENTITY_CHANGED: The current program differs from this intent."
                            .into(),
                    );
                }
                Ok(json!(self.jobs.set_preview_intent(
                    integer(&v, "session")?,
                    integer(&v, "revision")?,
                    &key
                )?))
            }
            "render" => {
                let p = project()?;
                let expected = v["expectedKey"].as_str();
                let intent = v
                    .get("intent")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        serde_json::from_value::<PreviewIntent>(v.clone())
                            .map_err(|e| e.to_string())
                    })
                    .transpose()?;
                let job = match (requested, intent.as_ref()) {
                    (Some(r), Some(i)) => self
                        .jobs
                        .preview_region_for_intent(p, height, proxies, r, expected, i)?,
                    (Some(r), None) => self.jobs.preview_region(p, height, proxies, r)?,
                    (None, Some(i)) => self
                        .jobs
                        .preview_for_intent(p, height, proxies, expected, i)?,
                    (None, None) => self.jobs.preview_checked(p, height, proxies, expected)?,
                };
                Ok(json!(job))
            }
            "assets" => {
                self.jobs.set_playback_assets_with_successor(
                    v["previewPath"].as_str().map(PathBuf::from),
                    v["sourcePath"].as_str().map(PathBuf::from),
                    v["successorPreviewPath"].as_str().map(PathBuf::from),
                )?;
                Ok(Value::Null)
            }
            "cancel" => {
                self.jobs.cancel(text(&v, "jobId")?)?;
                Ok(Value::Null)
            }
            "list" => Ok(json!(self.jobs.list())),
            "plan" => {
                let p = project()?;
                let marker = format!("driver-plan-{}", id());
                let destination = self.ctx.cache_dir.join(format!("{marker}.mp4"));
                let plan = preview::compile(
                    &self.ctx,
                    &p,
                    height,
                    proxies,
                    &destination,
                    &marker,
                    requested.as_ref(),
                    None,
                )?;
                let result = json!({"working_region":plan.working_region,"output_frames":plan.output_frames,"input_ranges":plan.input_ranges,"audio_prefix_fallbacks":plan.audio_prefix_fallbacks,"video_seek_checks":plan.video_seek_checks,"video_prefix_fallbacks":plan.video_prefix_fallbacks,"filter_graph":plan.filter_graph,"args":plan.args});
                fixture::cleanup(&self.ctx, &marker);
                Ok(result)
            }
            "proxy" => {
                let p = project()?;
                let m = p
                    .media
                    .iter()
                    .find(|m| Some(m.id.as_str()) == v["mediaId"].as_str())
                    .ok_or("Media missing")?;
                Ok(json!(self.jobs.proxy(m.clone())?))
            }
            "set_proxy" => Ok(json!(self
                .store
                .lock()
                .map_err(|_| "Project state unavailable")?
                .set_proxy(text(&v, "mediaId")?, PathBuf::from(text(&v, "path")?))?)),
            "telemetry" => {
                let now = usage();
                let mut peak = self.peak.lock().unwrap();
                update_peak(&mut peak, now);
                let pins = self.ctx.processes.protected_assets();
                let partials = fs::read_dir(&self.ctx.cache_dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().contains("mono-part"))
                    .count();
                Ok(
                    json!({"sampler_period_ms":20,"decoder_parent_pid":decoder_parent(),"own_rss_bytes":now.own_rss,"ffmpeg_rss_bytes":now.ffmpeg_rss,"ffmpeg_count":now.ffmpeg_count,"own_rss_peak_bytes":peak.own_rss,"ffmpeg_rss_peak_bytes":peak.ffmpeg_rss,"ffmpeg_count_peak":peak.ffmpeg_count,"decoder_ffmpeg_rss_bytes":now.decoder_rss,"decoder_ffmpeg_count":now.decoder_count,"decoder_ffmpeg_rss_peak_bytes":peak.decoder_rss,"decoder_ffmpeg_count_peak":peak.decoder_count,"combined_ffmpeg_rss_peak_bytes":peak.combined_rss,"combined_ffmpeg_count_peak":peak.combined_count,"active_managed_children":self.ctx.processes.active_count(),"managed_spawn_count":self.ctx.processes.total_spawn_count(),"cache_bytes":fixture::disk(&self.ctx.cache_dir),"cache_files":fs::read_dir(&self.ctx.cache_dir).map(|d|d.count()).unwrap_or(0),"partial_count":partials,"preview_ownership":self.jobs.preview_ownership(),"protected_assets":{"files":pins.file_count,"pins":pins.pin_count,"bytes":pins.bytes}}),
                )
            }
            "reset_peak" => {
                *self.peak.lock().unwrap() = usage();
                Ok(Value::Null)
            }
            _ => Err("Unknown operation".into()),
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(),3,"Arguments: project.monocut cache-directory Inter.ttf; set MONO_CUT_FFMPEG and MONO_CUT_FFPROBE");
    let cache_dir = PathBuf::from(&args[1]);
    fs::create_dir_all(&cache_dir).unwrap();
    let ctx = MediaContext {
        ffmpeg: PathBuf::from(std::env::var_os("MONO_CUT_FFMPEG").expect("MONO_CUT_FFMPEG")),
        ffprobe: PathBuf::from(std::env::var_os("MONO_CUT_FFPROBE").expect("MONO_CUT_FFPROBE")),
        font: PathBuf::from(&args[2]),
        cache_dir: cache_dir.clone(),
        processes: Default::default(),
    };
    let out = Arc::new(Mutex::new(()));
    let events = out.clone();
    let jobs = JobManager::new(
        ctx.clone(),
        Arc::new(move |job| emit(&events, json!({"event":"job","job":job}))),
    );
    let preparation = Instant::now();
    let mut store = ProjectStore::new(cache_dir.join("driver-recovery.json"));
    store.open_hydrated(&PathBuf::from(&args[0]), &ctx).unwrap();
    let host = Arc::new(Host {
        ctx,
        jobs,
        store: Mutex::new(store),
        peak: Mutex::new(usage()),
    });
    emit(
        &out,
        json!({"event":"ready","project":host.store.lock().unwrap().get(),"startup_hydration_seconds":preparation.elapsed().as_secs_f64(),"driver_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()),"logical_processors":std::thread::available_parallelism().ok().map(|n|n.get())}),
    );
    let done = Arc::new(AtomicBool::new(false));
    let sample_host = host.clone();
    let sample_done = done.clone();
    let sampler = std::thread::spawn(move || {
        while !sample_done.load(Ordering::Acquire) {
            let now = usage();
            let mut peak = sample_host.peak.lock().unwrap();
            update_peak(&mut peak, now);
            drop(peak);
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let active = Arc::new(AtomicUsize::new(0));
    let mut workers = vec![];
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                emit(&out, json!({"id":null,"error":e.to_string()}));
                continue;
            }
        };
        let id = request["id"].clone();
        if request["op"] == "shutdown" {
            host.jobs.shutdown();
            emit(&out, json!({"id":id,"result":null}));
            break;
        }
        if active.fetch_add(1, Ordering::AcqRel) >= 32 {
            active.fetch_sub(1, Ordering::AcqRel);
            emit(
                &out,
                json!({"id":id,"error":"Driver request bound exceeded"}),
            );
            continue;
        }
        let host = host.clone();
        let out = out.clone();
        let active = active.clone();
        workers.push(std::thread::spawn(move||{let began=Instant::now();let reply=match host.call(request){Ok(result)=>json!({"id":id,"result":result,"backend_rpc_seconds":began.elapsed().as_secs_f64()}),Err(error)=>json!({"id":id,"error":error,"backend_rpc_seconds":began.elapsed().as_secs_f64()})};emit(&out,reply);active.fetch_sub(1,Ordering::AcqRel);}));
        // Finished RPC handles carry no engine children; periodically reap them.
        if workers.len() > 64 {
            let mut pending = vec![];
            for worker in workers.drain(..) {
                if worker.is_finished() {
                    worker.join().unwrap();
                } else {
                    pending.push(worker);
                }
            }
            workers = pending;
        }
    }
    host.jobs.shutdown();
    for worker in workers {
        worker.join().unwrap();
    }
    done.store(true, Ordering::Release);
    sampler.join().unwrap();
}
