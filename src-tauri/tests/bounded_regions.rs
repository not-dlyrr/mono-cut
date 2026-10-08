//! Bounded preview clocks are compared with the unchanged full-sequence model.
//! Encoded/decode-only synthetic media; no playback or device audio.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    preview, render,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;
fn ctx(d: &Path) -> MediaContext {
    let r = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    MediaContext {
        ffmpeg: std::env::var_os("MONO_CUT_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from("ffmpeg")),
        ffprobe: std::env::var_os("MONO_CUT_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from("ffprobe")),
        font: r.join("Inter.ttf"),
        cache_dir: d.join("cache"),
        processes: Default::default(),
    }
}
fn fixture(c: &MediaContext, path: &Path, fps: &str, offset: bool) {
    let shift = if offset { "setpts=PTS+5/TB" } else { "null" };
    let ashift = if offset {
        "asetpts=PTS+5.2/TB"
    } else {
        "anull"
    };
    media::run_ffmpeg(
        c,
        &[
            "-copyts".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("testsrc2=size=160x90:rate={fps}:duration=8"),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "aevalsrc=0.7*sin(2*PI*997*t)+if(lt(mod(t\\,0.7)\\,0.01)\\,0.3\\,0):s=44100:d=8".into(),
            "-filter_complex".into(),
            format!("[0:v]geq=lum='40+mod(N,30)*5':cb=128:cr=128,{shift}[v];[1:a]{ashift}[a]"),
            "-map".into(),
            "[v]".into(),
            "-map".into(),
            "[a]".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            "-avoid_negative_ts".into(),
            "disabled".into(),
            path.to_string_lossy().into(),
        ],
    )
    .unwrap();
}
fn project(c: &MediaContext, path: &Path, fps: Rational) -> Project {
    let mut p = Project::new("Regions".into(), 160, 90, fps);
    p.media.push(media::probe(c, path).unwrap());
    let media_id = p.media[0].id.clone();
    let track_id = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id,
            track_id,
            start: 0,
            source_in: Some(Rational::new(7, 13)),
            duration: Some(180),
        },
    )
    .unwrap();
    p
}
fn settings(p: &Project) -> ExportSettings {
    ExportSettings {
        width: 160,
        height: 90,
        fps: p.fps,
        codec: "ffv1".into(),
        crf: 22,
        audio_bitrate: 160,
        sample_rate: 48000,
    }
}
fn decode(c: &MediaContext, path: &Path, video: bool) -> Vec<u8> {
    let a = if video {
        vec!["-map", "0:v:0", "-pix_fmt", "rgb24", "-f", "rawvideo"]
    } else {
        vec!["-map", "0:a:0", "-ac", "2", "-ar", "48000", "-f", "f32le"]
    };
    let o = media::command(&c.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(a)
        .arg("pipe:1")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}
fn samples(frame: i64, fps: Rational) -> usize {
    let n = frame as i128 * fps.den as i128 * 48000;
    ((n + fps.num as i128 - 1) / fps.num as i128) as usize
}
fn compare(
    c: &MediaContext,
    p: &Project,
    r: &PreviewRegion,
    fullv: &[u8],
    fulla: &[u8],
    tag: &str,
) {
    let path = c.cache_dir.join(format!("{tag}.mkv"));
    let plan = render::compile_region(c, p, &settings(p), false, &path, tag, r).unwrap();
    assert!(
        plan.working_region.end_frame - plan.working_region.start_frame
            <= r.end_frame - r.start_frame + 32
    );
    assert!(!plan.input_ranges.is_empty());
    for range in &plan.input_ranges {
        assert!(
            range.duration.value() < 3.5,
            "unbounded source span {range:?}"
        );
    }
    media::run_ffmpeg(c, &plan.args).unwrap();
    let v = decode(c, &path, true);
    let a = decode(c, &path, false);
    let expected =
        &fullv[r.start_frame as usize * 160 * 90 * 3..r.end_frame as usize * 160 * 90 * 3];
    assert_eq!(v.len(), expected.len());
    let bad = v
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (a, b))| a != b);
    assert!(bad.is_none(), "{tag} first wrong video byte {bad:?}");
    let first = samples(r.start_frame, p.fps) * 8;
    let last = samples(r.end_frame, p.fps) * 8;
    assert_eq!(a.len(), last - first, "audio sample coverage {tag}");
    let mut peak = 0f32;
    let mut sum = 0f64;
    for (x, y) in a.chunks_exact(4).zip(fulla[first..last].chunks_exact(4)) {
        let d =
            f32::from_le_bytes(x.try_into().unwrap()) - f32::from_le_bytes(y.try_into().unwrap());
        peak = peak.max(d.abs());
        sum += (d * d) as f64;
    }
    let rms = (sum / (a.len() / 4) as f64).sqrt();
    assert!(
        peak < 0.0002,
        "{tag} audio phase drift peak={peak} rms={rms}"
    );
}
#[test]
fn bounded_regions_preserve_mixed_frames_audio_speed_offsets_and_envelopes() {
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    for (ix, (fps, offset, speed)) in [
        (Rational::new(30, 1), false, Rational::new(1, 1)),
        (Rational::new(30000, 1001), true, Rational::new(7, 6)),
    ]
    .into_iter()
    .enumerate()
    {
        let source = d.path().join(format!("source{ix}.mkv"));
        fixture(&c, &source, "30000/1001", offset);
        let mut p = project(&c, &source, fps);
        let id = p.clips[0].id.clone();
        // Fixed-duration legacy renderer fixture; transactional Speed has separate tests.
        p.clips[0].speed=speed; p.clips[0].render_offset=None;
        edit::apply(&mut p,EditCommand::UpdateClip{id:id.clone(),patch:json!({"fade_in":42,"fade_out":48,"keyframes":[{"property":"volume","frame":0,"value":0.2},{"property":"volume","frame":100,"value":1.1},{"property":"opacity","frame":0,"value":0.4},{"property":"opacity","frame":145,"value":1.0},{"property":"x","frame":0,"value":-15.0},{"property":"x","frame":170,"value":16.0}]})}).unwrap();
        edit::apply(
            &mut p,
            EditCommand::Split {
                ids: vec![id],
                frame: 71,
            },
        )
        .unwrap();
        let full = c.cache_dir.join(format!("full{ix}.mkv"));
        let plan =
            render::compile(&c, &p, &settings(&p), false, &full, &format!("full{ix}")).unwrap();
        media::run_ffmpeg(&c, &plan.args).unwrap();
        let v = decode(&c, &full, true);
        let a = decode(&c, &full, false);
        for (n, r) in [
            PreviewRegion {
                start_frame: 0,
                end_frame: 30,
            },
            PreviewRegion {
                start_frame: 61,
                end_frame: 91,
            },
            PreviewRegion {
                start_frame: 91,
                end_frame: 121,
            },
            PreviewRegion {
                start_frame: 179,
                end_frame: 180,
            },
        ]
        .iter()
        .enumerate()
        {
            compare(&c, &p, r, &v, &a, &format!("case{ix}-{n}"));
        }
    }
}
#[test]
fn bounded_audio_mix_matches_full_float_bits_with_inactive_automation() {
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    media::ensure_cache(&c).unwrap();
    let mut p = Project::new("Mixed gain precision".into(), 160, 90, Rational::new(30, 1));
    // Keep the Stage4B source signals, quantization, and levels. Audio-only
    // files make this regression inexpensive without changing its samples.
    for (frequency, phase) in [(440, "0"), (660, "0.25")] {
        let path = d.path().join(format!("source-{frequency}.wav"));
        media::run_ffmpeg(
            &c,
            &[
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!("aevalsrc=if(lt(mod(t+{phase}\\,1)\\,0.025)\\,0.22*sin(2*PI*{frequency}*t)\\,0):s=48000:d=16"),
                "-c:a".into(),
                "pcm_s16le".into(),
                "-ac".into(),
                "2".into(),
                path.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        p.media.push(media::probe(&c, &path).unwrap());
    }
    let a1 = p.tracks[1].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddTrack {
            kind: "audio".into(),
            name: "Audio 2".into(),
        },
    )
    .unwrap();
    let a2 = p.tracks.last().unwrap().id.clone();
    for (track_id, media_index, start, name) in [
        (&a1, 0, 0, "early-automation"),
        (&a1, 1, 1080, "late-static"),
        (&a2, 0, 1080, "late-automation"),
    ] {
        let media_id = p.media[media_index].id.clone();
        edit::apply(
            &mut p,
            EditCommand::AddClip {
                media_id,
                track_id: track_id.clone(),
                start,
                source_in: Some(if start == 0 { Rational::zero() } else { Rational::new(3, 4) }),
                duration: Some(360),
            },
        )
        .unwrap();
        let clip = p.clips.last_mut().unwrap();
        clip.id = name.into();
        clip.volume = 0.55;
        if start == 0 {
            clip.fade_in = 15;
        } else if track_id == &a2 {
            clip.volume = 0.35;
            clip.fade_in = 15;
            clip.fade_out = 15;
            clip.keyframes = vec![
                Keyframe { property: "volume".into(), frame: 0, value: 0.15 },
                Keyframe { property: "volume".into(), frame: 180, value: 0.35 },
                Keyframe { property: "volume".into(), frame: 359, value: 0.2 },
            ];
        }
    }
    let region = PreviewRegion { start_frame: 1100, end_frame: 1250 };
    let full = c.cache_dir.join("mix-full-float.mkv");
    let bounded = c.cache_dir.join("mix-bounded-float.mkv");
    for (path, is_region, tag) in [
        (&full, false, "mix-full-float"),
        (&bounded, true, "mix-bounded-float"),
    ] {
        let mut plan = if is_region {
            render::compile_region(&c, &p, &settings(&p), false, path, tag, &region)
        } else {
            render::compile(&c, &p, &settings(&p), false, path, tag)
        }
        .unwrap();
        // Expose unquantized samples; FLAC's integer conversion can hide an
        // order-dependent one-ULP difference before the common mix limiter.
        let codec = plan.args.iter().position(|arg| arg == "-c:a").unwrap();
        plan.args[codec + 1] = "pcm_f32le".into();
        media::run_ffmpeg(&c, &plan.args).unwrap();
    }
    let full_pcm = decode(&c, &full, false);
    let bounded_pcm = decode(&c, &bounded, false);
    let first = samples(region.start_frame, p.fps) * 8;
    let last = samples(region.end_frame, p.fps) * 8;
    assert_eq!(bounded_pcm.len(), 240_000 * 8, "Exact stereo sample coverage");
    let expected = &full_pcm[first..last];
    let different = bounded_pcm.chunks_exact(4).zip(expected.chunks_exact(4))
        .enumerate().filter(|(_, (actual, full))| actual != full).map(|(i, _)| i)
        .collect::<Vec<_>>();
    eprintln!("Mixed static/automated gain {}: {} interleaved samples, {} different float bit patterns; first={:?}", preview::RECIPE, bounded_pcm.len() / 4, different.len(), different.first());
    assert!(different.is_empty(), "Inactive early automation changed bounded mixer rounding: {} differing samples; first={:?}", different.len(), different.first());
    assert_eq!(c.processes.active_count(), 0);
}

#[test]
fn inherited_negative_fades_and_returned_unity_keep_exact_sample_clocks() {
    use sha2::{Digest,Sha256};
    let d=TempDir::new().unwrap();let c=ctx(d.path());media::ensure_cache(&c).unwrap();
    let source=d.path().join("offset-inherited-envelope.mkv");
    // Same 48 kHz PCM quantization, pulse level and source offsets as the
    // independently diagnosed counterexample; only video dimensions are smaller.
    media::run_ffmpeg(&c,&[
        "-copyts".into(),"-f".into(),"lavfi".into(),"-i".into(),"testsrc2=size=160x90:rate=30:duration=16".into(),
        "-f".into(),"lavfi".into(),"-i".into(),
        "aevalsrc=if(lt(mod(t\\,1)\\,0.025)\\,0.22*sin(2*PI*440*t)\\,0):s=48000:d=16".into(),
        "-filter_complex".into(),"[0:v]setpts=PTS+3/TB[v];[1:a]asetpts=PTS+3.2/TB[a]".into(),
        "-map".into(),"[v]".into(),"-map".into(),"[a]".into(),"-c:v".into(),"ffv1".into(),
        "-c:a".into(),"pcm_s16le".into(),"-ac".into(),"2".into(),"-fps_mode".into(),"passthrough".into(),
        "-avoid_negative_ts".into(),"disabled".into(),source.to_string_lossy().into(),
    ]).unwrap();
    let mut legacy=Project::new("Inherited sample clocks".into(),160,90,Rational::new(30,1));
    legacy.media.push(media::probe(&c,&source).unwrap());
    for (track,volume) in [(0,0.0),(1,0.55)] {
        let media_id=legacy.media[0].id.clone();let track_id=legacy.tracks[track].id.clone();
        edit::apply(&mut legacy,EditCommand::AddClip{media_id,track_id,start:0,source_in:None,duration:Some(480)}).unwrap();
        let clip=legacy.clips.last_mut().unwrap();clip.linked_id=Some("av-envelope".into());clip.volume=volume;
        clip.fade_in=90;clip.fade_out=120;
        if track==1 {clip.keyframes=vec![Keyframe{property:"volume".into(),frame:0,value:0.25},
            Keyframe{property:"volume".into(),frame:210,value:0.75},Keyframe{property:"volume".into(),frame:479,value:0.5}];}
    }
    let id=legacy.clips[0].id.clone();
    edit::apply(&mut legacy,EditCommand::Trim{id:id.clone(),edge:"in".into(),frame:31}).unwrap();
    edit::apply(&mut legacy,EditCommand::Trim{id:id.clone(),edge:"out".into(),frame:451}).unwrap();
    assert!(legacy.clips.iter().all(|clip|clip.start==31 && clip.duration==420 && clip.fade_in_start()==-31 && clip.retime.is_none()));
    let mut returned=legacy.clone();
    edit::apply(&mut returned,EditCommand::RetimeClip{id:id.clone(),speed:Rational::new(7,6)}).unwrap();
    edit::apply(&mut returned,EditCommand::RetimeClip{id,speed:Rational::one()}).unwrap();
    assert!(returned.clips.iter().all(|clip|clip.start==31 && clip.duration==420 && clip.retime.as_ref().unwrap().envelope.fade_in_start==Rational::new(-31,30)));
    let region=PreviewRegion{start_frame:171,end_frame:321};
    let capture=std::env::var_os("MONO_CUT_CLOCK_CAPTURE_DIR").map(PathBuf::from);
    if let Some(dir)=&capture {fs::create_dir_all(dir).unwrap();fs::copy(&source,dir.join("source.mkv")).unwrap();}
    let mut evidence=vec![];let mut reference:Option<(Vec<u8>,Vec<u8>)>=None;
    for (label,p) in [("legacy",&legacy),("returned-unity",&returned)] {
        if let Some(dir)=&capture {fs::write(dir.join(format!("{label}.json")),serde_json::to_vec_pretty(p).unwrap()).unwrap();}
        let modes=if capture.is_some(){vec![false,true]}else{vec![false]};
        for old_divided_clock in modes {
            let tag=format!("{label}-{}",if old_divided_clock{"v7-control"}else{"current"});
            let mut outputs=vec![];
            for bounded in [false,true] {
                let name=format!("{tag}-{}",if bounded{"bounded"}else{"full"});
                let path=c.cache_dir.join(format!("{name}.mkv"));
                let mut plan=if bounded {render::compile_region(&c,p,&settings(p),false,&path,&name,&region)}
                    else {render::compile(&c,p,&settings(p),false,&path,&name)}.unwrap();
                let codec=plan.args.iter().position(|arg|arg=="-c:a").unwrap();plan.args[codec+1]="pcm_f32le".into();
                if old_divided_clock {
                    // Private causal capture replays exactly the old post-trim
                    // clock. Production paths and the public gate remain v8.
                    assert_eq!(plan.filter_graph.matches(",asetpts=N,").count(),1);
                    plan.filter_graph=plan.filter_graph.replace(",asetpts=N,",",asetpts=N/SR/TB,");
                    let script=plan.args.iter().position(|arg|arg=="-filter_complex_script").unwrap();
                    fs::write(&plan.args[script+1],&plan.filter_graph).unwrap();
                }
                media::run_ffmpeg(&c,&plan.args).unwrap();
                let audio=decode(&c,&path,false);let video=decode(&c,&path,true);
                if let Some(dir)=&capture {
                    fs::copy(&path,dir.join(format!("{name}.mkv"))).unwrap();
                    fs::write(dir.join(format!("{name}-filter.txt")),&plan.filter_graph).unwrap();
                    fs::write(dir.join(format!("{name}-args.json")),serde_json::to_vec_pretty(&plan.args).unwrap()).unwrap();
                }
                outputs.push((audio,video));
            }
            let first=samples(region.start_frame,p.fps)*8;let last=samples(region.end_frame,p.fps)*8;
            let full_audio=&outputs[0].0[first..last];let bounded_audio=&outputs[1].0;
            assert_eq!(bounded_audio.len(),240_000*8,"Exact 240,000 stereo sample coverage");
            let different=full_audio.chunks_exact(4).zip(bounded_audio.chunks_exact(4)).filter(|(a,b)|a!=b).count();
            let frame_bytes=160*90*3;let full_video=&outputs[0].1[171*frame_bytes..321*frame_bytes];
            assert_eq!(full_video,outputs[1].1,"All 150 lossless RGB frames match");
            eprintln!("Inherited negative fade {tag} {}: {different}/480000 different float bit patterns",preview::RECIPE);
            if old_divided_clock {assert!(different>0,"Private old-clock control must reproduce the diagnosed counterexample");}
            else {
                assert_eq!(different,0,"Inherited envelope phase changed bounded float samples");
                if let Some((audio,video))=&reference {assert_eq!(audio,&outputs[0].0,"Returning to unity changed full audio");assert_eq!(video,&outputs[0].1,"Returning to unity changed full RGB");}
                else {reference=Some((outputs[0].0.clone(),outputs[0].1.clone()));}
            }
            evidence.push(json!({"label":tag,"old_divided_clock":old_divided_clock,"different_values":different,
                "interleaved_float_values":480000,"full_slice_sha256":format!("{:x}",Sha256::digest(full_audio)),
                "bounded_slice_sha256":format!("{:x}",Sha256::digest(bounded_audio))}));
        }
    }
    assert_eq!(c.processes.active_count(),0);
    if let Some(dir)=capture {fs::write(dir.join("clock-regression-results.json"),serde_json::to_vec_pretty(&json!({
        "recipe":preview::RECIPE,"source_sha256":format!("{:x}",Sha256::digest(fs::read(source).unwrap())),
        "controls":evidence,"native_ui_or_audio_launched":false,"active_managed_children":0})).unwrap()).unwrap();}
}
#[test]
fn region_identity_manifest_bounds_and_superseding_intent() {
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    let source = d.path().join("source.mkv");
    fixture(&c, &source, "30", false);
    let p = project(&c, &source, Rational::new(30, 1));
    let a = PreviewRegion {
        start_frame: 30,
        end_frame: 60,
    };
    let b = PreviewRegion {
        start_frame: 60,
        end_frame: 90,
    };
    let manager = JobManager::new(c.clone(), Arc::new(|_| {}));
    let key = manager.preview_region_identity(&p, 120, false, &a).unwrap();
    assert_ne!(
        key,
        manager.preview_region_identity(&p, 120, false, &b).unwrap()
    );
    let session = manager.begin_preview_session();
    let intent = manager.set_preview_intent(session, 1, &key).unwrap();
    let j = manager
        .preview_region_for_intent(p.clone(), 120, false, a.clone(), Some(&key), &intent)
        .unwrap();
    let started = Instant::now();
    let job = loop {
        let j = manager.list().into_iter().find(|x| x.id == j.id).unwrap();
        if j.status != "running" {
            break j;
        }
        assert!(started.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(job.status, "complete", "{job:?}");
    assert_eq!(job.preview_region, Some(a.clone()));
    assert!(preview::cached_region(&c, &p, 120, &key, Some(&a)).is_some());
    assert!(preview::cached_region(&c, &p, 120, &key, Some(&b)).is_none());
    let path = c.cache_dir.join(format!("preview-{key}.json"));
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["origin"] = json!({"num":0,"den":1});
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(preview::cached_region(&c, &p, 120, &key, Some(&a)).is_none());
    let k = manager.preview_region_identity(&p, 120, false, &b).unwrap();
    manager.set_preview_intent(session, 2, &k).unwrap();
    assert!(manager
        .preview_region_for_intent(p, 120, false, a, Some(&key), &intent)
        .unwrap_err()
        .contains("PREVIEW_IDENTITY_CHANGED"));
}

#[test]
fn long_gop_regions_keep_real_first_frames_and_stop_at_filter_eof() {
    let d = TempDir::new().unwrap();
    for (ix, offset, fps, speed) in [
        (0, false, Rational::new(30, 1), Rational::new(1, 1)),
        (1, true, Rational::new(30000, 1001), Rational::new(7, 6)),
    ] {
        let c = ctx(d.path());
        let source = d.path().join(format!("long-gop-{ix}.mkv"));
        let shift = if offset { "setpts=PTS+5/TB" } else { "null" };
        let ashift = if offset {
            "asetpts=PTS+5.2/TB"
        } else {
            "anull"
        };
        media::run_ffmpeg(&c, &[
            "-copyts".into(), "-f".into(), "lavfi".into(), "-i".into(),
            "testsrc2=size=160x90:rate=30000/1001:duration=20".into(),
            "-f".into(), "lavfi".into(), "-i".into(),
            "aevalsrc=0.5*sin(2*PI*997*t):s=44100:d=20".into(),
            "-filter_complex".into(),
            format!("[0:v]geq=lum='80+mod(N,140)':cb='110+mod(N,13)':cr='120+mod(N,17)',{shift}[v];[1:a]{ashift}[a]"),
            "-map".into(), "[v]".into(), "-map".into(), "[a]".into(),
            "-c:v".into(), "libx264".into(), "-crf".into(), "9".into(),
            "-g".into(), "250".into(), "-keyint_min".into(), "250".into(),
            "-sc_threshold".into(), "0".into(), "-bf".into(), "3".into(),
            "-c:a".into(), "pcm_f32le".into(), "-fps_mode".into(), "passthrough".into(),
            "-avoid_negative_ts".into(), "disabled".into(), source.to_string_lossy().into(),
        ]).unwrap();
        let mut p = project(&c, &source, fps);
        let id = p.clips[0].id.clone();
        // Preserve this legacy renderer fixture's selected source interval.
        p.clips[0].speed=speed; p.clips[0].render_offset=None;
        edit::apply(
            &mut p,
            EditCommand::UpdateClip {
                id: id.clone(),
                patch: json!({
                    "duration":450,"fade_in":42,"fade_out":48,
                    "keyframes":[{"property":"opacity","frame":0,"value":0.4},
                                 {"property":"opacity","frame":400,"value":1.0},
                                 {"property":"x","frame":0,"value":-12.0},
                                 {"property":"x","frame":420,"value":12.0}]
                }),
            },
        )
        .unwrap();
        edit::apply(
            &mut p,
            EditCommand::Split {
                ids: vec![id],
                frame: 71,
            },
        )
        .unwrap();
        let mut s = preview::settings(&p, 120).unwrap();
        s.codec = "ffv1".into();
        let full = c.cache_dir.join(format!("long-full-{ix}.mkv"));
        let full_plan =
            render::compile(&c, &p, &s, false, &full, &format!("long-full-{ix}")).unwrap();
        media::run_ffmpeg(&c, &full_plan.args).unwrap();
        let fullv = decode(&c, &full, true);
        let fulla = decode(&c, &full, false);
        let frame_bytes = s.width as usize * s.height as usize * 3;
        let manager = JobManager::new(c.clone(), Arc::new(|_| {}));
        for (tag, r) in [
            (
                "still",
                PreviewRegion {
                    start_frame: 180,
                    end_frame: 181,
                },
            ),
            (
                "continuous",
                PreviewRegion {
                    start_frame: 180,
                    end_frame: 330,
                },
            ),
        ] {
            let out = c.cache_dir.join(format!("long-{ix}-{tag}.mkv"));
            let plan =
                render::compile_region(&c, &p, &s, false, &out, &format!("long-{ix}-{tag}"), &r)
                    .unwrap();
            let first_input = plan.args.iter().position(|x| x == "-i").unwrap();
            assert!(plan.args[..first_input]
                .iter()
                .any(|x| x == "-noaccurate_seek"));
            assert!(
                !plan.args[..first_input].iter().any(|x| x == "-t"),
                "Video duration must not consume long-GOP preroll"
            );
            assert_eq!(plan.output_frames, r.end_frame - r.start_frame);
            assert_eq!(
                plan.working_region.end_frame - plan.working_region.start_frame,
                r.end_frame - r.start_frame + 30
            );
            let stream = p.media[0].timing.as_ref().unwrap().video_stream.unwrap();
            let needle = format!("[0:{stream}]");
            let graph = plan
                .filter_graph
                .replacen(&needle, &format!("{needle}showinfo,"), 1);
            assert_ne!(graph, plan.filter_graph, "No input stream instrumented");
            let mut args = plan.args.clone();
            let graph_path = &args[args
                .iter()
                .position(|x| x == "-filter_complex_script")
                .unwrap()
                + 1];
            fs::write(graph_path, graph).unwrap();
            let log_level = args.iter().position(|x| x == "-loglevel").unwrap() + 1;
            args[log_level] = "info".into();
            let result = media::command(&c.ffmpeg).args(&args).output().unwrap();
            let log = String::from_utf8_lossy(&result.stderr);
            assert!(result.status.success(), "{log}");
            let timestamps: Vec<f64> = log
                .lines()
                .filter(|line| line.contains("showinfo") && line.contains(" n:"))
                .filter_map(|line| {
                    line.split_once("pts_time:")
                        .and_then(|(_, tail)| tail.split_whitespace().next())
                        .and_then(|x| x.parse().ok())
                })
                .collect();
            assert!(
                !timestamps.is_empty(),
                "No upstream video decode diagnostics: {log}"
            );
            let source_origin = p.media[0].timing.as_ref().unwrap().origin.value();
            let range = &plan.input_ranges[0];
            assert!(
                timestamps[0] + 0.5 < source_origin + range.seek.value(),
                "Fixture did not exercise distant keyframe preroll"
            );
            let source_end = source_origin + range.seek.value() + range.duration.value();
            let last = *timestamps.last().unwrap();
            assert!(last<=source_end+2.0/(30000.0/1001.0), "Filter EOF decoded past the contributing source end: last={last}, end={source_end}");
            assert!(
                last < source_origin + 19.0,
                "Region decoded the full long source"
            );
            let v = decode(&c, &out, true);
            let expected =
                &fullv[r.start_frame as usize * frame_bytes..r.end_frame as usize * frame_bytes];
            assert_eq!(
                v, expected,
                "Long-GOP {ix}/{tag} RGB differs from the shared full reference"
            );
            let a = decode(&c, &out, false);
            let first = samples(r.start_frame, p.fps) * 8;
            let end = samples(r.end_frame, p.fps) * 8;
            assert_eq!(a.len(), end - first, "Long-GOP exact sample coverage");
            let peak = a
                .chunks_exact(4)
                .zip(fulla[first..end].chunks_exact(4))
                .map(|(x, y)| {
                    (f32::from_le_bytes(x.try_into().unwrap())
                        - f32::from_le_bytes(y.try_into().unwrap()))
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(peak < 0.0002, "Long-GOP audio clock changed: peak={peak}");
            let j = manager
                .preview_region(p.clone(), 120, false, r.clone())
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            let done = loop {
                let job = manager.list().into_iter().find(|x| x.id == j.id).unwrap();
                if job.status != "running" {
                    break job;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(done.status, "complete", "{done:?}");
            let native = decode(&c, Path::new(done.path.as_ref().unwrap()), true);
            assert_eq!(native.len(), expected.len());
            for (frame, reference) in native
                .chunks_exact(frame_bytes)
                .zip(expected.chunks_exact(frame_bytes))
            {
                assert!(
                    reference.iter().any(|x| *x > 20),
                    "Long-GOP full-reference fixture frame is too dark for the nonblack assertion"
                );
                let rms = (frame
                    .iter()
                    .zip(reference)
                    .map(|(x, y)| (*x as f64 - *y as f64).powi(2))
                    .sum::<f64>()
                    / frame_bytes as f64)
                    .sqrt();
                assert!(
                    rms <= 6.0,
                    "Actual native long-GOP {ix}/{tag} frame RMS {rms} exceeds6"
                );
                assert!(
                    frame.iter().any(|x| *x > 20),
                    "Actual native long-GOP preview was black"
                );
            }
            eprintln!("VERIFIED long-GOP {ix}/{tag}: {} input frames from{} to{} (source end{}), {} lossless RGB frames byte-exact, exact PCM phase, native H264 every-frame RMS<=6 and nonblack.", timestamps.len(), timestamps[0],last,source_end,r.end_frame-r.start_frame);
        }
        manager.shutdown();
        assert_eq!(c.processes.active_count(), 0);
    }
}

#[test]
fn precise_audio_cache_reuses_invalidates_bounds_pins_and_cancels_cleanly() {
    use mono_cut_lib::audio_clock;
    use std::sync::atomic::{AtomicBool, Ordering};
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    let source = d.path().join("audio-source.mkv");
    fixture(&c, &source, "30", true);
    let m = media::probe(&c, &source).unwrap();
    let first = audio_clock::ensure(&c, &m, false, 48000, None).unwrap();
    let count = c.processes.total_spawn_count();
    let same = audio_clock::ensure(&c, &m, false, 48000, None).unwrap();
    assert_eq!(first.path, same.path);
    assert_eq!(
        count,
        c.processes.total_spawn_count(),
        "Warm PCM lookup spawned a process"
    );
    assert!(c.processes.is_protected(&first.path));
    assert!(c.processes.is_protected(first.path.with_extension("json")));
    let changed_rate = audio_clock::ensure(&c, &m, false, 44100, None).unwrap();
    assert_ne!(first.path, changed_rate.path);
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(2))
        .unwrap();
    let changed_source = audio_clock::ensure(&c, &m, false, 48000, None).unwrap();
    assert_ne!(first.path, changed_source.path);
    let mut huge = m.clone();
    huge.duration = Rational::new(10_000, 1);
    let count = c.processes.total_spawn_count();
    assert!(audio_clock::ensure(&c, &huge, false, 48000, None)
        .err()
        .unwrap()
        .contains("256 MiB"));
    assert_eq!(count, c.processes.total_spawn_count());
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_ctx = c.clone();
    let worker_m = m.clone();
    let flag = cancel.clone();
    let worker = std::thread::spawn(move || {
        audio_clock::ensure(&worker_ctx, &worker_m, false, 96000, Some(&flag))
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while c.processes.active_count() == 0 {
        assert!(Instant::now() < deadline, "Audio preparation did not start");
        std::thread::yield_now();
    }
    cancel.store(true, Ordering::Release);
    assert!(worker.join().unwrap().err().unwrap().contains("cancelled"));
    assert_eq!(c.processes.active_count(), 0);
    assert!(
        !fs::read_dir(&c.cache_dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("part")),
        "Cancelled preparation left a partial"
    );
    let retried = audio_clock::ensure(&c, &m, false, 96000, None).unwrap();
    assert!(retried.path.is_file());
    drop(retried);
    drop(changed_source);
    drop(changed_rate);
    drop(same);
    drop(first);
    for ix in 0..audio_clock::MAX_ENTRIES + 3 {
        let path = c
            .cache_dir
            .join(format!("pressure-{ix}-audio-clock-v1.wav"));
        fs::File::create(&path)
            .unwrap()
            .set_len(9 * 1024 * 1024)
            .unwrap();
        fs::write(path.with_extension("json"), b"{}").unwrap();
    }
    let pinned = c
        .processes
        .pin_file(c.cache_dir.join("pressure-0-audio-clock-v1.wav"))
        .unwrap();
    audio_clock::prune(&c);
    assert!(pinned.path().is_file());
    let outputs: Vec<_> = fs::read_dir(&c.cache_dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with("-audio-clock-v1.wav")
        })
        .collect();
    assert!(outputs.len() <= audio_clock::MAX_ENTRIES);
    assert!(
        outputs
            .iter()
            .map(|e| e.metadata().unwrap().len())
            .sum::<u64>()
            <= audio_clock::MAX_BYTES
    );
}

#[test]
fn oversized_audio_uses_exact_prefix_without_allocating_full_pcm() {
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    let source = d.path().join("fallback-source.mkv");
    fixture(&c, &source, "30000/1001", true);
    let mut p = project(&c, &source, Rational::new(30000, 1001));
    p.media[0].duration = Rational::new(10_000, 1);
    let id = p.clips[0].id.clone();
    // Explicit legacy fixed-duration fixture, including its existing rate.
    p.clips[0].speed=Rational::new(7,6); p.clips[0].render_offset=None;
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id,
            patch: json!({"fade_in":50,"fade_out":50}),
        },
    )
    .unwrap();
    let full = c.cache_dir.join("fallback-full.mkv");
    let plan = render::compile(&c, &p, &settings(&p), false, &full, "fallback-full").unwrap();
    media::run_ffmpeg(&c, &plan.args).unwrap();
    let region = PreviewRegion {
        start_frame: 61,
        end_frame: 91,
    };
    let out = c.cache_dir.join("fallback-region.mkv");
    let plan = render::compile_region(&c, &p, &settings(&p), false, &out, "fallback-plan", &region)
        .unwrap();
    assert_eq!(plan.audio_prefix_fallbacks.len(), 1);
    assert_eq!(plan.audio_prefix_fallbacks[0].seek, Rational::zero());
    assert!(plan.asset_pins.is_empty());
    assert!(plan.input_ranges[0].seek.value() > 0.);
    assert!(
        plan.audio_prefix_fallbacks[0].duration.value() > plan.input_ranges[0].duration.value()
    );
    compare(
        &c,
        &p,
        &region,
        &decode(&c, &full, true),
        &decode(&c, &full, false),
        "fallback-compare",
    );
    assert!(!fs::read_dir(&c.cache_dir).unwrap().flatten().any(|e| e
        .file_name()
        .to_string_lossy()
        .ends_with("-audio-clock-v1.wav")));
}

/// Count actual input decoder work rather than inferring it from short output.
/// A timeout also prevents a repeated 10,000-second still prefix from hanging
/// this regression test. Readers drain pipes while the managed child runs.
fn render_counted_still(c: &MediaContext, args: &[String]) -> usize {
    use std::{io::Read, process::Stdio};
    let mut args = args.to_vec();
    let level = args.iter().position(|a| a == "-loglevel").unwrap();
    args[level + 1] = "verbose".into();
    let mut command = media::command(&c.ffmpeg);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = c.processes.spawn(&mut command).unwrap();
    let mut stdout = child.take_stdout().unwrap();
    let mut stderr = child.take_stderr().unwrap();
    let output = std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
    let errors = std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut block = [0u8; 4096];
        loop {
            let n = stderr.read(&mut block).unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            timed_out = true;
            child.kill().unwrap();
            break child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    output.join().unwrap().unwrap();
    let log = String::from_utf8(errors.join().unwrap()).unwrap();
    drop(child);
    assert!(
        !timed_out,
        "Still source prefix work exceeded five seconds: {log}"
    );
    assert!(status.success(), "{log}");
    let counts: Vec<usize> = log
        .lines()
        .filter_map(|line| {
            line.split_once("frames decoded").and_then(|(prefix, _)| {
                prefix
                    .split_whitespace()
                    .last()
                    .and_then(|word| word.parse().ok())
            })
        })
        .collect();
    assert!(
        !counts.is_empty(),
        "FFmpeg did not report input decoding: {log}"
    );
    counts.iter().sum()
}

#[test]
fn still_regions_skip_large_source_offsets_and_preserve_split_envelopes() {
    let d = TempDir::new().unwrap();
    let c = ctx(d.path());
    let source = d.path().join("offset-still.png");
    media::run_ffmpeg(
        &c,
        &[
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=160x90:rate=30".into(),
            "-frames:v".into(),
            "1".into(),
            source.to_string_lossy().into(),
        ],
    )
    .unwrap();
    let mut reference = project(&c, &source, Rational::new(30000, 1001));
    assert_eq!(reference.media[0].kind, "image");
    let id = reference.clips[0].id.clone();
    // Historical still-renderer fixture; new Speed operations reject still images.
    reference.clips[0].speed=Rational::new(7,6); reference.clips[0].render_offset=None;
    edit::apply(
        &mut reference,
        EditCommand::UpdateClip {
            id: id.clone(),
            patch: json!({"fade_in":60,"fade_out":75,"keyframes":[
                {"property":"opacity","frame":0,"value":0.4},
                {"property":"opacity","frame":145,"value":1.0},
                {"property":"x","frame":0,"value":-15.0},
                {"property":"x","frame":170,"value":16.0},
                {"property":"scale","frame":0,"value":0.8},
                {"property":"scale","frame":170,"value":1.1}
            ]}),
        },
    )
    .unwrap();
    let mut actual = reference.clone();
    actual.clips[0].source_in = actual.clips[0]
        .source_in
        .checked_add(Rational::new(10_000, 1))
        .unwrap();
    for p in [&mut reference, &mut actual] {
        edit::apply(
            p,
            EditCommand::Trim {
                id: id.clone(),
                edge: "in".into(),
                frame: 12,
            },
        )
        .unwrap();
        edit::apply(
            p,
            EditCommand::Trim {
                id: id.clone(),
                edge: "out".into(),
                frame: 176,
            },
        )
        .unwrap();
    }
    edit::apply(
        &mut actual,
        EditCommand::Split {
            ids: vec![id],
            frame: 71,
        },
    )
    .unwrap();
    let right = actual
        .clips
        .iter()
        .find(|clip| clip.start == 71)
        .unwrap()
        .id
        .clone();
    edit::apply(
        &mut actual,
        EditCommand::Split {
            ids: vec![right],
            frame: 111,
        },
    )
    .unwrap();
    let full = c.cache_dir.join("still-reference.mkv");
    let plan = render::compile(
        &c,
        &reference,
        &settings(&reference),
        false,
        &full,
        "still-reference",
    )
    .unwrap();
    media::run_ffmpeg(&c, &plan.args).unwrap();
    let v = decode(&c, &full, true);
    let a = decode(&c, &full, false);
    let shifted_full = c.cache_dir.join("still-large-offset-full.mkv");
    let plan = render::compile(
        &c,
        &actual,
        &settings(&actual),
        false,
        &shifted_full,
        "still-large-offset-full",
    )
    .unwrap();
    let decoded = render_counted_still(&c, &plan.args);
    assert!(
        decoded <= actual.length() as usize + actual.clips.len() * 16,
        "Unbounded full still input: {decoded}"
    );
    assert_eq!(
        decode(&c, &shifted_full, true),
        v,
        "Large image source offset or repeated split changed full RGB"
    );
    assert_eq!(
        decode(&c, &shifted_full, false),
        a,
        "Still full reference changed silence coverage"
    );
    eprintln!("VERIFIED large-offset full still: {decoded} decoded input frames, 176 output frames, source offset >10,000 seconds.");
    for (n, region) in [
        PreviewRegion {
            start_frame: 0,
            end_frame: 30,
        },
        PreviewRegion {
            start_frame: 61,
            end_frame: 91,
        },
        PreviewRegion {
            start_frame: 91,
            end_frame: 121,
        },
        PreviewRegion {
            start_frame: 175,
            end_frame: 176,
        },
    ]
    .iter()
    .enumerate()
    {
        let tag = format!("still-region-{n}");
        let out = c.cache_dir.join(format!("{tag}-counted.mkv"));
        let plan =
            render::compile_region(&c, &actual, &settings(&actual), false, &out, &tag, region)
                .unwrap();
        let mut max_decoded = 0usize;
        for range in &plan.input_ranges {
            assert_eq!(range.seek, Rational::zero());
            let frames = range.duration.checked_mul(actual.fps).unwrap();
            assert_eq!(frames.den, 1);
            assert!(
                frames.num <= plan.working_region.end_frame - plan.working_region.start_frame + 1
            );
            max_decoded += frames.num as usize + 16;
        }
        let decoded = render_counted_still(&c, &plan.args);
        assert!(
            decoded <= max_decoded,
            "Repeated still prefix in {region:?}: decoded={decoded}, bound={max_decoded}"
        );
        compare(&c, &actual, region, &v, &a, &tag);
        eprintln!("VERIFIED {tag}: {decoded} decoded input frames, {max_decoded} bounded maximum; every RGB byte and exact silent sample coverage match the unsplit full reference.");
    }
    assert_eq!(c.processes.active_count(), 0);
}
