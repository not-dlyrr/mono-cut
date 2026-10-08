//! Real preview reuse and cancellation. Media is generated and decoded silently.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    storage::ProjectStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier, Mutex},
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
fn fixture(ctx: &MediaContext, path: &Path, color: &str, seconds: u32) {
    media::run_ffmpeg(
        ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c={color}:size=160x90:rate=30:duration={seconds}"),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("aevalsrc=0.1*sin(2*PI*997*t):s=48000:d={seconds}"),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "flac".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
}
fn project(ctx: &MediaContext, source: &Path, frames: i64) -> Project {
    let mut p = Project::new("Preview cache".into(), 160, 90, Rational::new(30, 1));
    p.media.push(media::probe(ctx, source).unwrap());
    let mid = p.media[0].id.clone();
    let tid = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: mid,
            track_id: tid,
            start: 0,
            source_in: None,
            duration: Some(frames),
        },
    )
    .unwrap();
    p
}
fn job_manager(ctx: &MediaContext) -> JobManager {
    JobManager::new(ctx.clone(), Arc::new(|_| {}))
}
fn wait_terminal(manager: &JobManager, job: &Job) -> Job {
    let start = Instant::now();
    loop {
        let current = manager
            .list()
            .into_iter()
            .find(|j| j.id == job.id)
            .expect("Running job must remain listed");
        if current.status != "running" {
            return current;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "Preview did not finish: {current:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn complete(manager: &JobManager, job: &Job) -> Job {
    let job = wait_terminal(manager, job);
    assert_eq!(job.status, "complete", "{job:?}");
    assert!(job.path.as_ref().is_some_and(|p| Path::new(p).is_file()));
    assert!(job.preview_key.as_ref().is_some_and(|key| !key.is_empty()));
    job
}
fn preview(manager: &JobManager, p: &Project, height: u32, proxies: bool) -> Job {
    let job = manager.preview(p.clone(), height, proxies).unwrap();
    complete(manager, &job)
}
fn assert_reuse(
    ctx: &MediaContext,
    manager: &JobManager,
    p: &Project,
    original: &Job,
    height: u32,
    proxies: bool,
    label: &str,
) {
    let count = ctx.processes.total_spawn_count();
    let reused = manager.preview(p.clone(), height, proxies).unwrap();
    assert_eq!(
        reused.status, "complete",
        "{label}: completed result must be returned directly"
    );
    assert_eq!(
        reused.id, original.id,
        "{label}: same manager must reuse its completed job"
    );
    assert_eq!(
        reused.path, original.path,
        "{label}: unchanged render must retain its playback asset"
    );
    assert_eq!(
        reused.preview_key, original.preview_key,
        "{label}: preview identity changed"
    );
    assert_eq!(
        ctx.processes.total_spawn_count(),
        count,
        "{label}: reuse unexpectedly spawned a decoder/prober/encoder"
    );
}
fn no_sidecars(ctx: &MediaContext) {
    let leftovers: Vec<_> = fs::read_dir(&ctx.cache_dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (name.starts_with(".mono-part-")
                || name.ends_with("-filters.txt")
                || name.contains("-title-"))
            .then_some(name)
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "Cancelled preview left temporary files: {leftovers:?}"
    );
}
fn first_pixel(ctx: &MediaContext, path: &Path) -> [u8; 3] {
    let result = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=1:1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout[..3].try_into().unwrap()
}

#[test]
fn completed_previews_reuse_across_metadata_edits_save_reopen_and_history_without_spawning() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("red.mkv");
    let unused = dir.path().join("unused-blue.mkv");
    fixture(&ctx, &source, "red", 1);
    fixture(&ctx, &unused, "blue", 1);
    let original = project(&ctx, &source, 30);
    let unused_media = media::probe(&ctx, &unused).unwrap();
    let path = dir.path().join("metadata.monocut");
    fs::write(&path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    let mut store = ProjectStore::new(dir.path().join("recovery.json"));
    store.open(&path).unwrap();
    let manager = job_manager(&ctx);
    let rendered = preview(&manager, &original, 180, false);
    let identity = manager.preview_identity(&original, 180, false).unwrap();
    assert_eq!(rendered.preview_key.as_deref(), Some(identity.as_str()));
    for command in [
        EditCommand::Rename {
            name: "Renamed project".into(),
        },
        EditCommand::Marker {
            frame: 12,
            name: "Review note".into(),
        },
        EditCommand::SetRange {
            in_point: Some(4),
            out_point: Some(20),
        },
        EditCommand::AddBin {
            name: "Camera".into(),
        },
    ] {
        let changed = store.apply(command, &ctx).unwrap();
        assert_reuse(
            &ctx,
            &manager,
            &changed,
            &rendered,
            180,
            false,
            "metadata editing",
        );
    }
    let current = store.get();
    let bin = current.bins[0].id.clone();
    let mid = current.media[0].id.clone();
    let assigned = store
        .apply(
            EditCommand::AssignBin {
                media_id: mid,
                bin_id: Some(bin),
            },
            &ctx,
        )
        .unwrap();
    assert_reuse(
        &ctx,
        &manager,
        &assigned,
        &rendered,
        180,
        false,
        "bin assignment",
    );
    store.save(&path).unwrap();
    assert_reuse(&ctx, &manager, &store.get(), &rendered, 180, false, "save");
    let reopened = ProjectStore::new(dir.path().join("reopened.json"))
        .open(&path)
        .unwrap();
    assert_reuse(&ctx, &manager, &reopened, &rendered, 180, false, "reopen");
    let mut cosmetic = reopened;
    cosmetic.id = id();
    cosmetic.clips[0].name = "Named clip".into();
    cosmetic.clips[0].linked_id = Some("link-only".into());
    cosmetic.tracks[0].name = "Named track".into();
    cosmetic.tracks[0].locked = true;
    cosmetic.media[0].name = "Named source".into();
    cosmetic.media[0].thumbnail = Some("unused-thumbnail.png".into());
    cosmetic.media[0].waveform = vec![0.2, 0.4];
    cosmetic.media.push(unused_media);
    assert_reuse(
        &ctx,
        &manager,
        &cosmetic,
        &rendered,
        180,
        false,
        "names, lock, unused import, waveform and thumbnail",
    );
    // Even an unavailable unused item cannot invalidate a render that never reads it.
    cosmetic.media.last_mut().unwrap().path = dir
        .path()
        .join("missing-unused.mkv")
        .to_string_lossy()
        .into_owned();
    cosmetic.media.last_mut().unwrap().missing = true;
    assert_reuse(
        &ctx,
        &manager,
        &cosmetic,
        &rendered,
        180,
        false,
        "missing unused media",
    );
    let mut stale = cosmetic.clone();
    stale.clips[0].brightness = 0.07;
    let count = ctx.processes.total_spawn_count();
    let rejection = manager
        .preview_checked(stale, 180, false, Some(&identity))
        .unwrap_err();
    assert!(
        rejection.starts_with("PREVIEW_IDENTITY_CHANGED:"),
        "Stale engine identity must be distinguishable by the interface: {rejection}"
    );
    assert_eq!(
        ctx.processes.total_spawn_count(),
        count,
        "A stale requested key must reject before launching work"
    );
    assert_reuse(
        &ctx,
        &manager,
        &cosmetic,
        &rendered,
        180,
        false,
        "Rejected stale request preserves existing result",
    );

    let original_state = store.get();
    let cid = original_state.clips[0].id.clone();
    let effect = store
        .apply(
            EditCommand::UpdateClip {
                id: cid,
                patch: json!({"brightness":0.1}),
            },
            &ctx,
        )
        .unwrap();
    let effect_render = preview(&manager, &effect, 180, false);
    assert_ne!(effect_render.preview_key, rendered.preview_key);
    assert_reuse(
        &ctx,
        &manager,
        &store.undo().unwrap(),
        &rendered,
        180,
        false,
        "undo to cached render",
    );
    assert_reuse(
        &ctx,
        &manager,
        &store.redo().unwrap(),
        &effect_render,
        180,
        false,
        "redo to cached render",
    );
    let spawn_count = ctx.processes.total_spawn_count();
    let start = Instant::now();
    for _ in 0..50 {
        assert_eq!(
            manager.preview_identity(&cosmetic, 180, false).unwrap(),
            identity
        );
    }
    assert_eq!(ctx.processes.total_spawn_count(), spawn_count);
    assert!(
        manager.list().len() <= 3,
        "Metadata events must not accumulate jobs"
    );
    eprintln!("VERIFIED metadata/save/reopen/undo/redo preview reuse without any media child spawn; 50 identity checks in {:.3} ms, {} retained jobs.",start.elapsed().as_secs_f64()*1000.,manager.list().len());
    manager.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn render_effects_track_controls_resolution_and_source_stat_changes_invalidate_identity() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("changing-source.mkv");
    fixture(&ctx, &source, "red", 2);
    let original = project(&ctx, &source, 30);
    let manager = job_manager(&ctx);
    let key = manager.preview_identity(&original, 180, false).unwrap();
    let count = ctx.processes.total_spawn_count();
    let mut variants = vec![];
    let mut p = original.clone();
    p.clips[0].opacity = 0.5;
    variants.push(("opacity", p));
    let mut p = original.clone();
    p.clips[0].volume = 0.5;
    variants.push(("volume", p));
    let mut p = original.clone();
    p.clips[0].transform.crop_left = 0.1;
    variants.push(("crop", p));
    let mut p = original.clone();
    p.clips[0].transform.x = 6.;
    variants.push(("transform", p));
    let mut p = original.clone();
    p.clips[0].brightness = 0.1;
    variants.push(("color", p));
    let mut p = original.clone();
    p.clips[0].fade_in = 15;
    variants.push(("fade", p));
    let mut p = original.clone();
    p.clips[0].keyframes.push(Keyframe {
        property: "opacity".into(),
        frame: 15,
        value: 0.6,
    });
    variants.push(("automation", p));
    let mut p = original.clone();
    p.clips[0].source_in = Rational::new(1, 2);
    variants.push(("source in", p));
    let mut p = original.clone();
    p.clips[0].start = 5;
    variants.push(("timeline position", p));
    let mut p = original.clone();
    p.clips[0].speed = Rational::new(2, 1);
    variants.push(("speed", p));
    let mut p = original.clone();
    p.tracks[0].muted = true;
    variants.push(("mute", p));
    let mut p = original.clone();
    p.tracks[0].hidden = true;
    variants.push(("hide", p));
    let mut p = original.clone();
    p.sample_rate = 44100;
    variants.push(("audio rate", p));
    let mut p = original.clone();
    p.fps = Rational::new(24, 1);
    variants.push(("sequence frame rate", p));
    let mut p = original.clone();
    p.width = 192;
    variants.push(("sequence aspect", p));
    for (label, p) in variants {
        assert_ne!(
            manager.preview_identity(&p, 180, false).unwrap(),
            key,
            "Render edit omitted from identity: {label}"
        );
    }
    assert_ne!(
        manager.preview_identity(&original, 120, false).unwrap(),
        key,
        "Preview resolution is a render setting"
    );
    assert_eq!(
        ctx.processes.total_spawn_count(),
        count,
        "Identity checks must use source metadata, not child probes or full decoding"
    );
    let before = preview(&manager, &original, 180, false);
    let before_pixel = first_pixel(&ctx, Path::new(before.path.as_ref().unwrap()));
    std::thread::sleep(Duration::from_millis(3));
    fixture(&ctx, &source, "blue", 2);
    let changed_key = manager.preview_identity(&original, 180, false).unwrap();
    assert_ne!(
        changed_key, key,
        "Changing source bytes/stat without a project edit must invalidate preview"
    );
    let after = preview(&manager, &original, 180, false);
    let after_pixel = first_pixel(&ctx, Path::new(after.path.as_ref().unwrap()));
    assert!(
        before_pixel[0] > 200
            && before_pixel[2] < 10
            && after_pixel[2] > 200
            && after_pixel[0] < 10,
        "New source must be decoded, not stale red preview: {before_pixel:?}->{after_pixel:?}"
    );
    fs::remove_file(&source).unwrap();
    let count = ctx.processes.total_spawn_count();
    if let Ok(missing_key) = manager.preview_identity(&original, 180, false) {
        assert_ne!(
            missing_key, changed_key,
            "A missing source cannot retain its previously rendered identity"
        );
    }
    if let Ok(job) = manager.preview(original.clone(), 180, false) {
        let missing = wait_terminal(&manager, &job);
        assert_eq!(
            missing.status, "failed",
            "A missing source must not silently replay a previously cached file: {missing:?}"
        );
        assert!(missing.path.is_none());
        assert!(missing
            .error
            .as_ref()
            .is_some_and(|error| !error.is_empty()));
    }
    assert_eq!(ctx.processes.total_spawn_count(), count);
    eprintln!("VERIFIED render settings/effects/track controls invalidate identity, source stat replacement changes actual red preview to blue, and missing active media rejects cached playback without spawning.");
    manager.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn original_and_proxy_keys_use_selected_file_fingerprints_and_resolution() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("proxy-source.mkv");
    fixture(&ctx, &source, "red", 1);
    let original = project(&ctx, &source, 30);
    let manager = job_manager(&ctx);
    let original_preview = preview(&manager, &original, 180, false);
    let proxy_job = manager.proxy(original.media[0].clone()).unwrap();
    let proxy = wait_terminal(&manager, &proxy_job);
    assert_eq!(proxy.status, "complete", "{proxy:?}");
    let mut proxied = original.clone();
    proxied.media[0].proxy = proxy.path.clone();
    proxied.media[0].proxy_timing_version = Some(NORMALIZED_TIMING_VERSION);
    assert!(media::proxy_is_current(&proxied.media[0]));
    assert_reuse(
        &ctx,
        &manager,
        &proxied,
        &original_preview,
        180,
        false,
        "Assigning an unused proxy",
    );
    let proxy_preview = preview(&manager, &proxied, 180, true);
    assert_ne!(proxy_preview.preview_key, original_preview.preview_key);
    assert_reuse(
        &ctx,
        &manager,
        &proxied,
        &proxy_preview,
        180,
        true,
        "Repeated proxy preview",
    );
    let low = preview(&manager, &proxied, 120, true);
    assert_ne!(low.preview_key, proxy_preview.preview_key);
    assert_reuse(
        &ctx,
        &manager,
        &proxied,
        &proxy_preview,
        180,
        true,
        "Restore cached preview resolution",
    );
    let proxy_path = Path::new(proxied.media[0].proxy.as_ref().unwrap());
    // MP4 readers ignore trailing bytes; changing length makes stat invalidation deterministic.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(proxy_path)
        .unwrap()
        .write_all(b"preview-stat-change")
        .unwrap();
    assert_ne!(
        manager.preview_identity(&proxied, 180, true).unwrap(),
        proxy_preview.preview_key.unwrap()
    );
    assert_eq!(
        manager.preview_identity(&proxied, 180, false).unwrap(),
        original_preview.preview_key.unwrap()
    );
    eprintln!("VERIFIED original/proxy switching and preview resolution reuse; only the selected proxy fingerprint changes its render key.");
    manager.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn completed_cache_survives_manager_restart_but_missing_tampered_or_evicted_entries_rebuild() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("manifest-source.mkv");
    fixture(&ctx, &source, "red", 1);
    let p = project(&ctx, &source, 30);
    let manager = job_manager(&ctx);
    let initial = preview(&manager, &p, 180, false);
    let mut output = PathBuf::from(initial.path.as_ref().unwrap());
    let manifest = ctx.cache_dir.join(format!(
        "preview-{}.json",
        initial.preview_key.as_ref().unwrap()
    ));
    assert!(
        manifest.is_file(),
        "Validated preview output must have a persistent manifest"
    );
    let count = ctx.processes.total_spawn_count();
    let restarted = job_manager(&ctx);
    let reused = restarted.preview(p.clone(), 180, false).unwrap();
    assert_eq!(reused.status, "complete");
    assert_eq!(reused.path, initial.path);
    assert_eq!(reused.preview_key, initial.preview_key);
    assert_eq!(
        ctx.processes.total_spawn_count(),
        count,
        "Validated persisted cache must not re-probe/re-encode on restart"
    );
    let original_manifest: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert!(original_manifest.is_object());
    for case in [
        "manifest-absent",
        "manifest-invalid",
        "manifest-version",
        "manifest-metadata",
        "output-stat-changed",
        "output-evicted",
    ] {
        match case {
            "manifest-absent" => {
                fs::remove_file(&manifest).unwrap();
            }
            "manifest-invalid" => {
                fs::write(&manifest, b"{malformed-json").unwrap();
            }
            "manifest-version" | "manifest-metadata" => {
                let mut changed: Value =
                    serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
                if case == "manifest-version" {
                    changed["recipe"] = json!("program-preview-v1");
                } else {
                    changed["width"] = json!(2);
                    changed["sample_rate"] = json!(8000);
                }
                fs::write(&manifest, serde_json::to_vec(&changed).unwrap()).unwrap();
            }
            "output-stat-changed" => {
                use std::io::Write;
                fs::OpenOptions::new()
                    .append(true)
                    .open(&output)
                    .unwrap()
                    .write_all(b"output-tampered")
                    .unwrap();
            }
            "output-evicted" => {
                fs::remove_file(&output).unwrap();
            }
            _ => unreachable!(),
        }
        let before_count = ctx.processes.total_spawn_count();
        let rebuilt = preview(&restarted, &p, 180, false);
        assert!(
            ctx.processes.total_spawn_count() > before_count,
            "{case}: invalid cache must produce a real replacement"
        );
        assert_eq!(
            rebuilt.preview_key, initial.preview_key,
            "{case}: identity should remain independent of output damage"
        );
        output = PathBuf::from(rebuilt.path.as_ref().unwrap());
        assert!(
            output
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!(
                    "preview-{}",
                    initial.preview_key.as_ref().unwrap()
                )),
            "{case}: repaired render must belong to the same cached identity"
        );
        assert!(manifest.is_file());
        let pixel = first_pixel(&ctx, &output);
        assert!(pixel[0] > 200 && pixel[2] < 10);
        assert_reuse(&ctx, &restarted, &p, &rebuilt, 180, false, case);
    }
    eprintln!("VERIFIED cache manifests survive manager restart without media processes; missing, malformed, incompatible-version/metadata, changed-stat, and evicted assets trigger real validated replacement.");
    manager.shutdown();
    restarted.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn simultaneous_identical_requests_coalesce_running_job_and_do_not_cancel_it() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("coalesce-source.mkv");
    fixture(&ctx, &source, "red", 2);
    let p = project(&ctx, &source, 60);
    let manager = job_manager(&ctx);
    let gate = Arc::new(Barrier::new(9));
    let count = ctx.processes.total_spawn_count();
    let mut workers = vec![];
    for _ in 0..8 {
        let m = manager.clone();
        let p = p.clone();
        let gate = gate.clone();
        workers.push(std::thread::spawn(move || {
            gate.wait();
            m.preview(p, 360, false).unwrap()
        }));
    }
    gate.wait();
    let jobs: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    for j in &jobs {
        assert_eq!(
            j.id, jobs[0].id,
            "Same running render must coalesce atomically"
        );
        assert_eq!(j.preview_key, jobs[0].preview_key);
    }
    let rendered = complete(&manager, &jobs[0]);
    assert_eq!(
        manager
            .list()
            .iter()
            .filter(|j| j.kind == "preview")
            .count(),
        1
    );
    let spawned = ctx.processes.total_spawn_count() - count;
    assert!(spawned<=3,"A single preview should launch only renderer and validation probes, not eight workers: {spawned}");
    assert_reuse(
        &ctx,
        &manager,
        &p,
        &rendered,
        360,
        false,
        "Coalesced completed preview",
    );
    eprintln!("VERIFIED eight simultaneous identical preview requests coalesce into one uncancelled job; {spawned} actual media children including validation.");
    manager.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn validated_preview_cache_is_bounded_while_active_playback_asset_survives_pruning() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("bounded-source.mkv");
    fixture(&ctx, &source, "red", 1);
    let p = project(&ctx, &source, 30);
    let manager = job_manager(&ctx);
    let playing = preview(&manager, &p, 120, false);
    let playing_path = PathBuf::from(playing.path.as_ref().unwrap());
    manager
        .set_playback_assets(Some(playing_path.clone()), None)
        .unwrap();
    assert!(ctx.processes.is_protected(&playing_path));
    for index in 1..=35 {
        let mut changed = p.clone();
        changed.clips[0].brightness = index as f64 / 100.;
        preview(&manager, &changed, 120, false);
    }
    let records: Vec<_> = fs::read_dir(&ctx.cache_dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("preview-") && name.ends_with(".json")
        })
        .collect();
    assert!(
        records.len() <= mono_cut_lib::preview::MAX_ENTRIES,
        "Preview cache retained {} records beyond its entry budget",
        records.len()
    );
    let output_bytes: u64 = records
        .iter()
        .map(|entry| {
            let manifest: Value = serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            fs::metadata(ctx.cache_dir.join(manifest["file"].as_str().unwrap()))
                .unwrap()
                .len()
        })
        .sum();
    assert!(output_bytes <= mono_cut_lib::preview::MAX_BYTES);
    assert!(
        playing_path.is_file(),
        "Pruning must preserve an actively consumed old preview"
    );
    assert_reuse(
        &ctx,
        &manager,
        &p,
        &playing,
        120,
        false,
        "Pinned older playback asset",
    );
    manager.set_playback_assets(None, None).unwrap();
    // The previous reuse touched its LRU record. Age only that record to make
    // the now-released asset an explicit oldest eviction candidate.
    let old_manifest = ctx.cache_dir.join(format!(
        "preview-{}.json",
        playing.preview_key.as_ref().unwrap()
    ));
    fs::File::options()
        .write(true)
        .open(old_manifest)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86400)),
        )
        .unwrap();
    // Move the manager's most-recent result away from the previous playback file.
    let mut changed = p.clone();
    changed.clips[0].brightness = 0.36;
    preview(&manager, &changed, 120, false);
    mono_cut_lib::preview::prune(&ctx);
    assert!(
        !playing_path.exists(),
        "Released oldest preview should be evicted once newer renders exceed the budget"
    );
    let rebuilt = preview(&manager, &p, 120, false);
    assert_ne!(rebuilt.path, playing.path);
    eprintln!("VERIFIED bounded preview cache: {} records/{output_bytes} bytes after 35 changed renders; active playback pin retained old asset, releasing it allowed eviction and actual rebuild.",records.len());
    manager.shutdown();
    no_sidecars(&ctx);
}

