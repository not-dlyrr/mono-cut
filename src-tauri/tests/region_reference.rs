//! Explicit ordinary-workload full-reference comparison. Decode/encode silently.
//! This gate is separate from performance timing and is ignored in ordinary CI.
#[path = "support/stage4b.rs"]
mod fixture;
use mono_cut_lib::{edit, jobs::JobManager, media, model::*, preview, render, storage};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn retained_plan(
    ctx: &media::MediaContext,
    p: &Project,
    dir: &Path,
    name: &str,
    proxies: bool,
    region: Option<&PreviewRegion>,
    codec: &str,
) -> (PathBuf, Value) {
    let output = dir.join(format!(
        "{name}.{}",
        if codec == "h264" { "mp4" } else { "mkv" }
    ));
    let plan = match (codec, region) {
        ("h264", region) => {
            preview::compile(ctx, p, 540, proxies, &output, name, region, None).unwrap()
        }
        (_, Some(r)) => {
            render::compile_region(ctx, p, &fixture::settings(codec), proxies, &output, name, r)
                .unwrap()
        }
        (_, None) => {
            render::compile(ctx, p, &fixture::settings(codec), proxies, &output, name).unwrap()
        }
    };
    fs::write(dir.join(format!("{name}-filters.txt")), &plan.filter_graph).unwrap();
    let graph_hash = fixture::hash(&dir.join(format!("{name}-filters.txt")));
    let mut input_options = vec![];
    let mut block_start = plan
        .args
        .iter()
        .position(|arg| arg == "-filter_complex_threads")
        .map(|index| index + 2)
        .unwrap_or(0);
    for (index, arg) in plan.args.iter().enumerate() {
        if arg == "-filter_complex_script" {
            break;
        }
        if arg == "-i" {
            input_options.push(plan.args[block_start..=index + 1].to_vec());
            block_start = index + 2;
        }
    }
    let evidence = json!({"working_region":plan.working_region,"output_frames":plan.output_frames,"input_ranges":plan.input_ranges,"audio_prefix_fallbacks":plan.audio_prefix_fallbacks,"video_seek_checks":plan.video_seek_checks,"video_prefix_fallbacks":plan.video_prefix_fallbacks,"filter_graph_sha256":graph_hash,"input_options":input_options,"input_range_semantics":"Seek/duration describe the selected input policy, not a hard packet/decode-read cap. Regular sampled initial PTS permits optimized seeking with exact filter trim EOF and unavoidable GOP preroll/read-ahead. Missing initial PTS uses the explicit origin-prefix video fallback; the 256-packet assessment does not certify later timestamps. PCM audio limits and long-source audio-prefix fallbacks are separate."});
    let began = Instant::now();
    fixture::run(ctx, &plan.args);
    let seconds = began.elapsed().as_secs_f64();
    fixture::cleanup(ctx, name);
    let mut record = evidence;
    record["encode_seconds"] = json!(seconds);
    record["output_sha256"] = json!(fixture::hash(&output));
    record["output_bytes"] = json!(fs::metadata(&output).unwrap().len());
    (output, record)
}
fn gray(ctx: &media::MediaContext, path: &Path, start: i64, end: i64) -> Vec<u8> {
    seeked_decoded(
        ctx,
        path,
        start / 30,
        &[
            "-vf".into(),
            format!(
                "trim=start_frame={}:end_frame={}",
                start % 30,
                end - start / 30 * 30
            ),
            "-an".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "gray".into(),
        ],
    )
}
fn seeked_decoded(
    ctx: &media::MediaContext,
    path: &Path,
    whole_seconds: i64,
    args: &[String],
) -> Vec<u8> {
    // These fixtures output exactly30fps. Seeking to an integer second avoids
    // Matroska's millisecond quantization at fractional-frame boundaries; the
    // remaining frame indices are selected after decode. A linear-decode check
    // below proves this optimization rather than assuming frame alignment.
    let output = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-nostdin", "-ss"])
        .arg(whole_seconds.to_string())
        .arg("-i")
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
fn frame(ctx: &media::MediaContext, path: &Path, index: i64) -> Vec<u8> {
    seeked_decoded(
        ctx,
        path,
        index / 30,
        &[
            "-vf".into(),
            format!("select=eq(n\\,{})", index % 30),
            "-frames:v".into(),
            "1".into(),
            "-an".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgb24".into(),
        ],
    )
}
fn check_reference_seek(ctx: &media::MediaContext, path: &Path) {
    for index in [0, 1, 29, 30, 31, 340, 370, 1770, 1799] {
        assert_eq!(
            frame(ctx, path, index),
            fixture::frame(ctx, path, index),
            "Whole-second reference seek must agree with linear decode at frame{index}"
        );
    }
}
fn marker_roi(v: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for y in 0..75 {
        out.extend_from_slice(&v[y * 960 * 3..y * 960 * 3 + 260 * 3]);
    }
    out
}
fn compare(ctx: &media::MediaContext, reference: &Path, output: &Path, r: &PreviewRegion) -> Value {
    let count = r.end_frame - r.start_frame;
    let a = gray(ctx, reference, r.start_frame, r.end_frame);
    let b = gray(ctx, output, 0, count);
    assert_eq!(
        a.len(),
        count as usize * 960 * 540,
        "Full reference cadence"
    );
    assert_eq!(b.len(), a.len(), "Region cadence");
    let pixels = fixture::video_metrics(&a, &b);
    // Declared before measurement: actual H.264CRF20/GOP15 preview to FFV1
    // reference. A wrong source-frame marker is checked separately below.
    assert!(
        pixels["rms"].as_f64().unwrap() <= 5.,
        "Video exceeds codec tolerance: {pixels}"
    );
    let mut frames = vec![];
    for index in [0, count / 2, count - 1] {
        let a = frame(ctx, reference, r.start_frame + index);
        let b = frame(ctx, output, index);
        assert_eq!(a.len(), 960 * 540 * 3);
        let metrics = fixture::video_metrics(&a, &b);
        assert!(
            metrics["rms"].as_f64().unwrap() <= 6.,
            "Frame exceeds codec tolerance: {metrics}"
        );
        // Baked source marker ROI is at top left of base footage; selecting
        // the wrong source cadence must not disappear in a large-image RMS.
        let marker = fixture::video_metrics(&marker_roi(&a), &marker_roi(&b));
        assert!(
            marker["rms"].as_f64().unwrap() <= 6.,
            "Frame marker exceeds codec tolerance: {marker}"
        );
        frames.push(json!({"global_frame":r.start_frame+index,"local_frame":index,"pixels":metrics,"marker_roi":marker}));
    }
    let first = r.start_frame * 1600;
    let last = r.end_frame * 1600;
    let a = fixture::audio(ctx, reference, first, last);
    let b = fixture::audio(ctx, output, 0, last - first);
    assert_eq!(a.len(), (last - first) as usize * 2);
    assert_eq!(
        b.len(),
        a.len(),
        "Exact sample coverage after AAC priming/padding crop"
    );
    let audio = fixture::audio_metrics(&a, &b);
    assert!(
        audio["rms"].as_f64().unwrap() <= 0.003,
        "Audio exceeds declared AAC tolerance: {audio}"
    );
    json!({"region":r,"origin_seconds":Rational::from_frames(r.start_frame,Rational::new(30,1)),"decoded_video_frames":count,"video_all_gray":pixels,"frame_samples":frames,"exact_stereo_sample_frames":last-first,"audio":audio})
}
#[test]
#[ignore = "Explicit silent 60s full-reference original/proxy correctness gate; run after performance measurement"]
fn ordinary_regions_match_shared_full_reference_with_exact_adjoining_coverage() {
    let dir = fixture::directory();
    let outdir = dir.join("reference");
    fs::create_dir_all(&outdir).unwrap();
    let ctx = fixture::context(&outdir);
    let original = fixture::fixture(&ctx, &dir);
    let immutable = serde_json::to_value(&original).unwrap();
    let mut p = original.clone();
    p.in_point = None;
    p.out_point = None;
    let jobs = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let mut report = json!({"source_hashes_before":fixture::snapshot_sources(),"test_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()),"ffmpeg_sha256":fixture::hash(&ctx.ffmpeg),"ffprobe_sha256":fixture::hash(&ctx.ffprobe),"font_sha256":fixture::hash(&ctx.font),"fixture_project_sha256":fixture::hash(&dir.join("ordinary-60s.monocut")),"driver":"region_reference.rs ignored actual-media test","native_display_or_audible_output":false,"video_tolerance":"gray allframes RMS<=5; RGB first/middle/last and source-marker ROI RMS<=6 on 0..255, actual shared H264CRF20/GOP15 preview versus losslessFFV1","audio_tolerance":"AAC160kbps versus losslessFLAC RMS<=0.003 amplitude, exact declared rational coverage crop, all stereo samples compared","sample_rate":48000,"cases":[]});
    // Adjacent 5s regions at 25/30s exercise title+speed+audio automation.
    // Other ranges cross dissolves, source clip boundaries and both sequence ends.
    let ranges = [
        (0, 150),
        (330, 480),
        (690, 840),
        (750, 900),
        (900, 1050),
        (1050, 1200),
        (1650, 1800),
    ];
    let mut retained_proxies = vec![];
    for proxies in [false, true] {
        if proxies {
            let began = Instant::now();
            let mut source_proxies = vec![];
            for m in &mut p.media {
                let job = jobs.proxy(m.clone()).unwrap();
                let job = fixture::wait(&jobs, &job.id, Duration::from_secs(90));
                let path = PathBuf::from(job.path.unwrap());
                retained_proxies.push(ctx.processes.pin_file(&path).unwrap());
                m.proxy = Some(path.to_string_lossy().into());
                m.proxy_timing_version = Some(NORMALIZED_TIMING_VERSION);
                source_proxies.push(json!({"media_id":m.id,"sha256":fixture::hash(&path),"bytes":fs::metadata(&path).unwrap().len()}));
            }
            report["proxy_preparation"] =
                json!({"seconds":began.elapsed().as_secs_f64(),"files":source_proxies});
            fixture::write(&outdir, "reference-results.json", &report);
        }
        let mode = if proxies { "proxy" } else { "original" };
        let (reference, plan) = retained_plan(
            &ctx,
            &p,
            &outdir,
            &format!("full-{mode}"),
            proxies,
            None,
            "ffv1",
        );
        check_reference_seek(&ctx, &reference);
        report[format!("{mode}_full_reference")] = plan;
        fixture::write(&outdir, "reference-results.json", &report);
        let mut adjoining = vec![];
        for (start, end) in ranges {
            let r = PreviewRegion {
                start_frame: start,
                end_frame: end,
            };
            let (output, plan) = retained_plan(
                &ctx,
                &p,
                &outdir,
                &format!("region-{mode}-{start}-{end}"),
                proxies,
                Some(&r),
                "h264",
            );
            assert!(
                plan["working_region"]["end_frame"].as_i64().unwrap()
                    - plan["working_region"]["start_frame"].as_i64().unwrap()
                    <= 180,
                "Working base must be region-bounded"
            );
            assert!(
                plan["input_ranges"].as_array().unwrap().len() <= 9,
                "Inactive clips must not be decoded"
            );
            for input in plan["input_ranges"].as_array().unwrap() {
                let duration: Rational = serde_json::from_value(input["duration"].clone()).unwrap();
                assert!(
                    duration.value() <= 8.,
                    "Nominal contributing source span exceeds the region plan: {input}"
                );
            }
            let mut case = compare(&ctx, &reference, &output, &r);
            case["mode"] = json!(mode);
            case["render_plan"] = plan;
            if start == 750 || start == 900 {
                adjoining.extend(fixture::audio(&ctx, &output, 0, 240000));
            }
            report["cases"].as_array_mut().unwrap().push(case);
            fixture::write(&outdir, "reference-results.json", &report);
        }
        let joined_reference = fixture::audio(&ctx, &reference, 750 * 1600, 1050 * 1600);
        assert_eq!(
            adjoining.len(),
            joined_reference.len(),
            "Adjoining exact origins cannot omit or duplicate samples"
        );
        let metrics = fixture::audio_metrics(&joined_reference, &adjoining);
        assert!(metrics["rms"].as_f64().unwrap() <= 0.003);
        report[format!("{mode}_adjoining_audio")] = json!({"regions":[[750,900],[900,1050]],"global_sample_coverage":[1200000,1680000],"samples":adjoining.len()/2,"metrics":metrics});
        fixture::write(&outdir, "reference-results.json", &report);
    }
    assert_eq!(
        serde_json::to_value(&original).unwrap(),
        immutable,
        "Preview never changes saved coordinates or user export range"
    );
    assert_eq!(
        serde_json::to_value(storage::read_project(&dir.join("ordinary-60s.monocut")).unwrap())
            .unwrap(),
        immutable,
        "The pre-preview saved fixture is preserved"
    );
    jobs.set_playback_assets(None, None).unwrap();
    jobs.shutdown();
    drop(retained_proxies);
    report["source_hashes_after"] = fixture::snapshot_sources();
    report["source_hashes_unchanged"] =
        json!(report["source_hashes_before"] == report["source_hashes_after"]);
    report["active_managed_children_after"] = json!(ctx.processes.active_count());
    report["cache_bytes"] = json!(fixture::disk(&ctx.cache_dir));
    fixture::write(&outdir, "reference-results.json", &report);
}

#[test]
#[ignore = "Explicit full-reference verification of every retained production-controller edit; use MONO_CUT_REGION_PRODUCTION_REPORT"]
fn timed_production_edits_decode_to_their_own_shared_full_reference() {
    let path = PathBuf::from(
        std::env::var_os("MONO_CUT_REGION_PRODUCTION_REPORT")
            .expect("MONO_CUT_REGION_PRODUCTION_REPORT"),
    );
    let measured: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let output = path.parent().unwrap().join("full-reference-verification");
    fs::create_dir_all(&output).unwrap();
    let ctx = fixture::context(&output);
    media::ensure_cache(&ctx).unwrap();
    let proxies = measured["settings"]["useProxies"].as_bool().unwrap();
    let baseline_path = PathBuf::from(measured["project"].as_str().unwrap());
    let baseline_sha256 = fixture::hash(&baseline_path);
    assert_eq!(
        baseline_sha256,
        measured["fixture_sha256"].as_str().unwrap()
    );
    let baseline = storage::read_project(&baseline_path).unwrap();
    let baseline_range = (baseline.in_point, baseline.out_point);
    let coordinates = |p: &Project| {
        p.clips
            .iter()
            .map(|c| (c.id.clone(), c.start, c.duration, c.source_in))
            .collect::<Vec<_>>()
    };
    let baseline_coordinates = coordinates(&baseline);
    let samples = measured["samples"].as_array().unwrap();
    assert_eq!(
        samples.len(),
        11,
        "One cold preparation and ten real edits are required"
    );
    assert_eq!(samples[0]["label"], "cold-first-preparation");
    assert!(samples[0]["command"].is_null());
    let mut expected_edits = baseline.clone();
    let mut evidence = json!({"measurement_report_sha256":fixture::hash(&path),"test_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()),"source_hashes_before":fixture::snapshot_sources(),"ffmpeg_sha256":fixture::hash(&ctx.ffmpeg),"ffprobe_sha256":fixture::hash(&ctx.ffprobe),"font_sha256":fixture::hash(&ctx.font),"mode":measured["mode"],"pre_preview_project_sha256":baseline_sha256,"pre_preview_export_range":baseline_range,"required_actual_edits":10,"purpose":"Each timed production first frame and 5s region is checked against its exact saved edited project through the shared whole-sequence compiler. This verification happens after timing, with no native display or sound.","video_tolerance":"Gray allframes RMS<=5; RGB+source-marker ROI first/middle/last RMS<=6 (0..255), actual shared H264CRF20/GOP15 preview versus losslessFFV1","audio_tolerance":"AAC160kbps versus losslessFLAC RMS<=0.003, exact 48000 stereo sample coverage cropped after decoder priming/padding","cases":[]});
    for (index, sample) in samples.iter().enumerate() {
        if index > 0 {
            assert_eq!(sample["label"], format!("warm-edit-{index}"));
            assert_eq!(sample["command"]["type"], "update_clip");
            let before = serde_json::to_value(&expected_edits.clips).unwrap();
            edit::apply(
                &mut expected_edits,
                serde_json::from_value(sample["command"].clone()).unwrap(),
            )
            .unwrap();
            assert_ne!(
                serde_json::to_value(&expected_edits.clips).unwrap(),
                before,
                "Every recorded edit must actually change a clip"
            );
        }
        let label = sample["label"].as_str().unwrap();
        let snapshot = PathBuf::from(sample["project_snapshot"].as_str().unwrap());
        assert_eq!(
            fixture::hash(&snapshot),
            sample["project_snapshot_sha256"].as_str().unwrap()
        );
        let saved = storage::read_project(&snapshot).unwrap();
        assert_eq!(
            (saved.in_point, saved.out_point),
            baseline_range,
            "Preview preserves the pre-preview user export range"
        );
        assert_eq!(
            coordinates(&saved),
            baseline_coordinates,
            "Preview preserves pre-preview clip coordinates"
        );
        assert_eq!(
            serde_json::to_value(&saved.clips).unwrap(),
            serde_json::to_value(&expected_edits.clips).unwrap(),
            "Snapshot contains exactly the recorded cumulative real edits"
        );
        let original_range = (saved.in_point, saved.out_point);
        let original_clip_coordinates: Vec<_> = saved
            .clips
            .iter()
            .map(|c| (c.id.clone(), c.start, c.duration, c.source_in))
            .collect();
        let mut full = saved.clone();
        full.in_point = None;
        full.out_point = None;
        let (reference, plan) = retained_plan(
            &ctx,
            &full,
            &output,
            &format!("production-{label}"),
            proxies,
            None,
            "ffv1",
        );
        let first_path = PathBuf::from(sample["first_frame"]["path"].as_str().unwrap());
        let first_region: PreviewRegion =
            serde_json::from_value(sample["first_frame"]["region"].clone()).unwrap();
        let playhead = sample["playhead"].as_i64().unwrap();
        if evidence["cases"].as_array().unwrap().is_empty() {
            check_reference_seek(&ctx, &reference);
        }
        let a = frame(&ctx, &reference, playhead);
        let b = frame(&ctx, &first_path, playhead - first_region.start_frame);
        let metrics = fixture::video_metrics(&a, &b);
        assert!(
            metrics["rms"].as_f64().unwrap() <= 6.,
            "Timed changed first frame exceeds declared tolerance: {label} {metrics}"
        );
        let marker = fixture::video_metrics(&marker_roi(&a), &marker_roi(&b));
        assert!(
            marker["rms"].as_f64().unwrap() <= 6.,
            "Timed first-frame source marker disagrees with full reference: {label} {marker}"
        );
        let first_sha = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&b);
            format!("{:x}", h.finalize())
        };
        assert_eq!(
            first_sha,
            sample["first_frame"]["sha256"].as_str().unwrap(),
            "Timed decoded frame no longer matches retained artifact"
        );
        let region: PreviewRegion =
            serde_json::from_value(sample["continuous"]["region"].clone()).unwrap();
        assert_eq!(
            region.end_frame - region.start_frame,
            150,
            "Timed continuous coverage must contain 5 seconds"
        );
        let path = PathBuf::from(sample["continuous"]["path"].as_str().unwrap());
        assert_eq!(
            fixture::hash(&path),
            sample["continuous"]["file_sha256"].as_str().unwrap()
        );
        let mut case = compare(&ctx, &reference, &path, &region);
        case["label"] = json!(label);
        case["edit_command"] = sample["command"].clone();
        case["timed_first_frame_rgb"] = metrics;
        case["timed_first_frame_marker_roi"] = marker;
        case["full_reference_plan"] = plan;
        case["saved_export_range"] = json!(original_range);
        case["pre_preview_export_range_and_coordinates_preserved"] = json!(true);
        case["recorded_cumulative_real_edits_match_saved_clips"] = json!(true);
        case["project_snapshot_sha256"] = sample["project_snapshot_sha256"].clone();
        let reread = storage::read_project(&snapshot).unwrap();
        assert_eq!((reread.in_point, reread.out_point), original_range);
        assert_eq!(
            reread
                .clips
                .iter()
                .map(|c| (c.id.clone(), c.start, c.duration, c.source_in))
                .collect::<Vec<_>>(),
            original_clip_coordinates
        );
        evidence["cases"].as_array_mut().unwrap().push(case);
        fixture::write(&output, "production-reference-results.json", &evidence);
        eprintln!("Verified exact edited full reference for {label}");
    }
    evidence["source_hashes_after"] = fixture::snapshot_sources();
    evidence["source_hashes_unchanged"] =
        json!(evidence["source_hashes_before"] == evidence["source_hashes_after"]);
    evidence["active_managed_children_after"] = json!(ctx.processes.active_count());
    fixture::write(&output, "production-reference-results.json", &evidence);
}
