use mono_cut_lib::{
    edit,
    media::{self, MediaContext},
    model::*,
    storage::{write_atomic, ProjectStore},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

fn context(dir: &Path) -> MediaContext {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/media");
    MediaContext {
        ffmpeg: std::env::var_os("MONO_CUT_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or("ffmpeg".into()),
        ffprobe: std::env::var_os("MONO_CUT_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or("ffprobe".into()),
        font: resources.join("Inter.ttf"),
        cache_dir: dir.join("cache"),
        processes: Default::default(),
    }
}
// Model-boundary cases need an available file, but never probe or decode it.
fn fixture(dir: &Path) -> Project {
    let mut p = Project::new("Retiming boundaries".into(), 160, 90, Rational::new(30, 1));
    let file = dir.join("model-source.bin");
    fs::write(&file, b"model boundary only").unwrap();
    p.media.push(Media {
        id: "source".into(),
        name: "Source".into(),
        path: file.to_string_lossy().into(),
        kind: "video".into(),
        duration: Rational::new(100, 1),
        fps: Rational::new(30, 1),
        width: 160,
        height: 90,
        has_audio: true,
        bin_id: None,
        thumbnail: None,
        waveform: vec![],
        proxy: None,
        timing: None,
        proxy_timing_version: None,
        legacy_source: None,
        missing: false,
    });
    let track = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: "source".into(),
            track_id: track,
            start: 20,
            source_in: Some(Rational::new(7, 13)),
            duration: Some(330),
        },
    )
    .unwrap();
    p
}
fn retime(p: &mut Project, id: &str, speed: Rational) {
    edit::apply(
        p,
        EditCommand::RetimeClip {
            id: id.into(),
            speed,
        },
    )
    .unwrap();
}
fn open_store(dir: &Path, p: &Project) -> ProjectStore {
    let path = dir.join("input.monocut");
    write_atomic(&path, p).unwrap();
    let mut s = ProjectStore::new(dir.join("recovery.json"));
    s.open(&path).unwrap();
    s
}
fn assert_failure_atomic(dir: &Path, p: Project, command: EditCommand) {
    let ctx = context(dir);
    let mut s = open_store(dir, &p);
    let renamed = s
        .apply(
            EditCommand::Rename {
                name: "Redo remains available".into(),
            },
            &ctx,
        )
        .unwrap();
    let before = s.undo().unwrap();
    let bytes = fs::read(dir.join("recovery.json")).unwrap();
    assert!(s.apply(command, &ctx).is_err());
    assert_eq!(s.get(), before);
    assert_eq!(fs::read(dir.join("recovery.json")).unwrap(), bytes);
    assert_eq!(s.redo().unwrap(), renamed);
    assert_eq!(s.undo().unwrap(), before);
}

#[test]
fn actual_full_source_retimes_transactionally_and_roundtrips_exactly() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let source = dir.path().join("full-480.mkv");
    media::run_ffmpeg(
        &ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=160x90:rate=30:duration=16".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "aevalsrc=if(between(t\\,4\\,4.02)\\,0.4\\,0):s=48000:d=16".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            "-shortest".into(),
            source.to_string_lossy().into(),
        ],
    )
    .unwrap();
    let mut s = ProjectStore::new(dir.path().join("recovery.json"));
    let p = s.import(vec![source], &ctx).unwrap();
    let original = s
        .apply(
            EditCommand::AddClip {
                media_id: p.media[0].id.clone(),
                track_id: p.tracks[0].id.clone(),
                start: 0,
                source_in: None,
                duration: Some(480),
            },
            &ctx,
        )
        .unwrap();
    let id = original.clips[0].id.clone();
    let faster = s
        .apply(
            EditCommand::RetimeClip {
                id: id.clone(),
                speed: Rational::new(2, 1),
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(faster.clips[0].duration, 240);
    assert_eq!(
        faster.clips[0].retime.as_ref().unwrap().source_span,
        Rational::new(16, 1)
    );
    assert_eq!(s.undo().unwrap(), original);
    assert_eq!(s.redo().unwrap(), faster);
    let slower = s
        .apply(
            EditCommand::RetimeClip {
                id: id.clone(),
                speed: Rational::new(1, 2),
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(slower.clips[0].duration, 960);
    let fractional = s
        .apply(
            EditCommand::RetimeClip {
                id: id.clone(),
                speed: Rational::new(7, 6),
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(fractional.clips[0].duration, 411);
    let restored = s
        .apply(
            EditCommand::RetimeClip {
                id: id.clone(),
                speed: Rational::one(),
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(restored.clips[0].duration, 480);
    assert_eq!(restored.clips[0].source_in, Rational::zero());
    let saved = dir.path().join("retimed.monocut");
    s.save(&saved).unwrap();
    let mut reopened = ProjectStore::new(dir.path().join("reopened-recovery.json"));
    assert_eq!(reopened.open_hydrated(&saved, &ctx).unwrap(), restored);
}

#[test]
fn repeated_rational_rates_keep_source_span_and_fractional_envelopes() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    p.fps = Rational::new(30000, 1001);
    let c = &mut p.clips[0];
    c.fade_in = 17;
    c.fade_in_start = Some(-9);
    c.fade_out = 29;
    c.fade_out_end = Some(372);
    c.keyframes = vec![
        Keyframe {
            property: "volume".into(),
            frame: 73,
            value: 0.2,
        },
        Keyframe {
            property: "volume".into(),
            frame: 330,
            value: 0.9,
        },
    ];
    c.render_offset = Some(9);
    c.composition = Some(CompositionOrigin {
        group_id: "split-layer".into(),
        offset: 9,
    });
    let id = c.id.clone();
    let origin = c
        .source_in
        .checked_sub(Rational::from_frames(9, p.fps))
        .unwrap();
    retime(&mut p, &id, Rational::new(7, 6));
    let exact = p.clips[0].retime.clone().unwrap();
    assert_eq!(p.clips[0].duration, 282);
    assert_eq!(exact.source_span, Rational::new(11011, 1000));
    assert_eq!(exact.render_source_origin, origin);
    assert_eq!(
        exact.envelope.keyframes[0].time,
        Rational::new(73073, 30000)
    );
    for speed in [
        Rational::new(17, 10),
        Rational::new(3, 7),
        Rational::new(32, 1),
        Rational::new(1, 20),
        Rational::one(),
    ] {
        retime(&mut p, &id, speed);
        let current = p.clips[0].retime.as_ref().unwrap();
        assert_eq!(current.source_span, exact.source_span);
        assert_eq!(current.envelope, exact.envelope);
        assert_eq!(current.render_source_origin, exact.render_source_origin);
        assert_eq!(
            current
                .composition_source_offset
                .unwrap()
                .checked_mul(Rational::new(speed.den, speed.num))
                .unwrap(),
            Rational::from_frames(9, p.fps)
        );
        assert_eq!(p.clips[0].source_in, Rational::new(7, 13));
        assert_eq!(p.clips[0].start, 20);
    }
    assert_eq!(p.clips[0].duration, 330);
    assert_eq!(p.clips[0].fade_in, 17);
    assert_eq!(p.clips[0].fade_in_start, Some(-9));
    assert_eq!(p.clips[0].fade_out_end, Some(372));
}

#[test]
fn linked_offsets_are_fixed_and_one_undo_restores_both_clips() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let mut audio = p.clips[0].clone();
    p.clips[0].linked_id = Some("av".into());
    audio.id = "audio".into();
    audio.track_id = p.tracks[1].id.clone();
    audio.source_in = Rational::new(1, 5);
    audio.linked_id = Some("av".into());
    p.clips.push(audio);
    let id = p.clips[0].id.clone();
    let ctx = context(dir.path());
    let mut s = open_store(dir.path(), &p);
    let original = s.get();
    let faster = s
        .apply(
            EditCommand::RetimeClip {
                id,
                speed: Rational::new(2, 1),
            },
            &ctx,
        )
        .unwrap();
    assert!(faster
        .clips
        .iter()
        .all(|c| c.duration == 165 && c.start == 20));
    assert_eq!(faster.clips[0].source_in, Rational::new(7, 13));
    assert_eq!(faster.clips[1].source_in, Rational::new(1, 5));
    assert_eq!(s.undo().unwrap(), original);
    assert_eq!(s.redo().unwrap(), faster);
}

#[test]
fn invalid_locked_missing_linked_range_and_collision_fail_without_history_changes() {
    for index in 0..11 {
        let dir = TempDir::new().unwrap();
        let mut p = fixture(dir.path());
        let id = p.clips[0].id.clone();
        let mut speed = Rational::new(2, 1);
        match index {
            0 => speed = Rational { num: 1, den: 0 },
            1 => speed = Rational::zero(),
            2 => speed = Rational::new(-1, 1),
            3 => speed = Rational::new(1, 21),
            4 => speed = Rational::new(33, 1),
            5 => p.tracks[0].locked = true,
            6 => {
                let mut companion = p.clips[0].clone();
                p.clips[0].linked_id = Some("link".into());
                companion.id = "locked-partner".into();
                companion.track_id = p.tracks[1].id.clone();
                companion.linked_id = Some("link".into());
                p.tracks[1].locked = true;
                p.clips.push(companion);
            }
            7 => {
                fs::remove_file(&p.media[0].path).unwrap();
            }
            8 => p.out_point = Some(350),
            9 => {
                let mut neighbor = p.clips[0].clone();
                neighbor.id = "neighbor".into();
                neighbor.start = 350;
                neighbor.duration = 30;
                p.clips.push(neighbor);
                speed = Rational::new(1, 2);
            }
            10 => {
                let mut companion = p.clips[0].clone();
                p.clips[0].linked_id = Some("link".into());
                companion.id = "offset".into();
                companion.track_id = p.tracks[1].id.clone();
                companion.start += 1;
                companion.linked_id = Some("link".into());
                p.clips.push(companion);
            }
            _ => unreachable!(),
        }
        assert_failure_atomic(dir.path(), p, EditCommand::RetimeClip { id, speed });
    }
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let id = p.clips[0].id.clone();
    p.clips[0].duration = 1;
    assert_failure_atomic(
        dir.path(),
        p,
        EditCommand::RetimeClip {
            id,
            speed: Rational::new(32, 1),
        },
    );
}

#[test]
fn titles_stills_and_mixed_speed_patches_reject_but_legacy_speed_patch_routes() {
    let dir = TempDir::new().unwrap();
    let p = fixture(dir.path());
    let id = p.clips[0].id.clone();
    assert_failure_atomic(
        dir.path(),
        p.clone(),
        EditCommand::UpdateClip {
            id: id.clone(),
            patch: json!({"speed":{"num":2,"den":1},"volume":0.5}),
        },
    );
    for image in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut p = fixture(dir.path());
        let id = p.clips[0].id.clone();
        if image {
            p.media[0].kind = "image".into();
            p.media[0].has_audio = false;
        } else {
            p.clips[0].media_id = None;
            p.clips[0].title = Some("Title".into());
        }
        assert_failure_atomic(
            dir.path(),
            p,
            EditCommand::RetimeClip {
                id,
                speed: Rational::new(2, 1),
            },
        );
    }
    let mut p = p;
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id: id.clone(),
            patch: json!({"speed":{"num":2,"den":1}}),
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].duration, 165);
    let canonical = p.clips[0].retime.clone();
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id,
            patch: json!({"opacity":0.4,"transform":{"x":2.0}}),
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].retime, canonical);
    assert_eq!(p.clips[0].opacity, 0.4);
}