#[test]
fn latest_preview_bursts_project_switches_and_shutdown_drain_workers_and_sidecars() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let red = dir.path().join("burst-red.mkv");
    let blue = dir.path().join("switch-blue.mkv");
    fixture(&ctx, &red, "red", 2);
    fixture(&ctx, &blue, "blue", 1);
    let p = project(&ctx, &red, 60);
    let other = project(&ctx, &blue, 30);
    let events: Arc<Mutex<Vec<(Instant, Job, usize)>>> = Default::default();
    let events_copy = events.clone();
    let registry = ctx.processes.clone();
    let manager = JobManager::new(
        ctx.clone(),
        Arc::new(move |j| {
            events_copy
                .lock()
                .unwrap()
                .push((Instant::now(), j, registry.active_count()))
        }),
    );
    let first = manager.preview(p.clone(), 1080, false).unwrap();
    let started = Instant::now();
    while ctx.processes.active_count() == 0
        && manager
            .list()
            .iter()
            .any(|j| j.id == first.id && j.status == "running")
    {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
    let count = ctx.processes.total_spawn_count();
    let burst_start = Instant::now();
    let mut submitted = vec![first];
    for index in 0..24 {
        let mut edited = p.clone();
        edited.clips[0].brightness = index as f64 / 100.;
        submitted.push(manager.preview(edited, 180, false).unwrap());
        assert!(
            ctx.processes.active_count() <= 2,
            "Obsolete preview children piled up"
        );
    }
    let latest_at = Instant::now();
    let latest = manager.preview(other.clone(), 180, false).unwrap();
    let completed = complete(&manager, &latest);
    assert_eq!(
        completed.preview_key,
        Some(manager.preview_identity(&other, 180, false).unwrap())
    );
    let pixel = first_pixel(&ctx, Path::new(completed.path.as_ref().unwrap()));
    assert!(
        pixel[2] > 200 && pixel[0] < 10,
        "Latest project must produce blue, not an obsolete red request: {pixel:?}"
    );
    for old in submitted {
        let terminal = wait_terminal(&manager, &old);
        assert_ne!(terminal.status, "running");
        assert_ne!(
            terminal.status, "failed",
            "Obsolete work must cancel cleanly: {terminal:?}"
        );
    }
    assert_eq!(ctx.processes.active_count(), 0);
    let observed = events.lock().unwrap();
    assert!(
        observed.iter().all(|(_, _, active)| *active <= 2),
        "Callback observed unbounded child fan-out"
    );
    assert!(
        !observed.iter().any(|(at, j, _)| *at >= latest_at
            && j.kind == "preview"
            && j.status == "complete"
            && j.preview_key != completed.preview_key),
        "Obsolete render completed after the latest request"
    );
    drop(observed);
    assert!(manager.list().len() <= 100);
    no_sidecars(&ctx);
    eprintln!("VERIFIED 24 changed preview requests plus project switch: latest blue result retained, bounded media children, cancelled worker/sidecar cleanup; {:.3}s burst and {} media launches.",burst_start.elapsed().as_secs_f64(),ctx.processes.total_spawn_count()-count);
    let mut pending = p;
    pending.clips[0].brightness = -0.1;
    let pending = manager.preview(pending, 1080, false).unwrap();
    manager.shutdown();
    assert_eq!(ctx.processes.active_count(), 0);
    assert_ne!(wait_terminal(&manager, &pending).status, "running");
    let after_shutdown = ctx.processes.total_spawn_count();
    assert!(manager.preview(other, 180, false).is_err());
    assert_eq!(ctx.processes.total_spawn_count(), after_shutdown);
    no_sidecars(&ctx);
}

