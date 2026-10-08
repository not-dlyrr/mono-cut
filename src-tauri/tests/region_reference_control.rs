//! Explicit diagnosis of the retained, predeclared CRF22 marker failure.
//! This leaves the original gate and production settings unchanged.
#[path = "support/stage4b.rs"]
mod fixture;
use mono_cut_lib::{model::*, render};
use serde_json::json;
use std::{fs, time::Instant};

#[test]
#[ignore = "Silent correctness control after the retained CRF22 gate failure"]
fn retained_marker_failure_lossless_and_quality_controls() {
    let dir = fixture::directory();
    let outdir = dir.join("reference-quality-control");
    fs::create_dir_all(&outdir).unwrap();
    let ctx = fixture::context(&outdir);
    let mut p = fixture::fixture(&ctx, &dir);
    p.in_point = None;
    p.out_point = None;
    let region = PreviewRegion {
        start_frame: 900,
        end_frame: 1050,
    };
    let retained_reference_dir = std::env::var_os("MONO_CUT_REGION_CONTROL_REFERENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.join("reference-crf22-retained"));
    let reference = retained_reference_dir.join("full-original.mkv");
    let failed_crf22_region = retained_reference_dir.join("region-original-900-1050.mp4");
    assert!(
        reference.is_file() && failed_crf22_region.is_file(),
        "Use the preserved CRF22 failure directory; do not substitute a new CRF20 result"
    );
    let rgb = |path: &std::path::Path, start: i64, end: i64| {
        fixture::decoded(
            &ctx,
            path,
            &[
                "-vf".into(),
                format!("trim=start_frame={start}:end_frame={end}"),
                "-an".into(),
                "-fps_mode".into(),
                "passthrough".into(),
                "-f".into(),
                "rawvideo".into(),
                "-pix_fmt".into(),
                "rgb24".into(),
            ],
        )
    };
    let expected = rgb(&reference, region.start_frame, region.end_frame);
    assert_eq!(expected.len(), 150 * 960 * 540 * 3);
    let expected_audio = fixture::audio(&ctx, &reference, 900 * 1600, 1050 * 1600);
    let mut report = json!({"purpose":"Retained CRF22 marker failure diagnosis, no production changes or relaxed tolerances", "source_hashes_before":fixture::snapshot_sources(), "test_executable_sha256":fixture::hash(&std::env::current_exe().unwrap()), "reference_path":reference,"reference_sha256":fixture::hash(&reference), "failed_crf22_region":failed_crf22_region,"failed_crf22_region_sha256":fixture::hash(&failed_crf22_region), "region":region, "controls":[]});
    for (codec, crf) in [("ffv1", 22), ("h264", 22), ("h264", 20), ("h264", 18)] {
        let mut settings = fixture::settings(codec);
        settings.crf = crf;
        let name = format!("control-{codec}-{crf}");
        let retained = codec == "h264" && crf == 22;
        let output = if retained {
            failed_crf22_region.clone()
        } else {
            outdir.join(format!(
                "{name}.{}",
                if codec == "ffv1" { "mkv" } else { "mp4" }
            ))
        };
        let plan =
            render::compile_region(&ctx, &p, &settings, false, &output, &name, &region).unwrap();
        fs::write(
            outdir.join(format!("{name}-filters.txt")),
            &plan.filter_graph,
        )
        .unwrap();
        let began = Instant::now();
        if !retained {
            fixture::run(&ctx, &plan.args);
        }
        let seconds = if retained {
            None
        } else {
            Some(began.elapsed().as_secs_f64())
        };
        let actual = rgb(&output, 0, 150);
        assert_eq!(actual.len(), expected.len());
        let mut markers = vec![];
        let mut frames = vec![];
        for index in 0..150 {
            let at = index * 960 * 540 * 3;
            let a = &expected[at..at + 960 * 540 * 3];
            let b = &actual[at..at + 960 * 540 * 3];
            let mut ar = Vec::with_capacity(75 * 260 * 3);
            let mut br = Vec::with_capacity(75 * 260 * 3);
            for y in 0..75 {
                ar.extend_from_slice(&a[y * 960 * 3..y * 960 * 3 + 260 * 3]);
                br.extend_from_slice(&b[y * 960 * 3..y * 960 * 3 + 260 * 3]);
            }
            let marker = fixture::video_metrics(&ar, &br);
            if index == 0 || index == 75 || index == 149 {
                frames.push(json!({"global_frame":900+index,"rgb":fixture::video_metrics(a,b),"marker_roi":marker}));
            }
            markers.push(marker["rms"].as_f64().unwrap());
        }
        let audio = fixture::audio(&ctx, &output, 0, 150 * 1600);
        assert_eq!(audio.len(), expected_audio.len());
        let pcm = fixture::audio_metrics(&expected_audio, &audio);
        let rgb_exact = actual == expected;
        let pcm_exact = audio == expected_audio;
        if codec == "ffv1" {
            assert!(rgb_exact, "Lossless all-frame RGB control must be exact");
            assert!(pcm_exact, "Lossless decoded PCM control must be exact");
        }
        let case = json!({"codec":codec,"crf":crf,"retained_failed_region":retained,"output_sha256":fixture::hash(&output),"output_bytes":fs::metadata(&output).unwrap().len(),"encode_seconds_correctness_only":seconds,"decoded_video_frames":150,"all_rgb_byte_exact":rgb_exact,"all_pcm_sample_exact":pcm_exact,"exact_stereo_sample_frames":240000,"audio":pcm,"sampled_frames":frames,"all_frame_marker_rms":markers,"maximum_all_frame_marker_rms":markers.iter().copied().fold(0.,f64::max),"same_predeclared_sample_marker_gate_passes":frames.iter().all(|f| f["marker_roi"]["rms"].as_f64().unwrap()<=6.),"working_region":plan.working_region,"input_ranges":plan.input_ranges,"args":plan.args,"args_executed":!retained});
        report["controls"].as_array_mut().unwrap().push(case);
        fixture::write(&outdir, "quality-control-results.json", &report);
        fixture::cleanup(&ctx, &name);
        eprintln!("Completed lossless/quality control {name}");
    }
    report["source_hashes_after"] = fixture::snapshot_sources();
    report["source_hashes_unchanged"] =
        json!(report["source_hashes_before"] == report["source_hashes_after"]);
    report["active_managed_children_after"] = json!(ctx.processes.active_count());
    fixture::write(&outdir, "quality-control-results.json", &report);
}