#[test]
fn preexisting_overlaps_may_shrink_but_cannot_expand() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let id = p.clips[0].id.clone();
    let mut other = p.clips[0].clone();
    other.id = "existing-overlap".into();
    other.start = 320;
    other.duration = 90;
    p.clips.push(other);
    retime(&mut p, &id, Rational::new(2, 1));
    assert_eq!(p.clips[1].start, 320);
    let before = p.clone();
    assert!(edit::apply(
        &mut p,
        EditCommand::RetimeClip {
            id,
            speed: Rational::new(1, 2)
        }
    )
    .is_err());
    assert_eq!(p, before);
}

#[test]
fn retimed_split_trim_and_property_edits_preserve_canonical_clocks() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    p.clips[0].source_in = Rational::new(2, 1);
    p.clips[0].render_offset = Some(30);
    p.clips[0].fade_in = 45;
    p.clips[0].fade_out = 45;
    p.clips[0].keyframes = vec![
        Keyframe {
            property: "volume".into(),
            frame: 0,
            value: 0.2,
        },
        Keyframe {
            property: "volume".into(),
            frame: 330,
            value: 0.8,
        },
        Keyframe {
            property: "opacity".into(),
            frame: 17,
            value: 0.4,
        },
        Keyframe {
            property: "opacity".into(),
            frame: 330,
            value: 1.0,
        },
    ];
    let id = p.clips[0].id.clone();
    retime(&mut p, &id, Rational::new(7, 6));
    edit::apply(
        &mut p,
        EditCommand::Split {
            ids: vec![id.clone()],
            frame: 120,
        },
    )
    .unwrap();
    let right = p.clips.iter().find(|c| c.id != id).unwrap().clone();
    let state = right.retime.clone().unwrap();
    assert_eq!(state.source_span, Rational::new(64, 9));
    assert_eq!(state.render_source_origin, Rational::one());
    assert_eq!(state.composition_source_offset, Some(Rational::new(35, 9)));
    assert_eq!(state.envelope.fade_in_start, Rational::new(-35, 9));
    assert_eq!(state.envelope.fade_out_end, Rational::new(64, 9));
    let right_id = right.id.clone();
    edit::apply(
        &mut p,
        EditCommand::Trim {
            id: right_id.clone(),
            edge: "in".into(),
            frame: 130,
        },
    )
    .unwrap();
    let right = p.clips.iter().find(|c| c.id == right_id).unwrap();
    assert_eq!(
        right.retime.as_ref().unwrap().source_span,
        Rational::new(121, 18)
    );
    let unchanged_volume: Vec<_> = right
        .retime
        .as_ref()
        .unwrap()
        .envelope
        .keyframes
        .iter()
        .filter(|k| k.property == "volume")
        .cloned()
        .collect();
    let mut keys = right.keyframes.clone();
    keys.iter_mut()
        .filter(|k| k.property == "opacity")
        .for_each(|k| k.value = 0.7);
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id: right_id.clone(),
            patch: json!({"keyframes":keys,"fade_in":6}),
        },
    )
    .unwrap();
    let right = p.clips.iter().find(|c| c.id == right_id).unwrap();
    let state = right.retime.as_ref().unwrap();
    assert_eq!(
        state
            .envelope
            .keyframes
            .iter()
            .filter(|k| k.property == "volume")
            .cloned()
            .collect::<Vec<_>>(),
        unchanged_volume
    );
    assert_eq!(state.envelope.fade_in, Rational::new(7, 30));
    assert_eq!(state.envelope.fade_in_start, Rational::zero());
    assert_eq!(state.render_source_origin, Rational::one());
}