#[test]
fn source_handoff_tickets_keep_latest_pin_through_stale_ui_acknowledgements() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let latest = ctx.cache_dir.join("source-latest.mp4");
    let stale = ctx.cache_dir.join("source-stale.mp4");
    fs::write(&latest, [1u8; 1024]).unwrap();
    fs::write(&stale, [2u8; 512]).unwrap();
    let manager = job_manager(&ctx);
    let old_ticket = manager.begin_source_request();
    let new_ticket = manager.begin_source_request();
    assert!(new_ticket > old_ticket);
    manager.retain_source(new_ticket, ctx.processes.pin_file(&latest).unwrap());
    manager.retain_source(old_ticket, ctx.processes.pin_file(&stale).unwrap());
    assert!(ctx.processes.is_protected(&latest));
    assert!(
        !ctx.processes.is_protected(&stale),
        "Stale source completion must drop its own pin"
    );
    assert_eq!(ctx.processes.protected_assets().pin_count, 1);
    manager.set_playback_assets(None, None).unwrap();
    assert!(
        ctx.processes.is_protected(&latest),
        "An older null playback update cannot drop the newly prepared source handoff"
    );
    manager
        .set_playback_assets(None, Some(stale.clone()))
        .unwrap();
    assert!(
        ctx.processes.is_protected(&latest),
        "Acknowledging a different older source must preserve the latest handoff"
    );
    assert_eq!(ctx.processes.protected_assets().pin_count, 2);
    manager.set_playback_assets(None, None).unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count, 1);
    manager.clear_preview();
    assert!(
        ctx.processes.is_protected(&latest),
        "An empty program must not unpin its independent source monitor handoff"
    );
    manager
        .set_playback_assets(None, Some(latest.clone()))
        .unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count,1,"Matching acknowledgement transfers the handoff into playback without a leaked duplicate pin");
    assert!(ctx.processes.is_protected(&latest));
    manager.set_playback_assets(None, None).unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
    let interrupted_ticket = manager.begin_source_request();
    manager.retain_source(interrupted_ticket, ctx.processes.pin_file(&latest).unwrap());
    assert!(ctx.processes.is_protected(&latest));
    manager.invalidate_source();
    assert!(
        !ctx.processes.is_protected(&latest),
        "Resetting or selecting a still image explicitly releases the obsolete prepared handoff"
    );
    manager
        .set_playback_assets(None, Some(stale.clone()))
        .unwrap();
    manager.retain_source(interrupted_ticket, ctx.processes.pin_file(&latest).unwrap());
    assert!(!ctx.processes.is_protected(&latest),"A video preparation finishing after image selection/reset cannot reacquire the invalidated handoff");
    assert!(
        ctx.processes.is_protected(&stale),
        "The currently consumed image/source remains pinned"
    );
    assert_eq!(ctx.processes.protected_assets().pin_count, 1);
    manager.invalidate_source();
    assert!(
        ctx.processes.is_protected(&stale),
        "Intent invalidation releases preparation retention, not acknowledged playback"
    );
    manager.set_playback_assets(None, None).unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
    assert_eq!(ctx.processes.total_spawn_count(), 0);
    eprintln!("VERIFIED latest source ticket wins; stale completion and older null/different-source acknowledgements cannot drop prepared media; matching playback acknowledgement transfers one pin; explicit image/reset invalidation releases obsolete retention and prevents late reacquisition without unpinning active playback.");
    manager.shutdown();
}

