//! Explicit actual-media Stage 4B benchmark; never plays media or starts the UI.
#[path = "support/stage4b.rs"]
mod fixture;
use mono_cut_lib::{jobs::JobManager, model::*, storage};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
#[ignore = "explicit 1080p fixture generation; run with MONO_CUT_REGION_BENCH_DIR"]
fn prepare_reproducible_ordinary_fixture_and_preserve_stress_evidence() {
    let dir = fixture::directory();
    let ctx = fixture::context(&dir);
    fixture::preserve_previous(&dir);
    fixture::fixture(&ctx, &dir);
}

#[test]
#[ignore = "Explicit prepared-proxy fixture generation, separate from warm preview timing"]
fn prepare_normalized_proxies_for_ordinary_fixture() {
    let dir = fixture::directory();
    let ctx = fixture::context(&dir);
    let mut p = fixture::fixture(&ctx, &dir);
    let jobs = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let began = Instant::now();
    let mut sources = vec![];
    let mut pins = vec![];
    for m in &mut p.media {
        let start = Instant::now();
        let job = jobs.proxy(m.clone()).unwrap();
        let complete = fixture::wait(&jobs, &job.id, Duration::from_secs(120));
        let path = PathBuf::from(complete.path.unwrap());
        pins.push(ctx.processes.pin_file(&path).unwrap());
        m.proxy = Some(path.to_string_lossy().into());
        m.proxy_timing_version = Some(NORMALIZED_TIMING_VERSION);
        sources.push(json!({"media_id":m.id,"seconds":start.elapsed().as_secs_f64(),"filename":path.file_name().unwrap().to_string_lossy(),"sha256":fixture::hash(&path),"bytes":fs::metadata(&path).unwrap().len()}));
    }
    let project_path = dir.join("ordinary-60s-proxy.monocut");
    storage::write_atomic(&project_path, &p).unwrap();
    fixture::write(
        &dir,
        "proxy-preparation.json",
        &json!({"source_hashes":fixture::snapshot_sources(),"test_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()),"prepared_seconds":began.elapsed().as_secs_f64(),"project_sha256":fixture::hash(&project_path),"sources":sources,"proxy_recipe":"engine normalized1280x720 H264CRF23,sharedclock,AAC audio","native_display_or_audible_output":false}),
    );
    jobs.shutdown();
    drop(pins);
    assert_eq!(ctx.processes.active_count(), 0);
}