#[test]
fn unlink_duplicate_and_slip_keep_canonical_ownership_consistent() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    p.clips[0].source_in = Rational::new(2, 1);
    p.clips[0].keyframes = vec![
        Keyframe {
            property: "volume".into(),
            frame: 0,
            value: 0.2,
        },
        Keyframe {
            property: "opacity".into(),
            frame: 30,
            value: 0.6,
        },
    ];
    let id = p.clips[0].id.clone();
    retime(&mut p, &id, Rational::new(7, 6));
    edit::apply(
        &mut p,
        EditCommand::Unlink {
            ids: vec![id.clone()],
        },
    )
    .unwrap();
    let video = p.clips.iter().find(|c| c.id == id).unwrap();
    let audio = p.clips.iter().find(|c| c.id != id).unwrap();
    assert!(video
        .retime
        .as_ref()
        .unwrap()
        .envelope
        .keyframes
        .iter()
        .all(|k| k.property != "volume"));
    assert!(audio
        .retime
        .as_ref()
        .unwrap()
        .envelope
        .keyframes
        .iter()
        .all(|k| k.property == "volume"));
    assert!(audio
        .retime
        .as_ref()
        .unwrap()
        .composition_source_offset
        .is_none());
    edit::apply(
        &mut p,
        EditCommand::Split {
            ids: vec![id.clone()],
            frame: 120,
        },
    )
    .unwrap();
    let fragment = p.clips.iter().find(|c| c.start == 120).unwrap().clone();
    edit::apply(
        &mut p,
        EditCommand::Duplicate {
            ids: vec![fragment.id.clone()],
        },
    )
    .unwrap();
    let duplicated = p
        .clips
        .iter()
        .find(|c| c.id != fragment.id && c.start == fragment.end())
        .unwrap();
    assert_eq!(
        duplicated
            .retime
            .as_ref()
            .unwrap()
            .composition_source_offset,
        Some(Rational::zero())
    );
    edit::apply(
        &mut p,
        EditCommand::Slip {
            id: fragment.id.clone(),
            delta: 3,
        },
    )
    .unwrap();
    let slipped = p.clips.iter().find(|c| c.id == fragment.id).unwrap();
    assert_eq!(
        slipped.retime.as_ref().unwrap().render_source_origin,
        slipped.source_in
    );
}