#[test]
fn clearing_an_empty_program_releases_completed_retention_after_playback_acknowledgement() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let manager = job_manager(&ctx);
    let mut p = Project::new("Retained program pin".into(), 160, 90, Rational::new(30, 1));
    let track = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddTitle {
            track_id: track,
            start: 0,
            duration: 30,
            text: "Pin fixture".into(),
        },
    )
    .unwrap();
    let key = manager.preview_identity(&p, 120, false).unwrap();
    let settings = mono_cut_lib::preview::settings(&p, 120).unwrap();
    // Seed only the already-validated record boundary. Real encode/validation is
    // covered above; this ownership regression intentionally launches no media.
    let output = ctx.cache_dir.join(format!("preview-{key}-pin-fixture.mp4"));
    fs::write(&output, [3u8; 1024]).unwrap();
    fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86400))
        .unwrap();
    let manifest = mono_cut_lib::preview::Manifest {
        recipe: mono_cut_lib::preview::RECIPE.into(),
        key: key.clone(),
        file: output.file_name().unwrap().to_string_lossy().into_owned(),
        stamp: mono_cut_lib::preview::FileStamp::read(&output).unwrap(),
        width: settings.width,
        height: settings.height,
        fps: p.fps,
        sample_rate: p.sample_rate,
        frames: 30,
        duration: Rational::one(),
    };
    fs::write(
        ctx.cache_dir.join(format!("preview-{key}.json")),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let retained = manager.preview(p, 120, false).unwrap();
    assert_eq!(retained.status, "complete");
    assert_eq!(ctx.processes.protected_assets().pin_count, 1);
    manager
        .set_playback_assets(Some(output.clone()), None)
        .unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count, 2);
    let record = ctx.cache_dir.join(format!("preview-{key}.json"));
    fs::File::options()
        .write(true)
        .open(&record)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86400))
        .unwrap();
    media::prune_cache(&ctx, 0).unwrap();
    assert!(
        output.is_file() && record.is_file(),
        "Global cache pruning must preserve the manifest paired with an aged active preview output"
    );
    manager.clear_preview();
    assert_eq!(
        ctx.processes.protected_assets().pin_count,
        1,
        "Clear releases completed cache retention but keeps the still-consumed playback asset"
    );
    assert!(!ctx.processes.remove_unprotected_file(&output).unwrap());
    media::prune_cache(&ctx, 0).unwrap();
    assert!(
        output.is_file() && record.is_file(),
        "An acknowledged active preview pair remains under the preview subcache budget"
    );
    manager.set_playback_assets(None, None).unwrap();
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
    assert!(ctx.processes.remove_unprotected_file(&output).unwrap());
    mono_cut_lib::preview::prune(&ctx);
    assert!(!ctx.cache_dir.join(format!("preview-{key}.json")).exists());
    assert_eq!(ctx.processes.total_spawn_count(), 0);
    eprintln!("VERIFIED empty-program clear releases completed retention; active playback protects until its null acknowledgement, then cached file and record can be removed without media launches.");
    manager.shutdown();
}

