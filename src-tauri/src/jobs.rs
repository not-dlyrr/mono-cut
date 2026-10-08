use crate::{
    media::{self, MediaContext},
    model::*,
    preview,
    processes::AssetPin,
    render,
};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

struct Entry {
    job: Job,
    cancel: Arc<AtomicBool>,
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn queued_render_cannot_launch_after_cached_intent_supersedes_it() {
        let dir = TempDir::new().unwrap();
        let ctx = MediaContext {
            ffmpeg: std::env::var_os("MONO_CUT_FFMPEG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("ffmpeg")),
            ffprobe: std::env::var_os("MONO_CUT_FFPROBE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("ffprobe")),
            font: PathBuf::from("unused-font.ttf"),
            cache_dir: dir.path().join("cache"),
            processes: Default::default(),
        };
        let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
        let a = Project::new("A".into(), 160, 90, Rational::new(30, 1));
        let mut b = a.clone();
        b.width = 320;
        let key_a = manager.preview_identity(&a, 180, false).unwrap();
        let key_b = manager.preview_identity(&b, 180, false).unwrap();
        let session = manager.begin_preview_session();
        let intent_b = manager.set_preview_intent(session, 1, &key_b).unwrap();
        let gate = manager.preview_gate.lock().unwrap();
        let worker = manager.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = worker.preview_for_intent(b, 180, false, Some(&key_b), &intent_b);
            result_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // Keep launch admission closed while the render request is pending.
        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        manager.set_preview_intent(session, 2, &key_a).unwrap();
        drop(gate);
        let rejected = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err();
        handle.join().unwrap();
        assert!(rejected.starts_with("PREVIEW_IDENTITY_CHANGED:"));
        assert!(manager.list().is_empty());
        assert_eq!(ctx.processes.total_spawn_count(), 0);
        assert_eq!(ctx.processes.active_count(), 0);
        assert_eq!(ctx.processes.protected_assets().pin_count, 0);
    }

    #[test]
    fn committed_user_outputs_survive_late_cancel_but_unique_previews_are_discarded() {
        for kind in ["export", "proxy", "preview"] {
            let dir = TempDir::new().unwrap();
            let ctx = MediaContext {
                ffmpeg: std::env::var_os("MONO_CUT_FFMPEG")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("ffmpeg")),
                ffprobe: std::env::var_os("MONO_CUT_FFPROBE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("ffprobe")),
                font: PathBuf::from("unused-font.ttf"),
                cache_dir: dir.path().join("cache"),
                processes: Default::default(),
            };
            let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
            let destination = dir.path().join(format!("{kind}.mkv"));
            if kind != "preview" {
                fs::write(&destination, b"previous output").unwrap();
            }
            let (committed_tx, committed_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let completion: PreviewCompletion = Box::new(move |_| {
                committed_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|e| e.to_string())
            });
            let job = manager
                .launch_pinned(
                    kind,
                    destination.clone(),
                    0.1,
                    None,
                    vec![],
                    Some(completion),
                    move |_, out| {
                        Ok(vec![
                            "-f".into(),
                            "lavfi".into(),
                            "-i".into(),
                            "color=c=blue:size=160x90:rate=30:duration=0.1".into(),
                            "-c:v".into(),
                            "ffv1".into(),
                            "-progress".into(),
                            "pipe:1".into(),
                            out.to_string_lossy().into_owned(),
                        ])
                    },
                )
                .unwrap();
            committed_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(
                destination.is_file(),
                "The commit must happen before the cancellation race"
            );
            assert_ne!(fs::read(&destination).unwrap(), b"previous output");
            manager.cancel(&job.id).unwrap();
            release_tx.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let terminal = loop {
                let latest = manager.list().into_iter().find(|j| j.id == job.id).unwrap();
                if latest.status != "running" {
                    break latest;
                }
                assert!(
                    Instant::now() < deadline,
                    "Worker did not finish cancellation"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            if kind == "preview" {
                assert_eq!(terminal.status, "cancelled");
                assert!(!destination.exists());
            } else {
                assert_eq!(terminal.status, "complete");
                assert_eq!(
                    terminal.path.as_deref(),
                    Some(destination.to_string_lossy().as_ref())
                );
                let actual = media::probe(&ctx, &destination).unwrap();
                assert_eq!((actual.width, actual.height), (160, 90));
            }
            manager.shutdown();
            assert_eq!(ctx.processes.active_count(), 0);
            assert_eq!(fs::read_dir(&ctx.cache_dir).unwrap().count(), 0);
        }
    }
}
#[derive(Default)]
struct PreviewState {
    latest_key: Option<String>,
    completed_pin: Option<AssetPin>,
    playback_preview: Option<AssetPin>,
    playback_source: Option<AssetPin>,
    prepared_source: Option<AssetPin>,
    ui_session: Option<u64>,
    ui_revision: u64,
    ui_key: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PreviewIntent {
    pub session: u64,
    pub revision: u64,
    pub key: String,
}
type PreviewCompletion = Box<dyn Fn(&Path) -> Result<(), String> + Send>;
#[derive(Clone)]
pub struct JobManager {
    ctx: MediaContext,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    callback: Arc<dyn Fn(Job) + Send + Sync>,
    preview_gate: Arc<Mutex<()>>,
    preview_state: Arc<Mutex<PreviewState>>,
    preview_sequence: Arc<AtomicU64>,
    source_sequence: Arc<AtomicU64>,
    preview_sessions: Arc<AtomicU64>,
}
impl JobManager {
    pub fn new(ctx: MediaContext, callback: Arc<dyn Fn(Job) + Send + Sync>) -> Self {
        Self {
            ctx,
            entries: Arc::new(Mutex::new(HashMap::new())),
            callback,
            preview_gate: Default::default(),
            preview_state: Default::default(),
            preview_sequence: Default::default(),
            source_sequence: Default::default(),
            preview_sessions: Default::default(),
        }
    }
    pub fn list(&self) -> Vec<Job> {
        let mut jobs: Vec<_> = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|e| e.job.clone())
            .collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        jobs
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let entries = self.entries.lock().map_err(|_| "Job state unavailable")?;
        let entry = entries.get(id).ok_or("Job not found")?;
        if entry.job.status == "running" {
            entry.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        {
            let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            for entry in entries
                .values()
                .filter(|entry| entry.job.status == "running")
            {
                entry.cancel.store(true, Ordering::Release);
            }
        }
        self.ctx.processes.shutdown();
        // Let job workers finish cancellation and remove graph/title sidecars before exit.
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.list().iter().any(|job| job.status == "running") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn update(
        &self,
        id: &str,
        status: &str,
        progress: f64,
        path: Option<String>,
        error: Option<String>,
    ) {
        let job = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let Some(e) = entries.get_mut(id) else {
                return;
            };
            e.job.status = status.into();
            e.job.progress = progress.clamp(0., 1.);
            e.job.path = path;
            e.job.error = error;
            e.job.clone()
        };
        (self.callback)(job)
    }
    fn launch_pinned<F>(
        &self,
        kind: &str,
        destination: PathBuf,
        duration: f64,
        preview_key: Option<String>,
        pins: Vec<AssetPin>,
        completion: Option<PreviewCompletion>,
        build: F,
    ) -> Result<Job, String>
    where
        F: FnOnce(&str, &Path) -> Result<Vec<String>, String> + Send + 'static,
    {
        if self.ctx.processes.is_closing() {
            return Err("Media processing cancelled: application is closing".into());
        }
        media::ensure_cache(&self.ctx)?;
        if !destination.parent().map(|p| p.is_dir()).unwrap_or(false) {
            return Err("Output folder does not exist".into());
        }
        let id = id();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            id: id.clone(),
            kind: kind.into(),
            status: "running".into(),
            progress: 0.,
            path: None,
            error: None,
            preview_key,
        };
        {
            let mut entries = self.entries.lock().map_err(|_| "Job state unavailable")?;
            if entries
                .values()
                .filter(|e| e.job.status == "running" && !e.cancel.load(Ordering::Acquire))
                .count()
                >= 2
            {
                return Err(
                    "Two media jobs are already running; cancel one or wait for completion".into(),
                );
            }
            if entries.len() >= 100 {
                entries.retain(|_, e| e.job.status == "running");
            }
            entries.insert(
                id.clone(),
                Entry {
                    job: job.clone(),
                    cancel: cancel.clone(),
                },
            );
        }
        (self.callback)(job.clone());
        let manager = self.clone();
        let completed_preview_key = job.preview_key.clone();
        let discard_on_late_cancel = kind == "preview";
        std::thread::spawn(move || {
            let _pins = pins;
            let ext = destination
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("mp4");
            let part = destination.with_file_name(format!(".mono-part-{id}.{ext}"));
            let temporary = match manager.ctx.processes.temporary_file(part.clone()) {
                Ok(temporary) => temporary,
                Err(_) => {
                    manager.update(&id, "cancelled", 0., None, None);
                    return;
                }
            };
            let outcome = (|| {
                if cancel.load(Ordering::Acquire) {
                    return Ok(false);
                }
                let args = build(&id, &part)?;
                if cancel.load(Ordering::Acquire) {
                    return Ok(false);
                }
                let mut process = media::command(&manager.ctx.ffmpeg);
                process
                    .args(args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let child = manager
                    .ctx
                    .processes
                    .spawn(&mut process)
                    .map_err(|e| format!("Could not start FFmpeg: {e}"))?;
                let stdout = child.take_stdout().unwrap();
                let stderr = child.take_stderr().unwrap();
                let (send, receive) = mpsc::channel();
                let progress_reader = std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        if let Some(v) = line.strip_prefix("out_time_us=") {
                            if let Ok(us) = v.parse::<f64>() {
                                let _ = send.send(us / 1e6);
                            }
                        }
                    }
                });
                let error_reader = std::thread::spawn(move || {
                    let mut reader = stderr;
                    let mut tail = vec![];
                    let mut block = [0u8; 4096];
                    loop {
                        let n = reader.read(&mut block).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        tail.extend_from_slice(&block[..n]);
                        if tail.len() > 65536 {
                            tail.drain(..tail.len() - 65536);
                        }
                    }
                    tail
                });
                let mut last = 0.;
                let status = loop {
                    if cancel.load(Ordering::Acquire) {
                        let _ = child.kill();
                        let status = child.wait().map_err(|e| e.to_string())?;
                        break status;
                    }
                    while let Ok(time) = receive.try_recv() {
                        let progress = (time / duration.max(0.001)).clamp(0., 0.995);
                        if progress - last >= 0.005 {
                            last = progress;
                            manager.update(&id, "running", progress, None, None);
                        }
                    }
                    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                        break status;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                };
                let _ = progress_reader.join();
                let error = error_reader.join().unwrap_or_default();
                if cancel.load(Ordering::Acquire) {
                    return Ok(false);
                }
                if !status.success() {
                    return Err(format!("FFmpeg failed: {}", media::diagnostic(&error)));
                }
                if !part.is_file() || fs::metadata(&part).map(|m| m.len()).unwrap_or(0) == 0 {
                    return Err("Encoder produced an empty output file".into());
                }
                if cancel.load(Ordering::Acquire) {
                    return Ok(false);
                }
                fs::rename(&part, &destination)
                    .map_err(|e| format!("Could not finalize output: {e}"))?;
                if let Some(completion) = completion {
                    if let Err(error) = completion(&destination) {
                        let _ = fs::remove_file(&destination);
                        return Err(error);
                    }
                }
                // User exports are committed once rename succeeds. A late cancel
                // must not remove the new destination after replacing an old file.
                if discard_on_late_cancel && cancel.load(Ordering::Acquire) {
                    let _ = fs::remove_file(&destination);
                    return Ok(false);
                }
                Ok(true)
            })();
            if !matches!(outcome, Ok(true)) {
                if let Some(key) = completed_preview_key.as_deref() {
                    preview::discard(&manager.ctx, key, &destination);
                    let mut state = manager
                        .preview_state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if state
                        .completed_pin
                        .as_ref()
                        .is_some_and(|pin| pin.path() == destination)
                    {
                        state.completed_pin = None;
                    }
                }
            }
            let _ = fs::remove_file(&part);
            drop(temporary);
            // Graphs and title sidecars are per-job and never part of a distributed project.
            if let Ok(entries) = fs::read_dir(&manager.ctx.cache_dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("{id}-"))
                        && (name.ends_with("filters.txt") || name.contains("-title-"))
                    {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
            let _ = media::prune_cache(&manager.ctx, 2 * 1024 * 1024 * 1024);
            match outcome {
                Ok(true) => manager.update(
                    &id,
                    "complete",
                    1.,
                    Some(destination.to_string_lossy().to_string()),
                    None,
                ),
                Ok(false) => manager.update(&id, "cancelled", 0., None, None),
                Err(error) => manager.update(&id, "failed", 0., None, Some(error)),
            }
        });
        Ok(job)
    }
    pub fn preview(&self, p: Project, height: u32, use_proxies: bool) -> Result<Job, String> {
        self.preview_checked(p, height, use_proxies, None)
    }
    pub fn preview_identity(
        &self,
        p: &Project,
        height: u32,
        use_proxies: bool,
    ) -> Result<String, String> {
        preview::identity(&self.ctx, p, height, use_proxies)
    }
    pub fn begin_preview_session(&self) -> u64 {
        let mut state = self.preview_state.lock().unwrap_or_else(|e| e.into_inner());
        let session = self.preview_sessions.fetch_add(1, Ordering::AcqRel) + 1;
        state.ui_session = Some(session);
        state.ui_revision = 0;
        state.ui_key = None;
        state.latest_key = None;
        state.completed_pin = None;
        self.preview_sequence.fetch_add(1, Ordering::AcqRel);
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        for entry in entries
            .values()
            .filter(|entry| entry.job.kind == "preview" && entry.job.status == "running")
        {
            entry.cancel.store(true, Ordering::Release);
        }
        session
    }
    /// Publish current UI intent even when it selects an already loaded cache.
    /// Ordering is independent of RPC arrival and late render replies.
    pub fn set_preview_intent(
        &self,
        session: u64,
        revision: u64,
        key: &str,
    ) -> Result<PreviewIntent, String> {
        if revision == 0
            || revision > 9_007_199_254_740_991
            || key.len() != 64
            || !key.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid preview intent".into());
        }
        let mut state = self
            .preview_state
            .lock()
            .map_err(|_| "Preview state unavailable")?;
        if state.ui_session != Some(session)
            || (revision <= state.ui_revision && state.ui_key.as_deref() != Some(key))
        {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this request.".into(),
            );
        }
        state.ui_revision = state.ui_revision.max(revision);
        state.ui_key = Some(key.into());
        if state.latest_key.as_deref() != Some(key) {
            self.preview_sequence.fetch_add(1, Ordering::AcqRel);
        }
        state.latest_key = Some(key.into());
        // Hold the admission state through cancellation. A request cannot insert
        // a differing-key job after this intent has already scanned the jobs.
        let entries = self.entries.lock().map_err(|_| "Job state unavailable")?;
        for entry in entries.values().filter(|entry| {
            entry.job.kind == "preview"
                && entry.job.status == "running"
                && entry.job.preview_key.as_deref() != Some(key)
        }) {
            entry.cancel.store(true, Ordering::Release);
        }
        Ok(PreviewIntent {
            session,
            revision: state.ui_revision,
            key: key.into(),
        })
    }
    fn preview_request_current(
        &self,
        state: &PreviewState,
        sequence: u64,
        key: &str,
        intent: Option<&PreviewIntent>,
    ) -> bool {
        match intent {
            Some(intent) => {
                state.ui_session == Some(intent.session)
                    && intent.revision > 0
                    && intent.revision <= state.ui_revision
                    && state.ui_key.as_deref() == Some(key)
                    && intent.key == key
                    && state.latest_key.as_deref() == Some(key)
            }
            None => {
                state.ui_session.is_none()
                    && (sequence == self.preview_sequence.load(Ordering::Acquire)
                        || state.latest_key.as_deref() == Some(key))
            }
        }
    }
    /// Empty programs have no replacement render to cancel their obsolete worker.
    pub fn clear_preview(&self) {
        let mut state = self.preview_state.lock().unwrap_or_else(|e| e.into_inner());
        self.preview_sequence.fetch_add(1, Ordering::AcqRel);
        state.latest_key = None;
        state.ui_key = None;
        state.completed_pin = None;
        // Keep invalidation and job cancellation atomic with launch admission.
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        for entry in entries
            .values()
            .filter(|e| e.job.kind == "preview" && e.job.status == "running")
        {
            entry.cancel.store(true, Ordering::Release);
        }
    }
    pub fn set_playback_assets(
        &self,
        preview_path: Option<PathBuf>,
        source_path: Option<PathBuf>,
    ) -> Result<(), String> {
        let preview_pin = preview_path
            .map(|p| self.ctx.processes.pin_file(p))
            .transpose()
            .map_err(|e| e.to_string())?;
        let source_pin = source_path
            .map(|p| self.ctx.processes.pin_file(p))
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut state = self
            .preview_state
            .lock()
            .map_err(|_| "Preview state unavailable")?;
        state.playback_preview = preview_pin;
        if source_pin.as_ref().is_some_and(|pin| {
            state
                .prepared_source
                .as_ref()
                .is_some_and(|prepared| pin.path() == prepared.path())
        }) {
            state.prepared_source = None;
        }
        state.playback_source = source_pin;
        Ok(())
    }
    pub fn begin_source_request(&self) -> u64 {
        self.source_sequence.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub fn invalidate_source(&self) {
        let mut state = self.preview_state.lock().unwrap_or_else(|e| e.into_inner());
        self.source_sequence.fetch_add(1, Ordering::AcqRel);
        state.prepared_source = None;
    }
    pub fn retain_source(&self, ticket: u64, pin: AssetPin) {
        let mut state = self.preview_state.lock().unwrap_or_else(|e| e.into_inner());
        if ticket == self.source_sequence.load(Ordering::Acquire) {
            state.prepared_source = Some(pin);
        }
    }
    pub fn preview_checked(
        &self,
        p: Project,
        height: u32,
        use_proxies: bool,
        expected_key: Option<&str>,
    ) -> Result<Job, String> {
        self.preview_request(p, height, use_proxies, expected_key, None)
    }
    pub fn preview_for_intent(
        &self,
        p: Project,
        height: u32,
        use_proxies: bool,
        expected_key: Option<&str>,
        intent: &PreviewIntent,
    ) -> Result<Job, String> {
        self.preview_request(p, height, use_proxies, expected_key, Some(intent))
    }
    fn preview_request(
        &self,
        mut p: Project,
        height: u32,
        use_proxies: bool,
        expected_key: Option<&str>,
        intent: Option<&PreviewIntent>,
    ) -> Result<Job, String> {
        p.validate()?;
        p.in_point = None;
        p.out_point = None;
        let key = self.preview_identity(&p, height, use_proxies)?;
        if expected_key
            .map(|expected| expected != key)
            .unwrap_or(false)
        {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: The current program differs from the requested preview."
                    .into(),
            );
        }
        let sequence = {
            let mut state = self
                .preview_state
                .lock()
                .map_err(|_| "Preview state unavailable")?;
            if intent.is_some() {
                let sequence = self.preview_sequence.load(Ordering::Acquire);
                if !self.preview_request_current(&state, sequence, &key, intent) {
                    return Err(
                        "PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this render."
                            .into(),
                    );
                }
                sequence
            } else {
                if state.ui_session.is_some() {
                    return Err("PREVIEW_IDENTITY_CHANGED: This render requires the current preview intent.".into());
                }
                let sequence = self.preview_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                state.latest_key = Some(key.clone());
                sequence
            }
        };
        // Identical queued requests join one render. Different keys retain latest-request semantics.
        let superseded = || {
            !self.preview_request_current(
                &self.preview_state.lock().unwrap_or_else(|e| e.into_inner()),
                sequence,
                &key,
                intent,
            )
        };
        let _gate = self
            .preview_gate
            .lock()
            .map_err(|_| "Preview request interrupted")?;
        if superseded() {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: A newer preview request superseded this request.".into(),
            );
        }
        let running = self
            .list()
            .into_iter()
            .filter(|j| j.kind == "preview" && j.status == "running")
            .collect::<Vec<_>>();
        for job in &running {
            let not_cancelled = self
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&job.id)
                .map(|e| !e.cancel.load(Ordering::Acquire))
                .unwrap_or(false);
            if job.preview_key.as_deref() == Some(&key) && not_cancelled {
                return Ok(job.clone());
            }
        }
        {
            let state = self
                .preview_state
                .lock()
                .map_err(|_| "Preview state unavailable")?;
            if !self.preview_request_current(&state, sequence, &key, intent) {
                return Err("PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this cancellation.".into());
            }
            let entries = self.entries.lock().map_err(|_| "Job state unavailable")?;
            for entry in entries.values().filter(|entry| {
                entry.job.kind == "preview"
                    && entry.job.status == "running"
                    && entry.job.preview_key.as_deref() != Some(&key)
            }) {
                entry.cancel.store(true, Ordering::Release);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self
            .list()
            .iter()
            .any(|j| j.kind == "preview" && j.status == "running")
        {
            if Instant::now() >= deadline {
                return Err("Previous preview is still stopping; refresh shortly.".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if superseded() {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: A newer preview request superseded this request.".into(),
            );
        }
        if let Some(path) = preview::cached(&self.ctx, &p, height, &key) {
            let pin = self
                .ctx
                .processes
                .pin_file(&path)
                .map_err(|e| e.to_string())?;
            if preview::cached(&self.ctx, &p, height, &key).as_ref() == Some(&path) {
                preview::touch(&self.ctx, &key);
                let mut admission = self
                    .preview_state
                    .lock()
                    .map_err(|_| "Preview state unavailable")?;
                if !self.preview_request_current(&admission, sequence, &key, intent) {
                    return Err("PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this cache request.".into());
                }
                admission.completed_pin = Some(pin);
                let existing = self.list().into_iter().find(|job| {
                    job.status == "complete"
                        && job.preview_key.as_deref() == Some(&key)
                        && job.path.as_deref() == Some(path.to_string_lossy().as_ref())
                });
                if let Some(job) = existing {
                    return Ok(job);
                }
                let job = Job {
                    id: id(),
                    kind: "preview".into(),
                    status: "complete".into(),
                    progress: 1.,
                    path: Some(path.to_string_lossy().into_owned()),
                    error: None,
                    preview_key: Some(key),
                };
                let mut entries = self.entries.lock().map_err(|_| "Job state unavailable")?;
                if entries.len() >= 100 {
                    entries.retain(|_, e| e.job.status == "running");
                }
                entries.insert(
                    job.id.clone(),
                    Entry {
                        job: job.clone(),
                        cancel: Arc::new(AtomicBool::new(false)),
                    },
                );
                drop(entries);
                drop(admission);
                (self.callback)(job.clone());
                return Ok(job);
            }
        }
        if !(120..=2160).contains(&height) {
            return Err("Preview height must be between 120 and 2160".into());
        }
        let height = height / 2 * 2;
        let width =
            ((height as f64 * p.width as f64 / p.height as f64 / 2.).round() as u32 * 2).max(16);
        let settings = ExportSettings {
            width,
            height,
            fps: p.fps,
            codec: "h264".into(),
            crf: 22,
            audio_bitrate: 160,
            sample_rate: p.sample_rate,
        };
        let duration = Rational::from_frames(
            p.out_point.unwrap_or(p.length()) - p.in_point.unwrap_or(0),
            p.fps,
        )
        .value();
        let destination = self
            .ctx
            .cache_dir
            .join(format!("preview-{key}-{}.mp4", id()));
        let mut pins = vec![self
            .ctx
            .processes
            .pin_file(&destination)
            .map_err(|e| e.to_string())?];
        for path in preview::source_paths(&p, use_proxies) {
            pins.push(
                self.ctx
                    .processes
                    .pin_file(path)
                    .map_err(|e| e.to_string())?,
            );
        }
        if p.clips.iter().any(|c| c.title.is_some()) {
            pins.push(
                self.ctx
                    .processes
                    .pin_file(&self.ctx.font)
                    .map_err(|e| e.to_string())?,
            );
        }
        let completed_project = p.clone();
        let completed_context = self.ctx.clone();
        let completed_state = self.preview_state.clone();
        let completed_key = key.clone();
        let completion: PreviewCompletion = Box::new(move |path| {
            preview::complete(
                &completed_context,
                &completed_project,
                height,
                use_proxies,
                &completed_key,
                path,
            )?;
            let mut state = completed_state
                .lock()
                .map_err(|_| "Preview state unavailable")?;
            if state.latest_key.as_deref() == Some(&completed_key) {
                state.completed_pin = Some(
                    completed_context
                        .processes
                        .pin_file(path)
                        .map_err(|e| e.to_string())?,
                );
            }
            Ok(())
        });
        let ctx = self.ctx.clone();
        let admission = self
            .preview_state
            .lock()
            .map_err(|_| "Preview state unavailable")?;
        if !self.preview_request_current(&admission, sequence, &key, intent) {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: A newer preview intent superseded this launch.".into(),
            );
        }
        let launched = self.launch_pinned(
            "preview",
            destination,
            duration,
            Some(key),
            pins,
            Some(completion),
            move |id, out| {
                let mut args = render::compile(&ctx, &p, &settings, use_proxies, out, id)?.args;
                // Short GOPs bound seek decoding work without changing rendering semantics.
                let key_interval = ((p.fps.num + p.fps.den * 2 - 1) / (p.fps.den * 2))
                    .max(1)
                    .to_string();
                let before_output = args.len() - 1;
                args.splice(
                    before_output..before_output,
                    [
                        "-g".into(),
                        key_interval.clone(),
                        "-keyint_min".into(),
                        key_interval,
                        "-sc_threshold".into(),
                        "0".into(),
                    ],
                );
                Ok(args)
            },
        );
        drop(admission);
        launched
    }
    pub fn export(
        &self,
        p: Project,
        path: PathBuf,
        settings: ExportSettings,
    ) -> Result<Job, String> {
        p.validate()?;
        render::validate_settings(&settings)?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if (settings.codec == "h264" && ext != "mp4") || (settings.codec == "ffv1" && ext != "mkv")
        {
            return Err("H.264 exports require .mp4; FFV1 exports require .mkv".into());
        }
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        let requested = path.canonicalize().unwrap_or(path.clone());
        if p.media.iter().any(|m| {
            Path::new(&m.path)
                .canonicalize()
                .map(|s| s == requested)
                .unwrap_or(false)
        }) {
            return Err("Export destination must not overwrite source media".into());
        }
        let duration = Rational::from_frames(
            p.out_point.unwrap_or(p.length()) - p.in_point.unwrap_or(0),
            p.fps,
        )
        .value();
        let ctx = self.ctx.clone();
        let mut pins = Vec::new();
        for source in preview::source_paths(&p, false)
            .into_iter()
            .chain(std::iter::once(path.clone()))
        {
            pins.push(
                self.ctx
                    .processes
                    .pin_file(source)
                    .map_err(|e| e.to_string())?,
            );
        }
        if p.clips.iter().any(|c| c.title.is_some()) {
            pins.push(
                self.ctx
                    .processes
                    .pin_file(&self.ctx.font)
                    .map_err(|e| e.to_string())?,
            );
        }
        self.launch_pinned(
            "export",
            path,
            duration,
            None,
            pins,
            None,
            move |id, out| Ok(render::compile(&ctx, &p, &settings, false, out, id)?.args),
        )
    }
    pub fn proxy(&self, m: Media) -> Result<Job, String> {
        if m.kind != "video" {
            return Err("Proxies are supported for video media".into());
        }
        if m.missing || !Path::new(&m.path).is_file() {
            return Err("Relink source media before generating a proxy".into());
        }
        let key = media::cache_key(Path::new(&m.path))?;
        let destination = self
            .ctx
            .cache_dir
            .join(format!("{}-{key}-proxy-v3.mp4", m.id));
        let duration = m.duration.value();
        let ctx = self.ctx.clone();
        let pins = vec![
            self.ctx
                .processes
                .pin_file(&m.path)
                .map_err(|e| e.to_string())?,
            self.ctx
                .processes
                .pin_file(&destination)
                .map_err(|e| e.to_string())?,
        ];
        self.launch_pinned(
            "proxy",
            destination,
            duration,
            None,
            pins,
            None,
            move |id, out| media::normalized_cache_args(&ctx, &m, 1280, 720, 23, out, id),
        )
    }
}