#[test]
fn malformed_retained_metadata_is_rejected() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let id = p.clips[0].id.clone();
    retime(&mut p, &id, Rational::new(2, 1));
    for case in 0..6 {
        let mut invalid = p.clone();
        let r = invalid.clips[0].retime.as_mut().unwrap();
        match case {
            0 => r.source_span = Rational::new(1, 1),
            1 => r.render_source_origin = Rational::new(-1, 1),
            2 => r.envelope.keyframes.push(SourceKeyframe {
                property: "volume".into(),
                time: Rational::new(12, 1),
                value: 0.2,
            }),
            3 => r.envelope.fade_in = Rational::new(-1, 1),
            4 => invalid.clips[0].fade_in = 1,
            5 => invalid.clips[0].keyframes.push(Keyframe {
                property: "volume".into(),
                frame: 0,
                value: 0.5,
            }),
            _ => unreachable!(),
        }
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn backward_trim_and_source_in_patches_preserve_exact_span_and_signed_phase() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let id = p.clips[0].id.clone();
    retime(&mut p, &id, Rational::new(7, 6));
    let original = p.clips[0].retime.clone().unwrap();
    let source_in = p.clips[0].source_in;
    let duration = p.clips[0].duration;
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id: id.clone(),
            patch: json!({"source_in":source_in,"duration":duration}),
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].retime.as_ref().unwrap(), &original);
    edit::apply(
        &mut p,
        EditCommand::Trim {
            id: id.clone(),
            edge: "in".into(),
            frame: 19,
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].duration, 283);
    let shifted = p.clips[0].retime.as_ref().unwrap();
    assert_eq!(shifted.source_span, Rational::new(1987, 180));
    assert_eq!(shifted.render_source_origin, original.render_source_origin);
    assert!(p.clips[0].source_in.value() < shifted.render_source_origin.value());
    let span = shifted.source_span;
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id,
            patch: json!({"source_in":{"num":3,"den":1}}),
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].retime.as_ref().unwrap().source_span, span);
    assert_eq!(
        p.clips[0].retime.as_ref().unwrap().render_source_origin,
        Rational::new(3, 1)
    );
}