#[test]
fn orphan_preview_outputs_are_removed_immediately_but_active_orphans_remain_pinned() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let orphan = ctx.cache_dir.join("preview-orphan-replaced.mp4");
    let playing = ctx.cache_dir.join("preview-orphan-playing.mp4");
    let malformed = ctx.cache_dir.join("preview-invalid-record.json");
    fs::write(&orphan, [4u8; 4096]).unwrap();
    fs::write(&playing, [5u8; 8192]).unwrap();
    fs::write(&malformed, b"{invalid-json").unwrap();
    let pin = ctx.processes.pin_file(&playing).unwrap();
    mono_cut_lib::preview::prune(&ctx);
    assert!(
        !orphan.exists(),
        "Replacement outputs absent from manifests must not accumulate outside the preview budget"
    );
    assert!(!malformed.exists());
    assert!(
        playing.exists(),
        "Active unindexed playback remains protected"
    );
    assert_eq!(ctx.processes.protected_assets().bytes, 8192);
    drop(pin);
    mono_cut_lib::preview::prune(&ctx);
    assert!(!playing.exists());
    assert_eq!(ctx.processes.total_spawn_count(), 0);
    eprintln!("VERIFIED unindexed replacement MP4s and malformed records are pruned immediately, while pinned playback survives until release; no media processes launched.");
}

