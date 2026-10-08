//! Ownership, failure and source/tool identity checks for initial video clocks.
//! A tiny disposable native probe process makes cancellation deterministic;
//! real H.264 missing-PTS decoding is covered by irregular_pts.rs.
use mono_cut_lib::{media::MediaContext, model::*, preview, video_seek};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn helper() -> &'static Path {
    static HELPER: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    &HELPER.get_or_init(|| {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("packet_probe.rs");
        fs::write(&source, r#"
use std::{fs,io::{self,Write},thread,time::Duration};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(args.windows(2).any(|a| a[0]=="-read_intervals"&&a[1]=="%+#256"));
    let path=args.last().unwrap(); let mode=fs::read_to_string(path).unwrap();
    match mode.trim() {
        "slow" => { thread::sleep(Duration::from_secs(60)); },
        "overflow" => { let block=vec![b'x';131_072]; let _=io::stdout().write_all(&block); },
        "malformed" => { print!("broken-json"); },
        "failure" => { eprintln!("deliberate probe failure"); std::process::exit(2); },
        "empty" => { print!("{{\"packets\":[]}}"); },
        "mutate" => { fs::write(path,"regular-plus-new-stamp").unwrap(); print!("{{\"packets\":[{{\"pts\":0}}]}}"); },
        "missing" => { print!("{{\"packets\":[{{}},{{\"pts\":0}},{{\"pts\":3}}]}}"); },
        // Reordered PTS and initially absent DTS are ordinary B-frame behavior.
        _ => { print!("{{\"packets\":[{{\"pts\":0}},{{\"pts\":3}},{{\"pts\":1}},{{\"pts\":2}}]}}"); },
    }
}
"#).unwrap();
        let executable = temp.path().join(if cfg!(windows) {"packet_probe.exe"} else {"packet_probe"});
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&source).arg("--edition=2021").arg("-o").arg(&executable).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        (temp, executable)
    }).1
}
fn ctx(dir: &Path) -> MediaContext {
    MediaContext {
        ffmpeg: helper().to_path_buf(),
        ffprobe: helper().to_path_buf(),
        cache_dir: dir.join("cache"),
        font: dir.join("unused.ttf"),
        processes: Default::default(),
    }
}
fn no_owners(ctx: &MediaContext) {
    assert_eq!(ctx.processes.active_count(), 0);
    assert_eq!(ctx.processes.protected_assets().pin_count, 0);
    assert!(
        !ctx.cache_dir.exists(),
        "Timestamp classification writes no files"
    );
}