#[test]
fn retiming_a_split_layer_preserves_its_sequence_composition_anchor() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    p.clips[0].start = 0;
    p.clips[0].duration = 200;
    p.clips[0].source_in = Rational::new(2, 1);
    let left = p.clips[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::Split {
            ids: vec![left.clone()],
            frame: 100,
        },
    )
    .unwrap();
    let right = p.clips.iter().find(|c| c.id != left).unwrap().id.clone();
    let mut overlay = p.clips[0].clone();
    overlay.id = "overlay-B".into();
    overlay.start = 25;
    overlay.duration = 150;
    overlay.composition = None;
    overlay.render_offset = None;
    p.clips.push(overlay);
    for speed in [Rational::new(2, 1), Rational::new(7, 6), Rational::one()] {
        retime(&mut p, &right, speed);
        let fragment = p.clips.iter().find(|c| c.id == right).unwrap();
        let offset = fragment
            .retime
            .as_ref()
            .unwrap()
            .composition_source_offset
            .unwrap()
            .checked_mul(Rational::new(speed.den, speed.num))
            .unwrap();
        assert_eq!(
            Rational::from_frames(fragment.start, p.fps)
                .checked_sub(offset)
                .unwrap(),
            Rational::zero()
        );
        assert_eq!(fragment.composition_start(), 0);
        // Returning to a slower rate requires clearing the fixed neighboring
        // overlay after the first anchor assertion, under the collision policy.
        p.clips.retain(|c| c.id != "overlay-B");
    }
}

