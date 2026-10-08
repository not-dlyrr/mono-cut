//! Silent actual-file coverage/content gate, run after exclusive Stage4C timing.
//! The native preview files must be retained by measure-preview-continuity.mjs.
#[path = "support/stage4b.rs"]
mod fixture;
use mono_cut_lib::{media, model::*, render, storage};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn float_bits_equal(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}
// Versions 2 and 3 continue collecting after an extra strict float-bit failure. It
// never replaces that property with an epsilon or normalizes either signal.
fn float_diagnosis(a: &[f32], b: &[f32], start_sample: i64) -> Value {
    assert_eq!(a.len(), b.len());
    let mut distances = BTreeMap::<u32, usize>::new();
    let mut frames = BTreeMap::<i64, usize>::new();
    let mut first = None;
    let mut last = None;
    let mut differing = 0usize;
    let mut positive = 0usize;
    let mut negative = 0usize;
    let mut zero = 0usize;
    for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
        if x.to_bits() == y.to_bits() {
            continue;
        }
        first.get_or_insert(i);
        last = Some(i);
        differing += 1;
        *distances
            .entry(x.to_bits().abs_diff(y.to_bits()))
            .or_default() += 1;
        *frames
            .entry((start_sample + i as i64 / 2) / 1600)
            .or_default() += 1;
        positive += usize::from(x > y);
        negative += usize::from(x < y);
        zero += usize::from(x == 0. || y == 0.);
    }
    let mut lags = vec![];
    for lag in -3i64..=3 {
        let begin = 0.max(-lag) as usize;
        let end = (a.len() as i64 / 2).min(a.len() as i64 / 2 - lag) as usize;
        let mut square = 0f64;
        let mut signal = 0f64;
        let mut dot = 0f64;
        let mut sum_x = 0f64;
        let mut sum_y = 0f64;
        let mut sum_y2 = 0f64;
        let n = (end - begin) * 2;
        for frame in begin..end {
            for channel in 0..2 {
                let x = a[frame * 2 + channel] as f64;
                let y = b[((frame as i64 + lag) * 2) as usize + channel] as f64;
                square += (x - y).powi(2);
                signal += x * x;
                dot += x * y;
                sum_x += x;
                sum_y += y;
                sum_y2 += y * y;
            }
        }
        let denominator =
            ((signal - sum_x * sum_x / n as f64) * (sum_y2 - sum_y * sum_y / n as f64)).sqrt();
        lags.push(json!({"stereo_sample_lag":lag,"interleaved_samples":n,"rms":(square/n as f64).sqrt(),"correlation":if denominator>0. {(dot-sum_x*sum_y/n as f64)/denominator} else {0.},"least_squares_gain_diagnostic_only":if signal>0. {dot/signal} else {1.}}));
    }
    json!({"byte_exact":differing==0,"metrics":fixture::audio_metrics(a,b),"different_interleaved_values":differing,"raw_bit_distance_histogram":distances,"signed_error_counts":{"positive":positive,"negative":negative},"zero_or_negative_zero_differences":zero,"first_different_interleaved_index":first,"last_different_interleaved_index":last,"first_different_global_stereo_sample":first.map(|i|start_sample+i as i64/2),"last_different_global_stereo_sample":last.map(|i|start_sample+i as i64/2),"differences_per_global_video_frame":frames,"fixed_lag_controls":lags,"signals_normalized_or_scaled":false,"causal_classification":"Sparse numerical differences are recorded without a confirmed DSP cause. No strict float-bit tolerance has been substituted."})
}
fn marker(v: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(260 * 75 * 3);
    for y in 0..75 {
        out.extend_from_slice(&v[y * 960 * 3..y * 960 * 3 + 260 * 3]);
    }
    out
}
fn decoded_video(
    ctx: &media::MediaContext,
    path: &Path,
    start: i64,
    end: i64,
    format: &str,
) -> Vec<u8> {
    // Whole-second seeking is valid for this explicit30fps FFV1 output. The
    // linear checks below establish this optimization against actual files.
    let out = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-nostdin", "-threads", "1", "-ss"])
        .arg((start / 30).to_string())
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(format!(
            "trim=start_frame={}:end_frame={}",
            start % 30,
            end - start / 30 * 30
        ))
        .args([
            "-fps_mode",
            "passthrough",
            "-threads",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            format,
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn rgb_frame(ctx: &media::MediaContext, path: &Path, index: i64) -> Vec<u8> {
    decoded_video(ctx, path, index, index + 1, "rgb24")
}
fn plan_record(plan: &render::RenderPlan, output: &Path, seconds: f64) -> Value {
    json!({"output_sha256":fixture::hash(output),"output_bytes":fs::metadata(output).unwrap().len(),"encode_seconds_correctness_only":seconds,"working_region":plan.working_region,"output_frames":plan.output_frames,"input_ranges":plan.input_ranges,"audio_prefix_fallbacks":plan.audio_prefix_fallbacks,"video_seek_checks":plan.video_seek_checks,"video_prefix_fallbacks":plan.video_prefix_fallbacks,"filter_graph_sha256":digest(plan.filter_graph.as_bytes()),"args":plan.args})
}
fn full_reference(
    ctx: &media::MediaContext,
    saved: &Project,
    dir: &Path,
    tag: &str,
    proxies: bool,
) -> (PathBuf, PathBuf, Value) {
    let mut full = saved.clone();
    full.in_point = None;
    full.out_point = None;
    let path = dir.join(format!("{tag}-full-flac.mkv"));
    let plan =
        render::compile(ctx, &full, &fixture::settings("ffv1"), proxies, &path, tag).unwrap();
    let began = Instant::now();
    fixture::run(ctx, &plan.args);
    let mut record = plan_record(&plan, &path, began.elapsed().as_secs_f64());
    for frame in [0, 149, 150, 299, 300, 449, 450, 599] {
        assert_eq!(
            rgb_frame(ctx, &path, frame),
            fixture::frame(ctx, &path, frame),
            "Reference seek differs from linear decoding at{frame}"
        );
    }
    // Use the original full compiler and retain its entire audio graph. Only
    // output routing/encoding changes to expose the unquantizedfloat samples.
    let float_path = dir.join(format!("{tag}-full-float.mka"));
    let mut float_args = plan.args.clone();
    let original_graph_sha = digest(plan.filter_graph.as_bytes());
    let retained_graph = dir.join(format!("{tag}-full-filter.txt"));
    fs::write(&retained_graph, &plan.filter_graph).unwrap();
    let float_graph = format!("{};[vout]nullsink", plan.filter_graph);
    fs::write(
        dir.join(format!("{tag}-full-float-filter.txt")),
        &float_graph,
    )
    .unwrap();
    let graph_flag = float_args
        .iter()
        .position(|a| a == "-filter_complex_script")
        .unwrap();
    // Reuse this exact full plan, including its still-pinned inputs and title
    // text files. Recompiling with another job name would only change private
    // support-file paths and obscure the byte-identical audio-graph control.
    fs::write(&float_args[graph_flag + 1], &float_graph).unwrap();
    let map = float_args
        .windows(2)
        .position(|a| a == ["-map", "[vout]"])
        .unwrap();
    float_args.drain(map..map + 2);
    let codec = float_args.iter().position(|a| a == "-c:a").unwrap();
    float_args[codec + 1] = "pcm_f32le".into();
    *float_args.last_mut().unwrap() = float_path.to_string_lossy().into();
    let began = Instant::now();
    fixture::run(ctx, &float_args);
    record["float_control"] = plan_record(&plan, &float_path, began.elapsed().as_secs_f64());
    record["float_control"]["args"] = json!(float_args);
    record["float_control"]["filter_graph_sha256"] = json!(digest(float_graph.as_bytes()));
    record["float_control"]["original_full_graph_sha256"] = json!(original_graph_sha);
    record["retained_original_filter_graph_sha256"] = json!(fixture::hash(&retained_graph));
    fixture::cleanup(ctx, tag);
    (path, float_path, record)
}
fn lossless_region(
    ctx: &media::MediaContext,
    saved: &Project,
    region: &PreviewRegion,
    dir: &Path,
    tag: &str,
    proxies: bool,
) -> (PathBuf, Value) {
    let mut p = saved.clone();
    p.in_point = None;
    p.out_point = None;
    let path = dir.join(format!("{tag}-lossless-float.mkv"));
    let mut plan = render::compile_region(
        ctx,
        &p,
        &fixture::settings("ffv1"),
        proxies,
        &path,
        tag,
        region,
    )
    .unwrap();
    let codec = plan.args.iter().position(|a| a == "-c:a").unwrap();
    plan.args[codec + 1] = "pcm_f32le".into();
    let began = Instant::now();
    fs::write(
        dir.join(format!("{tag}-bounded-filter.txt")),
        &plan.filter_graph,
    )
    .unwrap();
    fixture::run(ctx, &plan.args);
    let record = plan_record(&plan, &path, began.elapsed().as_secs_f64());
    fixture::cleanup(ctx, tag);
    (path, record)
}
fn encoded_profile(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap();
    let marker = b"options: ";
    let start = bytes
        .windows(marker.len())
        .position(|p| p == marker)
        .unwrap()
        + marker.len();
    let end = start + bytes[start..].iter().position(|b| *b == 0).unwrap();
    assert!(end - start < 4096);
    let text = String::from_utf8_lossy(&bytes[start..end]);
    let values: BTreeMap<_, _> = text
        .split_whitespace()
        .filter_map(|s| s.split_once('='))
        .collect();
    assert_eq!(values["rc"], "crf");
    assert_eq!(values["crf"], "20.0");
    assert_eq!(values["keyint"], "15");
    assert_eq!(values["scenecut"], "0");
    json!({"rate_control":values["rc"],"video_crf":20,"gop_frames":15,"effective_min_keyint":values["keyint_min"],"scenecut":0})
}
fn compare(
    ctx: &media::MediaContext,
    full: &Path,
    float_full: &Path,
    native: &Path,
    saved: &Project,
    region: &PreviewRegion,
    dir: &Path,
    tag: &str,
    proxies: bool,
) -> (Value, Vec<f32>, Vec<f32>) {
    let count = region.end_frame - region.start_frame;
    let expected_gray = decoded_video(ctx, full, region.start_frame, region.end_frame, "gray");
    let actual_gray = decoded_video(ctx, native, 0, count, "gray");
    assert_eq!(expected_gray.len(), count as usize * 960 * 540);
    assert_eq!(actual_gray.len(), expected_gray.len());
    let all_gray = fixture::video_metrics(&expected_gray, &actual_gray);
    let per_frame_gray_max = expected_gray
        .chunks_exact(960 * 540)
        .zip(actual_gray.chunks_exact(960 * 540))
        .map(|(a, b)| fixture::video_metrics(a, b)["rms"].as_f64().unwrap())
        .fold(0f64, f64::max);
    assert!(
        all_gray["rms"].as_f64().unwrap() <= 5. && per_frame_gray_max <= 5.,
        "Gray picture exceeded unchanged limit5:{all_gray}/{per_frame_gray_max}"
    );
    drop(expected_gray);
    drop(actual_gray);
    let mut pictures = vec![];
    for local in [0, count / 2, count - 1] {
        let a = rgb_frame(ctx, full, region.start_frame + local);
        let b = rgb_frame(ctx, native, local);
        assert_eq!(a.len(), 960 * 540 * 3);
        assert_eq!(b.len(), a.len());
        let rgb = fixture::video_metrics(&a, &b);
        let roi = fixture::video_metrics(&marker(&a), &marker(&b));
        assert!(
            rgb["rms"].as_f64().unwrap() <= 6. && roi["rms"].as_f64().unwrap() <= 6.,
            "RGB/marker exceeded unchanged limit6:{rgb}/{roi}"
        );
        pictures.push(json!({"global_frame":region.start_frame+local,"local_frame":local,"rgb":rgb,"source_marker":roi}));
    }
    let start_sample = region.start_frame * 1600;
    let end_sample = region.end_frame * 1600;
    let expected = fixture::audio(ctx, full, start_sample, end_sample);
    let actual = fixture::audio(ctx, native, 0, end_sample - start_sample);
    assert_eq!(expected.len(), (end_sample - start_sample) as usize * 2);
    assert_eq!(
        actual.len(),
        expected.len(),
        "ExactAAC rational coverage omitted samples"
    );
    let audio = fixture::audio_metrics(&expected, &actual);
    assert!(
        audio["rms"].as_f64().unwrap() <= 0.003,
        "OrdinaryAAC exceeds unchanged.003gate:{audio}"
    );
    let expected_float = fixture::audio(ctx, float_full, start_sample, end_sample);
    let (lossless, plan) = lossless_region(ctx, saved, region, dir, tag, proxies);
    let region_float = fixture::audio(ctx, &lossless, 0, end_sample - start_sample);
    assert_eq!(region_float.len(), expected_float.len());
    let strict_float_exact = float_bits_equal(&region_float, &expected_float);
    let float_diagnostic = float_diagnosis(&expected_float, &region_float, start_sample);
    let expected_rgb = decoded_video(ctx, full, region.start_frame, region.end_frame, "rgb24");
    let region_rgb = decoded_video(ctx, &lossless, 0, count, "rgb24");
    assert_eq!(
        region_rgb, expected_rgb,
        "Lossless adjoining allRGBframes differ from full original render"
    );
    let exact_rgb_hash = digest(&region_rgb);
    drop(region_rgb);
    drop(expected_rgb);
    let control = dir.join(format!("{tag}-independent-aac.m4a"));
    fixture::run(
        ctx,
        &[
            "-i".into(),
            lossless.to_string_lossy().into(),
            "-map".into(),
            "0:a:0".into(),
            "-vn".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "160k".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
            control.to_string_lossy().into(),
        ],
    );
    let independent = fixture::audio(ctx, &control, 0, end_sample - start_sample);
    assert_eq!(independent.len(), actual.len());
    let control_metrics = fixture::audio_metrics(&actual, &independent);
    let float_bytes: Vec<u8> = region_float.iter().flat_map(|f| f.to_le_bytes()).collect();
    let result = json!({"region":region,"global_frame_coverage":[region.start_frame,region.end_frame],"global_stereo_sample_coverage":[start_sample,end_sample],"local_media_origin":Rational::from_frames(region.start_frame,saved.fps),"video_frames":count,"stereo_sample_frames":end_sample-start_sample,"native_sha256":fixture::hash(native),"encoded_profile":encoded_profile(native),"gray_allframes":all_gray,"gray_perframe_max_rms":per_frame_gray_max,"selected_rgb":pictures,"ordinary_aac_vs_full_flac":audio,"ordinary_audio_gate_passed":true,"lossless_all_rgb_exact":true,"lossless_all_rgb_sha256":exact_rgb_hash,"lossless_float_pcm_full_exact":strict_float_exact,"lossless_float_pcm_diagnosis":float_diagnostic,"lossless_float_pcm_sha256":digest(&float_bytes),"lossless_plan":plan,"matching_encoder_control":{"sha256":fixture::hash(&control),"native_decoded_samples_exact":float_bits_equal(&actual,&independent),"metrics":control_metrics,"purpose":"Encode unchanged boundedfloat samples at the actualAAC160k profile. This control classifies encoder effects only; it does not replace or relax the.003ordinary AACversusFLAC gate."}});
    (result, actual, region_float)
}

#[test]
#[ignore = "Explicit silent Stage4C actual-file full-reference check; set MONO_CUT_CONTINUITY_REPORT after timing"]
fn freshly_prepared_continuity_assets_match_original_full_render_without_frame_or_sample_gaps() {
    let report_path = PathBuf::from(
        std::env::var_os("MONO_CUT_CONTINUITY_REPORT").expect("MONO_CUT_CONTINUITY_REPORT"),
    );
    let measured: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(measured["verified_file_helper"], true);
    let proxies = measured["settings"]["useProxies"].as_bool().unwrap();
    let output = report_path.parent().unwrap().join("content-reference-v3");
    fs::create_dir_all(&output).unwrap();
    let ctx = fixture::context(&output);
    media::ensure_cache(&ctx).unwrap();
    let baseline_path = PathBuf::from(measured["project"].as_str().unwrap());
    assert_eq!(
        fixture::hash(&baseline_path),
        measured["fixture_sha256"].as_str().unwrap()
    );
    let baseline = storage::read_project(&baseline_path).unwrap();
    let coordinates = |p: &Project| {
        p.clips
            .iter()
            .map(|c| (c.id.clone(), c.start, c.duration, c.source_in))
            .collect::<Vec<_>>()
    };
    let baseline_coords = coordinates(&baseline);
    let baseline_range = (baseline.in_point, baseline.out_point);
    let before = fixture::snapshot_sources();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let changed_test = "src-tauri/tests/continuity_reference.rs";
    let mut shared_hashes = BTreeMap::new();
    for (name, expected) in measured["sources"].as_object().unwrap() {
        let actual = fixture::hash(&repo.join(name));
        if name != changed_test {
            assert_eq!(
                actual,
                expected.as_str().unwrap(),
                "Frozen measured source changed: {name}"
            );
        }
        shared_hashes.insert(name.clone(), json!({"timing_sha256":expected,"reference_sha256":actual,"same":expected==&json!(actual),"versioned_diagnostic_test_exception":name==changed_test}));
    }
    for (name, path) in [
        ("ffmpeg_sha256", &ctx.ffmpeg),
        ("ffprobe_sha256", &ctx.ffprobe),
        ("font_sha256", &ctx.font),
    ] {
        assert_eq!(
            fixture::hash(path),
            measured["tools"][name].as_str().unwrap(),
            "Frozen media tool changed: {name}"
        );
    }
    let preservation = report_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("reference-v1-preserved/preservation-manifest.json");
    // Historical captures accompany the review cohorts, but a fresh reproduction
    // has no prior failed run. Absence is reported honestly instead of requiring
    // a private artifact. Present but invalid/unreadable manifests still fail.
    let original_manifest: Option<Value> = match fs::read(&preservation) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).unwrap()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("Cannot read historical preservation manifest: {error}"),
    };
    let mut report = json!({"stage":"4C","test_source_version":3,"diagnostic_collection":"All v1 properties remain strict. Version3 preserves the Version2 strict collection policy and makes historical captures optional. It records every strict float-bit failure and continues ordinary picture/audio/reference checks before failing the aggregate assertion. The collector substitutes no epsilon or normalization and changes no production settings. The separately documented v7 shared mixing-precision fix is tested against all original strict properties.","v1_preservation_manifest":original_manifest,"measurement_report_sha256":fixture::hash(&report_path),"test_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()),"test_source_sha256":fixture::hash(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/continuity_reference.rs")),"shared_timing_source_linkage":shared_hashes,"production_and_harness_hashes_match_frozen_timing":true,"source_hashes_before":before,"ffmpeg_sha256":fixture::hash(&ctx.ffmpeg),"ffprobe_sha256":fixture::hash(&ctx.ffprobe),"font_sha256":fixture::hash(&ctx.font),"mode":measured["mode"],"native_display_or_audio":false,"tolerances":{"all_gray_rms":5,"selected_rgb_and_marker_rms":6,"ordinary_aac_vs_flac_rms":0.003},"known_sharp_pulse_fidelity_failure":"The accepted Stage4B retained missinginitialPTS sharp-pulse AACversusquantizedFLAC .003qualitygate still fails. Exact floatPCM and matchingAAC controls classified that residual; it remains a separate codec-fidelity limitation and is not changed by this ordinary continuity test.","cases":[],"references":[],"strict_float_failures":[],"verified_content":false,"preserved_saved_coordinates_and_export_range":true});
    let mut references: BTreeMap<String, (PathBuf, PathBuf)> = BTreeMap::new();
    let mut joined_native = vec![];
    let mut joined_float = vec![];
    for scenario in measured["scenarios"].as_array().unwrap() {
        let label = scenario["label"].as_str().unwrap();
        let action = scenario["action_started_ms"].as_f64();
        let files = if label == "forward-20s" {
            scenario["consumed"].as_array().unwrap()
        } else {
            scenario["files"].as_array().unwrap()
        };
        let mut seen = std::collections::BTreeSet::new();
        for file in files {
            if !seen.insert(file["job_id"].as_str().unwrap()) {
                continue;
            }
            let post_action = action
                .map(|a| file["inspection_started_ms"].as_f64().unwrap() >= a)
                .unwrap_or(false);
            if label != "forward-20s" && !post_action {
                continue;
            }
            let snapshot_key = if post_action {
                "after_project_snapshot"
            } else {
                "project_snapshot"
            };
            let snapshot = PathBuf::from(scenario[snapshot_key].as_str().unwrap());
            let hash_key = format!("{snapshot_key}_sha256");
            assert_eq!(
                fixture::hash(&snapshot),
                scenario[hash_key.as_str()].as_str().unwrap()
            );
            let saved = storage::read_project(&snapshot).unwrap();
            assert_eq!(coordinates(&saved), baseline_coords);
            assert_eq!((saved.in_point, saved.out_point), baseline_range);
            assert_eq!(saved.fps, Rational::new(30, 1));
            assert_eq!(saved.sample_rate, 48000);
            let mut semantic = serde_json::to_value(&saved).unwrap();
            semantic["name"] = Value::Null;
            let reference_key = digest(&serde_json::to_vec(&semantic).unwrap());
            if !references.contains_key(&reference_key) {
                let tag = format!("reference-{}", references.len());
                let (full, float, plan) = full_reference(&ctx, &saved, &output, &tag, proxies);
                report["references"].as_array_mut().unwrap().push(json!({"key":reference_key,"project_snapshot_sha256":fixture::hash(&snapshot),"plan":plan}));
                references.insert(reference_key.clone(), (full, float));
                fixture::write(&output, "continuity-reference-results.json", &report);
            }
            let (full, float) = references.get(&reference_key).unwrap();
            let native = PathBuf::from(file["retained_path"].as_str().unwrap());
            assert_eq!(
                fixture::hash(&native),
                file["file_sha256"].as_str().unwrap()
            );
            let region: PreviewRegion = serde_json::from_value(file["region"].clone()).unwrap();
            if file["playable"] == false {
                // The original edit endpoint is a silent picture still. The
                // existingAAC fidelity gate applies to playable coverage, not
                // a single-frame file that never carries transport audio.
                assert_eq!(region.end_frame - region.start_frame, 1);
                let a = rgb_frame(&ctx, full, region.start_frame);
                let b = rgb_frame(&ctx, &native, 0);
                assert_eq!(a.len(), 960 * 540 * 3);
                assert_eq!(b.len(), a.len());
                let rgb = fixture::video_metrics(&a, &b);
                let roi = fixture::video_metrics(&marker(&a), &marker(&b));
                assert!(rgb["rms"].as_f64().unwrap() <= 6. && roi["rms"].as_f64().unwrap() <= 6.);
                report["cases"].as_array_mut().unwrap().push(json!({"scenario":label,"region":region,"playable":false,"reference_key":reference_key,"native_sha256":fixture::hash(&native),"encoded_profile":encoded_profile(&native),"still_rgb":rgb,"still_marker":roi,"audio_gate":"The existing ordinaryAAC fidelity gate is applied separately to the playable150-frame region."}));
                fixture::write(&output, "continuity-reference-results.json", &report);
                continue;
            }
            let tag = format!("{label}-{}-{}", region.start_frame, region.end_frame);
            let (mut result, native_audio, float_audio) = compare(
                &ctx, full, float, &native, &saved, &region, &output, &tag, proxies,
            );
            result["scenario"] = json!(label);
            result["reference_key"] = json!(reference_key);
            if result["lossless_float_pcm_full_exact"] != true {
                report["strict_float_failures"].as_array_mut().unwrap().push(json!({"scenario":label,"region":region,"property":"Production bounded floatPCM must equal original full floatPCM byte for byte","diagnosis":result["lossless_float_pcm_diagnosis"]}));
            }
            result["file_ready_before_boundary"] = json!(
                label != "forward-20s"
                    || file["file_ready_ms"].as_f64().unwrap()
                        <= scenario["playback_started_ms"].as_f64().unwrap()
                            + region.start_frame as f64 / 30. * 1000.
            );
            // The first current asset is prepared beforethe transport starts;
            // all adjoining successors must be newlyvalidated beforeboundary.
            assert_eq!(result["file_ready_before_boundary"], true);
            if label == "forward-20s" {
                joined_native.extend(native_audio);
                joined_float.extend(float_audio);
            }
            report["cases"].as_array_mut().unwrap().push(result);
            fixture::write(&output, "continuity-reference-results.json", &report);
            eprintln!("Collected unchanged ordinary/lossless picture gates for {label} actual region {}..{}; strict float equality {}",region.start_frame,region.end_frame,report["cases"].as_array().unwrap().last().unwrap()["lossless_float_pcm_full_exact"]);
        }
    }
    let first_reference = report["cases"][0]["reference_key"].as_str().unwrap();
    let (full, float) = references.get(first_reference).unwrap();
    let expected_native = fixture::audio(&ctx, full, 0, 960000);
    let expected_float = fixture::audio(&ctx, float, 0, 960000);
    assert_eq!(joined_native.len(), 960000 * 2);
    assert_eq!(joined_float.len(), 960000 * 2);
    let joined_error = fixture::audio_metrics(&expected_native, &joined_native);
    assert!(joined_error["rms"].as_f64().unwrap() <= 0.003);
    let adjoining_exact = float_bits_equal(&joined_float, &expected_float);
    if !adjoining_exact {
        report["strict_float_failures"].as_array_mut().unwrap().push(json!({"scenario":"forward-20s adjoining","region":[0,600],"property":"Adjoining floatPCM cannot omit/duplicate or change a single sample bit","diagnosis":float_diagnosis(&expected_float,&joined_float,0)}));
    }
    report["adjoining"] = json!({"frame_coverage":[0,600],"stereo_sample_coverage":[0,960000],"video_frames":600,"stereo_sample_frames":960000,"regions":[[0,150],[150,300],[300,450],[450,600]],"native_aac_vs_full_flac":joined_error,"float_pcm_full_exact":adjoining_exact,"exact_frame_and_sample_coverage":true});
    assert_eq!(
        fixture::hash(&baseline_path),
        measured["fixture_sha256"].as_str().unwrap()
    );
    report["source_hashes_after"] = fixture::snapshot_sources();
    assert_eq!(
        report["source_hashes_before"],
        report["source_hashes_after"]
    );
    report["active_managed_children_after"] = json!(ctx.processes.active_count());
    assert_eq!(ctx.processes.active_count(), 0);
    for media in &baseline.media {
        let expected = measured["fixture_sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == media.id)
            .unwrap();
        assert_eq!(
            fixture::hash(Path::new(&media.path)),
            expected["original_sha256"].as_str().unwrap()
        );
        if let Some(proxy) = &media.proxy {
            assert_eq!(
                fixture::hash(Path::new(proxy)),
                expected["proxy_sha256"].as_str().unwrap()
            );
        }
    }
    report["inputs_and_tools_match_frozen_timing"] = json!(true);
    report["all_original_ordinary_picture_audio_gates_passed"] = json!(true);
    report["all_lossless_rgb_frames_exact"] = json!(true);
    report["strict_float_gate_passed"] = json!(report["strict_float_failures"]
        .as_array()
        .unwrap()
        .is_empty());
    report["verified_content"] = report["strict_float_gate_passed"].clone();
    fixture::write(&output, "continuity-reference-results.json", &report);
    assert_eq!(report["strict_float_gate_passed"],true,"Strict original float-bit properties failed; full diagnostic report is retained, verified_content remains false");
}
