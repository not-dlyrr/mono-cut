pub mod edit;
pub mod jobs;
pub mod media;
pub mod model;
pub mod preview;
pub mod processes;
pub mod render;
pub mod storage;

use jobs::{JobManager, PreviewIntent};
use media::MediaContext;
use model::{Capabilities, EditCommand, ExportSettings, Job, Project, Rational};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use storage::ProjectStore;
use tauri::{Emitter, Manager, State};

struct EditorState {
    store: Arc<Mutex<ProjectStore>>,
    jobs: Arc<JobManager>,
    ctx: MediaContext,
}

fn store_lock(state: &EditorState) -> Result<std::sync::MutexGuard<'_, ProjectStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "Editor state was interrupted; recover the autosave.".to_string())
}

#[tauri::command]
fn get_project(state: State<EditorState>) -> Result<Project, String> {
    Ok(store_lock(&state)?.get())
}
#[tauri::command]
fn new_project(
    name: String,
    width: u32,
    height: u32,
    fps: Rational,
    state: State<EditorState>,
) -> Result<Project, String> {
    store_lock(&state)?.new_project(name, width, height, fps)
}
#[tauri::command]
async fn edit(command: EditCommand, state: State<'_, EditorState>) -> Result<Project, String> {
    let store = state.store.clone();
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| "Editor lock interrupted".to_string())?
            .apply(command, &ctx)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn undo(state: State<EditorState>) -> Result<Project, String> {
    store_lock(&state)?.undo()
}
#[tauri::command]
fn redo(state: State<EditorState>) -> Result<Project, String> {
    store_lock(&state)?.redo()
}
#[tauri::command]
fn save_project(path: String, state: State<EditorState>) -> Result<Project, String> {
    store_lock(&state)?.save(&PathBuf::from(path))
}
#[tauri::command]
async fn open_project(path: String, state: State<'_, EditorState>) -> Result<Project, String> {
    let store = state.store.clone();
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut locked = store
            .lock()
            .map_err(|_| "Editor lock interrupted".to_string())?;
        locked.open_hydrated(&PathBuf::from(path), &ctx)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn recovery_available(state: State<EditorState>) -> Result<bool, String> {
    Ok(store_lock(&state)?.recovery_available())
}
#[tauri::command]
async fn recover_project(state: State<'_, EditorState>) -> Result<Project, String> {
    let store = state.store.clone();
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| "Editor lock interrupted".to_string())?
            .recover_hydrated(&ctx)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn import_media(
    paths: Vec<String>,
    state: State<'_, EditorState>,
) -> Result<Project, String> {
    let store = state.store.clone();
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .lock()
            .map_err(|_| "Editor lock interrupted".to_string())?
            .import(paths.into_iter().map(PathBuf::from).collect(), &ctx)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn render_preview(
    height: u32,
    use_proxies: bool,
    expected_key: Option<String>,
    intent: Option<PreviewIntent>,
    state: State<'_, EditorState>,
) -> Result<Job, String> {
    let jobs = state.jobs.clone();
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let project = store.lock().map_err(|_| "Project state unavailable")?.get();
        match intent {
            Some(intent) => jobs.preview_for_intent(
                project,
                height,
                use_proxies,
                expected_key.as_deref(),
                &intent,
            ),
            None => jobs.preview_checked(project, height, use_proxies, expected_key.as_deref()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn begin_preview_session(state: State<EditorState>) -> u64 {
    state.jobs.begin_preview_session()
}
#[tauri::command]
async fn set_preview_intent(
    height: u32,
    use_proxies: bool,
    expected_key: String,
    session: u64,
    revision: u64,
    state: State<'_, EditorState>,
) -> Result<PreviewIntent, String> {
    let jobs = state.jobs.clone();
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Keep the store snapshot current through publication, so a queued RPC
        // cannot publish a project state that Undo has already replaced.
        let store = store.lock().map_err(|_| "Project state unavailable")?;
        let key = jobs.preview_identity(&store.get(), height, use_proxies)?;
        if key != expected_key {
            return Err(
                "PREVIEW_IDENTITY_CHANGED: The current program differs from this intent.".into(),
            );
        }
        jobs.set_preview_intent(session, revision, &key)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn preview_identity(
    height: u32,
    use_proxies: bool,
    state: State<'_, EditorState>,
) -> Result<serde_json::Value, String> {
    let jobs = state.jobs.clone();
    let store = state.store.clone();
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let project = store.lock().map_err(|_| "Project state unavailable")?.get();
        let key = jobs.preview_identity(&project, height, use_proxies)?;
        let cached_path = preview::cached(&ctx, &project, height, &key)
            .map(|path| path.to_string_lossy().into_owned());
        Ok(serde_json::json!({"key":key,"cached_path":cached_path}))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn set_playback_assets(
    preview_path: Option<String>,
    source_path: Option<String>,
    state: State<EditorState>,
) -> Result<(), String> {
    if preview_path.is_none() {
        let store = store_lock(&state)?;
        if store.get().clips.is_empty() {
            state.jobs.clear_preview();
        }
    }
    state.jobs.set_playback_assets(
        preview_path.map(PathBuf::from),
        source_path.map(PathBuf::from),
    )
}
#[tauri::command]
fn export_project(
    path: String,
    settings: ExportSettings,
    state: State<EditorState>,
) -> Result<Job, String> {
    state
        .jobs
        .export(store_lock(&state)?.get(), PathBuf::from(path), settings)
}
#[tauri::command]
fn generate_proxy(media_id: String, state: State<EditorState>) -> Result<Job, String> {
    let project = store_lock(&state)?.get();
    let media = project
        .media
        .into_iter()
        .find(|m| m.id == media_id)
        .ok_or("Media does not exist")?;
    state.jobs.proxy(media)
}
#[tauri::command]
fn cancel_job(id: String, state: State<EditorState>) -> Result<(), String> {
    state.jobs.cancel(&id)
}
#[tauri::command]
fn get_jobs(state: State<EditorState>) -> Vec<Job> {
    state.jobs.list()
}
#[tauri::command]
async fn capabilities(state: State<'_, EditorState>) -> Result<Capabilities, String> {
    let ctx = state.ctx.clone();
    tauri::async_runtime::spawn_blocking(move || media::capabilities(&ctx))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
fn invalidate_source(state: State<EditorState>) {
    state.jobs.invalidate_source();
}
#[tauri::command]
async fn prepare_source(media_id: String, state: State<'_, EditorState>) -> Result<String, String> {
    let ctx = state.ctx.clone();
    let jobs = state.jobs.clone();
    let source_ticket = jobs.begin_source_request();
    let media = store_lock(&state)?
        .get()
        .media
        .into_iter()
        .find(|m| m.id == media_id)
        .ok_or("Media does not exist")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (path, pin) = media::prepare_source_pinned(&ctx, &media)?;
        jobs.retain_source(source_ticket, pin);
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn resolve_media(app: &tauri::App, name: &str, variable: &str) -> PathBuf {
    if let Some(value) = std::env::var_os(variable) {
        return PathBuf::from(value);
    }
    let resource = app
        .path()
        .resource_dir()
        .unwrap_or_default()
        .join("resources")
        .join("media")
        .join(name);
    if resource.exists() {
        return resource;
    }
    if let Ok(current) = std::env::current_dir() {
        for directory in [
            current.join("resources"),
            current.join("src-tauri").join("resources"),
        ] {
            let development = directory.join("media").join(name);
            if development.exists() {
                return development;
            }
        }
    }
    PathBuf::from(name)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cache_dir = app.path().app_cache_dir()?;
            std::fs::create_dir_all(&cache_dir)?;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let ctx = MediaContext {
                ffmpeg: resolve_media(
                    app,
                    if cfg!(windows) {
                        "ffmpeg.exe"
                    } else {
                        "ffmpeg"
                    },
                    "MONO_CUT_FFMPEG",
                ),
                ffprobe: resolve_media(
                    app,
                    if cfg!(windows) {
                        "ffprobe.exe"
                    } else {
                        "ffprobe"
                    },
                    "MONO_CUT_FFPROBE",
                ),
                font: resolve_media(app, "Inter.ttf", "MONO_CUT_FONT"),
                cache_dir,
                processes: Default::default(),
            };
            let store = Arc::new(Mutex::new(ProjectStore::new(
                data_dir.join("recovery.json"),
            )));
            let handle = app.handle().clone();
            let callback_store = store.clone();
            let jobs = Arc::new(JobManager::new(
                ctx.clone(),
                Arc::new(move |job: Job| {
                    if job.kind == "proxy" && job.status == "complete" {
                        if let Some(ref path) = job.path {
                            if let Ok(mut store) = callback_store.lock() {
                                // Proxy filenames contain the media id; attach only to the matching media.
                                let project = store.get();
                                let mut attached = false;
                                for media in project.media {
                                    if path.contains(&media.id) {
                                        match store.set_proxy(&media.id, PathBuf::from(path)) {
                                            Ok(_) => attached = true,
                                            Err(error) => {
                                                let _ = handle.emit("project-error", format!("Proxy was generated but could not be attached: {error}"));
                                            }
                                        }
                                    }
                                }
                                if attached {
                                    let _ = handle.emit("project-changed", store.get());
                                }
                            }
                        }
                    }
                    let _ = handle.emit("job-progress", job);
                }),
            ));
            app.manage(EditorState { store, jobs, ctx });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_project,
            new_project,
            edit,
            undo,
            redo,
            save_project,
            open_project,
            recovery_available,
            recover_project,
            import_media,
            render_preview,
            begin_preview_session,
            set_preview_intent,
            preview_identity,
            set_playback_assets,
            export_project,
            generate_proxy,
            cancel_job,
            get_jobs,
            capabilities,
            prepare_source,
            invalidate_source
        ])
        .build(tauri::generate_context!())
        .expect("Mono Cut failed to launch")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app.try_state::<EditorState>() {
                    state.jobs.shutdown();
                }
            }
        });
}