#[test]
fn no_op_generic_controls_keep_fractional_tail_anchors_and_hidden_keys() {
    let dir = TempDir::new().unwrap();
    let mut p = fixture(dir.path());
    let c = &mut p.clips[0];
    c.fade_in = 17;
    c.fade_out = 29;
    c.fade_in_start = Some(-9);
    c.fade_out_end = Some(372);
    c.keyframes = vec![
        Keyframe {
            property: "volume".into(),
            frame: 17,
            value: 0.2,
        },
        Keyframe {
            property: "volume".into(),
            frame: 18,
            value: 0.3,
        },
        Keyframe {
            property: "volume".into(),
            frame: 330,
            value: 0.9,
        },
        Keyframe {
            property: "opacity".into(),
            frame: 73,
            value: 0.4,
        },
        Keyframe {
            property: "opacity".into(),
            frame: 149,
            value: 0.7,
        },
    ];
    let id = c.id.clone();
    retime(&mut p, &id, Rational::new(32, 1));
    let c = &p.clips[0];
    let canonical = c.retime.clone().unwrap();
    let mut keys = c.keyframes.clone();
    keys.reverse();
    let patch = json!({"duration":c.duration,"source_in":c.source_in,"fade_in":c.fade_in,"fade_out":c.fade_out,"keyframes":keys});
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id: id.clone(),
            patch,
        },
    )
    .unwrap();
    assert_eq!(p.clips[0].retime.as_ref().unwrap(), &canonical);
    assert_eq!(
        canonical
            .envelope
            .keyframes
            .iter()
            .filter(|k| k.property == "volume")
            .count(),
        3
    );
    assert_eq!(
        p.clips[0]
            .keyframes
            .iter()
            .filter(|k| k.property == "volume")
            .count(),
        2
    );
    edit::apply(
        &mut p,
        EditCommand::UpdateClip {
            id,
            patch: json!({"fade_in":1}),
        },
    )
    .unwrap();
    assert_eq!(
        p.clips[0].retime.as_ref().unwrap().envelope.fade_in,
        Rational::new(16, 15)
    );
    assert_eq!(
        p.clips[0].retime.as_ref().unwrap().envelope.fade_in_start,
        Rational::zero()
    );
}
