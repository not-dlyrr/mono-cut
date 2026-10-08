//! Ownership, rather than recent modification time, protects live cache assets.
use mono_cut_lib::{
    media::{prune_cache, MediaContext},
    processes::ProcessRegistry,
};
use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tempfile::TempDir;

fn context(dir: &Path) -> MediaContext {
    MediaContext {
        ffmpeg: PathBuf::from("ffmpeg"),
        ffprobe: PathBuf::from("ffprobe"),
        font: PathBuf::from("Inter.ttf"),
        cache_dir: dir.to_path_buf(),
        processes: Arc::new(ProcessRegistry::default()),
    }
}
fn old_file(path: &Path, bytes: usize) {
    fs::write(path, vec![7; bytes]).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
        .unwrap();
}

#[test]
fn aged_playback_and_worker_assets_survive_pressure_until_every_owner_releases() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let preview = dir.path().join("preview-old.mp4");
    let source = dir.path().join("source-old.mp4");
    let proxy = dir.path().join("proxy-old.mp4");
    let stale = dir.path().join("unused-old.mp4");
    let partial = dir.path().join(".mono-part-running.mp4");
    for path in [&preview, &source, &proxy, &stale, &partial] {
        old_file(path, 64);
    }
    let preview_pin = ctx.processes.pin_file(&preview).unwrap();
    let source_player = ctx.processes.pin_file(&source).unwrap();
    let source_worker = ctx
        .processes
        .pin_file(dir.path().join(".").join("source-old.mp4"))
        .unwrap();
    let proxy_worker = ctx.processes.pin_file(&proxy).unwrap();
    let protected = ctx.processes.protected_assets();
    assert_eq!(
        (protected.file_count, protected.pin_count, protected.bytes),
        (3, 4, 192)
    );
    prune_cache(&ctx, 0).unwrap();
    assert!(
        !stale.exists(),
        "An unused aged asset must be eligible for eviction"
    );
    assert!(preview.exists() && source.exists() && proxy.exists());
    assert!(
        partial.exists(),
        "In-progress partial files retain their separate exclusion"
    );
    drop(source_player);
    prune_cache(&ctx, 0).unwrap();
    assert!(
        source.exists(),
        "A second owner still protects the same source"
    );
    drop(source_worker);
    prune_cache(&ctx, 0).unwrap();
    assert!(
        !source.exists(),
        "After final owner release the old source is evictable"
    );
    drop(preview_pin);
    drop(proxy_worker);
    prune_cache(&ctx, 0).unwrap();
    assert!(!preview.exists() && !proxy.exists());
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
}

#[test]
fn precreation_pin_survives_atomic_cache_commit_and_path_aliases() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let output = dir.path().join("future-preview.mp4");
    let before_commit = ctx.processes.pin_file(&output).unwrap();
    assert_eq!(ctx.processes.protected_assets().bytes, 0);
    let part = dir.path().join(".mono-part-commit.mp4");
    old_file(&part, 100);
    fs::rename(&part, &output).unwrap();
    let alias = dir.path().join(".").join("future-preview.mp4");
    assert!(ctx.processes.is_protected(&alias));
    assert_eq!(ctx.processes.protected_assets().bytes, 100);
    assert!(!ctx.processes.remove_unprotected_file(&alias).unwrap());
    prune_cache(&ctx, 0).unwrap();
    assert!(output.exists());
    let second = ctx
        .processes
        .pin_file(output.canonicalize().unwrap())
        .unwrap();
    drop(before_commit);
    assert!(ctx.processes.is_protected(&output));
    drop(second);
    assert!(ctx.processes.remove_unprotected_file(&alias).unwrap());
    assert!(!output.exists());
}

#[test]
fn ordinary_unused_cache_stays_within_budget_and_closing_rejects_new_pins() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    for index in 0..8 {
        old_file(&dir.path().join(format!("stale-{index}.mp4")), 64);
    }
    prune_cache(&ctx, 128).unwrap();
    let bytes: u64 = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.metadata().unwrap().len())
        .sum();
    assert!(bytes <= 128);
    ctx.processes.shutdown();
    assert!(ctx
        .processes
        .pin_file(dir.path().join("after-close.mp4"))
        .is_err());
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
}

#[test]
fn successful_spawn_counter_is_independent_of_temporary_files_and_pins() {
    let registry = Arc::new(ProcessRegistry::default());
    let dir = TempDir::new().unwrap();
    let pin = registry.pin_file(dir.path().join("asset.mp4")).unwrap();
    let temporary = registry
        .temporary_file(dir.path().join("temporary.mp4"))
        .unwrap();
    assert_eq!(registry.total_spawn_count(), 0);
    let missing = registry.spawn(&mut std::process::Command::new(
        "mono-cut-definitely-missing-test-command",
    ));
    assert!(missing.is_err());
    assert_eq!(registry.total_spawn_count(), 0);
    #[cfg(windows)]
    let mut command = {
        let mut c = mono_cut_lib::media::command(Path::new("cmd.exe"));
        c.args(["/c", "exit", "0"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "exit 0"]);
        c
    };
    let child = registry.spawn(&mut command).unwrap();
    assert!(child.wait().unwrap().success());
    drop(child);
    assert_eq!(registry.total_spawn_count(), 1);
    assert_eq!(registry.active_count(), 0);
    drop(temporary);
    drop(pin);
    assert_eq!(registry.total_spawn_count(), 1);
}