fn wait_for_active_preview(ctx: &MediaContext, manager: &JobManager, job: &Job) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = manager.list().into_iter().find(|j| j.id == job.id).unwrap();
        assert_eq!(
            current.status, "running",
            "Preview finished before its running cancellation boundary: {current:?}"
        );
        if ctx.processes.active_count() > 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Preview never spawned its owned media process"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn promptly_cancelled(ctx: &MediaContext, manager: &JobManager, job: &Job) -> Duration {
    let started = Instant::now();
    loop {
        let current = manager.list().into_iter().find(|j| j.id == job.id).unwrap();
        if current.status == "cancelled" && ctx.processes.active_count() == 0 {
            assert!(
                current.path.is_none(),
                "An abandoned render must not publish a playable result"
            );
            return started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "Superseded preview did not promptly release its process: {current:?}; active={}",
            ctx.processes.active_count()
        );
        assert_ne!(
            current.status, "failed",
            "Cancellation must not surface a render failure: {current:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cached_current_intent_cancels_running_work_and_rejects_its_delayed_request_without_encoding() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("intent-red.mkv");
    fixture(&ctx, &source, "red", 3);
    let a = project(&ctx, &source, 30);
    let mut b = a.clone();
    b.clips[0].duration = 90;
    b.clips[0].opacity = 0.76;
    let manager = job_manager(&ctx);
    let session = manager.begin_preview_session();
    let key_a = manager.preview_identity(&a, 180, false).unwrap();
    let key_b = manager.preview_identity(&b, 1080, false).unwrap();
    let intent_a = manager.set_preview_intent(session, 1, &key_a).unwrap();
    let ready_a = complete(
        &manager,
        &manager
            .preview_for_intent(a.clone(), 180, false, Some(&key_a), &intent_a)
            .unwrap(),
    );
    let ready_path = PathBuf::from(ready_a.path.as_ref().unwrap());
    let ready_bytes = fs::read(&ready_path).unwrap();
    manager
        .set_playback_assets(Some(ready_path.clone()), None)
        .unwrap();
    let intent_b = manager.set_preview_intent(session, 2, &key_b).unwrap();
    let running_b = manager
        .preview_for_intent(b.clone(), 1080, false, Some(&key_b), &intent_b)
        .unwrap();
    wait_for_active_preview(&ctx, &manager, &running_b);
    let launches = ctx.processes.total_spawn_count();

    // This is the UI's ready-cache shortcut: publish A's intent and retain A,
    // without making a render request for A or cancelling B by its job ID.
    let current_a = manager.set_preview_intent(session, 3, &key_a).unwrap();
    manager
        .set_playback_assets(Some(ready_path.clone()), None)
        .unwrap();
    let cancellation = promptly_cancelled(&ctx, &manager, &running_b);
    assert_eq!(
        ctx.processes.total_spawn_count(),
        launches,
        "Returning to cached A must not start a decoder/prober/encoder"
    );
    assert_eq!(
        mono_cut_lib::preview::cached(&ctx, &a, 180, &key_a),
        Some(ready_path.clone())
    );
    assert_eq!(fs::read(&ready_path).unwrap(), ready_bytes);

    // Model a B bridge request that was queued before Undo, but reaches native
    // admission only after A became authoritative. It cannot revive B.
    let stale = manager
        .preview_for_intent(b, 1080, false, Some(&key_b), &intent_b)
        .unwrap_err();
    assert!(
        stale.starts_with("PREVIEW_"),
        "Stale intent needs a distinguishable bridge error: {stale}"
    );
    assert_eq!(ctx.processes.total_spawn_count(), launches);
    assert_eq!(ctx.processes.active_count(), 0);
    let reused = manager
        .preview_for_intent(a, 180, false, Some(&key_a), &current_a)
        .unwrap();
    assert_eq!(
        (reused.id, reused.path, reused.status),
        (ready_a.id, ready_a.path, "complete".into())
    );
    assert_eq!(ctx.processes.total_spawn_count(), launches);
    no_sidecars(&ctx);
    eprintln!("VERIFIED ready A intent cancels running B in {cancellation:?}; ready asset unchanged and retained, delayed B admission rejected, cached A reuse launches zero media children.");
    manager.set_playback_assets(None, None).unwrap();
    manager.shutdown();
}

#[test]
fn monotonic_intents_reject_older_different_work_but_keep_same_key_running_sharing() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("shared-intent-blue.mkv");
    fixture(&ctx, &source, "blue", 3);
    let mut p = project(&ctx, &source, 90);
    p.clips[0].opacity = 0.83;
    let mut other = p.clone();
    other.clips[0].brightness = 0.13;
    let manager = job_manager(&ctx);
    let session = manager.begin_preview_session();
    let key = manager.preview_identity(&p, 720, false).unwrap();
    let other_key = manager.preview_identity(&other, 720, false).unwrap();
    let old_same_key = manager.set_preview_intent(session, 10, &key).unwrap();
    let running = manager
        .preview_for_intent(p.clone(), 720, false, Some(&key), &old_same_key)
        .unwrap();
    wait_for_active_preview(&ctx, &manager, &running);
    let launches = ctx.processes.total_spawn_count();
    let latest = manager.set_preview_intent(session, 11, &key).unwrap();
    let older_identical = manager.set_preview_intent(session, 9, &key).unwrap();
    assert_eq!(
        (
            older_identical.session,
            older_identical.revision,
            &older_identical.key
        ),
        (latest.session, latest.revision, &latest.key)
    );
    let stale = manager
        .set_preview_intent(session, 10, &other_key)
        .unwrap_err();
    assert!(stale.starts_with("PREVIEW_"), "{stale}");
    let stale_request = manager
        .preview_for_intent(other, 720, false, Some(&other_key), &old_same_key)
        .unwrap_err();
    assert!(stale_request.starts_with("PREVIEW_"), "{stale_request}");
    assert_eq!(
        ctx.processes.total_spawn_count(),
        launches,
        "Rejected old intent must not launch replacement work"
    );

    let mut metadata_only = p.clone();
    metadata_only.name = "Saved and renamed during render".into();
    metadata_only.clips[0].name = "New clip label".into();
    metadata_only.tracks[0].locked = true;
    assert_eq!(
        manager
            .preview_identity(&metadata_only, 720, false)
            .unwrap(),
        key
    );
    for token in [&old_same_key, &latest, &older_identical] {
        let joined = manager
            .preview_for_intent(metadata_only.clone(), 720, false, Some(&key), token)
            .unwrap();
        assert_eq!(
            joined.id, running.id,
            "Older same-key tokens must share current running work"
        );
        assert_ne!(joined.status, "cancelled");
        assert_eq!(ctx.processes.total_spawn_count(), launches);
    }
    let ready = complete(&manager, &running);
    assert_eq!(ready.preview_key.as_deref(), Some(key.as_str()));
    assert_eq!(
        ctx.processes.total_spawn_count(),
        launches + 1,
        "Only the running encode and its final metadata validation may spawn"
    );
    no_sidecars(&ctx);
    eprintln!("VERIFIED monotonic revisions reject older differing intents without replacement work; older/current same-key tokens and metadata-only contexts share one uncancelled validated worker.");
    manager.shutdown();
}