#[test]
fn initial_clock_cache_obeys_source_stream_assessor_stamps_and_cancelled_hits() {
    let temp = TempDir::new().unwrap();
    let mut ctx = ctx(temp.path());
    let source = temp.path().join("source.bin");
    fs::write(&source, "regular").unwrap();
    let count = ctx.processes.total_spawn_count();
    let first = video_seek::inspect(&ctx, &source, "0", None).unwrap();
    assert!(!first.cache_hit && !first.needs_prefix());
    let warm = video_seek::inspect(&ctx, &source, "0", None).unwrap();
    assert!(warm.cache_hit);
    assert_eq!(first.identity_key, warm.identity_key);
    assert_eq!(ctx.processes.total_spawn_count() - count, 1);
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(video_seek::inspect(&ctx, &source, "0", Some(&cancelled))
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(ctx.processes.total_spawn_count() - count, 1);
    let other_stream = video_seek::inspect(&ctx, &source, "1", None).unwrap();
    assert!(!other_stream.cache_hit);
    assert_ne!(first.identity_key, other_stream.identity_key);
    fs::write(&source, "missing").unwrap();
    // Same-length rewrite still invalidates by the precise modification stamp.
    let next = video_seek::inspect(&ctx, &source, "0", None).unwrap();
    assert!(!next.cache_hit && next.needs_prefix());
    assert_eq!(next.first_missing_packet, Some(0));
    assert_eq!(next.missing_pts, 1);
    assert_ne!(first.identity_key, next.identity_key);
    let copied_probe = temp.path().join(if cfg!(windows) {
        "other-probe.exe"
    } else {
        "other-probe"
    });
    fs::copy(helper(), &copied_probe).unwrap();
    ctx.ffprobe = copied_probe;
    let changed_tool = video_seek::inspect(&ctx, &source, "0", None).unwrap();
    assert!(!changed_tool.cache_hit && changed_tool.needs_prefix());
    assert_ne!(next.identity_key, changed_tool.identity_key);
    no_owners(&ctx);
}

#[test]
fn running_packet_inspection_cancels_reaps_and_retries_without_cached_failure() {
    let temp = TempDir::new().unwrap();
    let ctx = ctx(temp.path());
    let source = temp.path().join("slow.bin");
    fs::write(&source, "slow").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_ctx = ctx.clone();
    let worker_source = source.clone();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        video_seek::inspect(&worker_ctx, &worker_source, "0", Some(&worker_cancel))
    });
    let start = Instant::now();
    while ctx.processes.active_count() == 0 {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(ctx.processes.protected_assets().pin_count, 2);
    cancel.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(3));
    no_owners(&ctx);
    cancel.store(false, Ordering::Release);
    fs::write(&source, "regular").unwrap();
    let retry = video_seek::inspect(&ctx, &source, "0", Some(&cancel)).unwrap();
    assert!(!retry.cache_hit && !retry.needs_prefix());
    no_owners(&ctx);
}

#[test]
fn invalid_failed_oversized_or_changed_source_probe_never_authorizes_fast_seek() {
    let temp = TempDir::new().unwrap();
    let ctx = ctx(temp.path());
    let source = temp.path().join("failure.bin");
    for mode in ["failure", "malformed", "empty", "overflow", "mutate"] {
        fs::write(&source, mode).unwrap();
        let before = ctx.processes.total_spawn_count();
        let error = video_seek::inspect(&ctx, &source, "0", None).unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(ctx.processes.total_spawn_count() - before, 1);
        no_owners(&ctx);
        // Retry the same failed stamp. Mutation is reset to its initiating input.
        if mode == "mutate" {
            fs::write(&source, mode).unwrap();
        }
        assert!(video_seek::inspect(&ctx, &source, "0", None).is_err());
        assert_eq!(ctx.processes.total_spawn_count() - before, 2);
        no_owners(&ctx);
    }
    fs::write(&source, "regular").unwrap();
    let retry = video_seek::inspect(&ctx, &source, "0", None).unwrap();
    assert!(!retry.cache_hit && !retry.needs_prefix());
    no_owners(&ctx);
}

#[test]
fn changed_timestamp_assessor_invalidates_preview_identity_and_queued_admission() {
    let temp = TempDir::new().unwrap();
    let mut ctx = ctx(temp.path());
    let p = Project::new("Identity".into(), 160, 90, Rational::new(24_000, 1001));
    let identity = preview::identity(&ctx, &p, 120, false).unwrap();
    let admission = preview::input_stamp_snapshot(&ctx, &p, false);
    let other = temp.path().join(if cfg!(windows) {
        "assessor.exe"
    } else {
        "assessor"
    });
    fs::copy(helper(), &other).unwrap();
    ctx.ffprobe = other.clone();
    assert_ne!(identity, preview::identity(&ctx, &p, 120, false).unwrap());
    assert!(admission.iter().any(|(path, _)| path == helper()));
    let changed_admission = preview::input_stamp_snapshot(&ctx, &p, false);
    let file = fs::OpenOptions::new().write(true).open(&other).unwrap();
    file.set_len(fs::metadata(&other).unwrap().len() + 1)
        .unwrap();
    assert!(!preview::input_stamps_current(&changed_admission));
    no_owners(&ctx);
}