#[test]
fn a_new_preview_session_cancels_prior_work_and_rejects_prior_context_even_with_matching_key() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("session-red.mkv");
    fixture(&ctx, &source, "red", 3);
    let a = project(&ctx, &source, 30);
    let mut b = a.clone();
    b.clips[0].duration = 90;
    b.clips[0].opacity = 0.79;
    let manager = job_manager(&ctx);
    let old_session = manager.begin_preview_session();
    let key_a = manager.preview_identity(&a, 180, false).unwrap();
    let key_b = manager.preview_identity(&b, 1080, false).unwrap();
    let old_a = manager.set_preview_intent(old_session, 1, &key_a).unwrap();
    let ready_a = complete(
        &manager,
        &manager
            .preview_for_intent(a.clone(), 180, false, Some(&key_a), &old_a)
            .unwrap(),
    );
    let old_b = manager.set_preview_intent(old_session, 2, &key_b).unwrap();
    let running_b = manager
        .preview_for_intent(b.clone(), 1080, false, Some(&key_b), &old_b)
        .unwrap();
    wait_for_active_preview(&ctx, &manager, &running_b);
    let launches = ctx.processes.total_spawn_count();
    let new_session = manager.begin_preview_session();
    assert_ne!(new_session, old_session);
    let new_a = manager.set_preview_intent(new_session, 1, &key_a).unwrap();
    let cancellation = promptly_cancelled(&ctx, &manager, &running_b);
    let older_session = manager
        .set_preview_intent(old_session, 9_007_199_254_740_991, &key_b)
        .unwrap_err();
    assert!(older_session.starts_with("PREVIEW_"), "{older_session}");
    for (p, height, key, token) in [(a.clone(), 180, &key_a, &old_a), (b, 1080, &key_b, &old_b)] {
        let stale = manager
            .preview_for_intent(p, height, false, Some(key), token)
            .unwrap_err();
        assert!(stale.starts_with("PREVIEW_"), "{stale}");
    }
    assert!(
        manager.preview(a.clone(), 180, false).is_err(),
        "An enrolled UI session must not admit requests missing its authoritative intent token"
    );
    let reused = manager
        .preview_for_intent(a, 180, false, Some(&key_a), &new_a)
        .unwrap();
    assert_eq!(
        (reused.id, reused.path, reused.status),
        (ready_a.id, ready_a.path, "complete".into())
    );
    assert_eq!(ctx.processes.total_spawn_count(), launches);
    assert_eq!(ctx.processes.active_count(), 0);
    no_sidecars(&ctx);
    eprintln!("VERIFIED project-session reset cancels prior worker in {cancellation:?}; old session cannot publish, render, or bypass intent admission, including identical semantic key; current session reuses validated cache without media launches.");
    manager.shutdown();
}
