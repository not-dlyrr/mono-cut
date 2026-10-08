//! A genuine old-v1 document used a container's absolute end as source duration.
//! Exercise its long saved edit through migration and byte-identical relinking.
//! All media is synthetic, all paths are temporary, and no audio is played.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    render,
    storage::ProjectStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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

fn fixture(ctx: &MediaContext, path: &Path, duration: &str, gray: &str) {
    media::run_ffmpeg(
        ctx,
        &vec![
            "-copyts".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c={gray}:size=160x90:rate=30:duration={duration}"),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("aevalsrc=if(between(t\\,0.5\\,0.52)\\,0.6\\,0):s=48000:d={duration}"),
            "-filter_complex".into(),
            "[0:v]setpts=PTS+3/TB[v];[1:a]asetpts=PTS+3.2/TB[a]".into(),
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
            path.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
}

fn container_duration(ctx: &MediaContext, path: &Path) -> Rational {
    let output = media::command(&ctx.ffprobe)
        .args(["-v", "error", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let raw: Value = serde_json::from_slice(&output.stdout).unwrap();
    media::decimal_rational(raw["format"]["duration"].as_str().unwrap()).unwrap()
}

fn historical_project(ctx: &MediaContext, source: &Path) -> Project {
    let mut p = Project::new(
        "Historical full-length edit".into(),
        160,
        90,
        Rational::new(30, 1),
    );
    let mut m = media::probe(ctx, source).unwrap();
    assert_eq!(m.duration, Rational::new(11, 5));
    m.duration = container_duration(ctx, source);
    assert_eq!(m.duration, Rational::new(26, 5));
    m.timing = None;
    m.proxy_timing_version = None;
    p.media.push(m);
    let media_id = p.media[0].id.clone();
    let track_id = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: media_id.clone(),
            track_id,
            start: 0,
            source_in: Some(Rational::zero()),
            duration: Some(156),
        },
    )
    .unwrap();
    edit::apply(
        &mut p,
        EditCommand::AddBin {
            name: "Historical bin".into(),
        },
    )
    .unwrap();
    let bin_id = p.bins[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AssignBin {
            media_id,
            bin_id: Some(bin_id),
        },
    )
    .unwrap();
    edit::apply(
        &mut p,
        EditCommand::Marker {
            frame: 21,
            name: "Historical pulse".into(),
        },
    )
    .unwrap();
    let clip_id = p.clips[0].id.clone();
    edit::apply(&mut p, EditCommand::UpdateClip { id: clip_id, patch: json!({"opacity":0.85,"brightness":0.03,"contrast":1.1,"volume":0.85,"transform":{"scale":0.8,"crop_left":0.05},"keyframes":[{"property":"volume","frame":0,"value":0.85},{"property":"volume","frame":155,"value":0.75}]}) }).unwrap();
    p
}

fn write_old_v1(path: &Path, p: &Project) {
    let mut raw = serde_json::to_value(p).unwrap();
    // Keep exactly the media fields present before source-timing migration,
    // including when future additive metadata is introduced.
    const OLD_FIELDS: &[&str] = &[
        "id",
        "name",
        "path",
        "kind",
        "duration",
        "fps",
        "width",
        "height",
        "has_audio",
        "bin_id",
        "thumbnail",
        "waveform",
        "proxy",
        "missing",
    ];
    raw["media"][0]
        .as_object_mut()
        .unwrap()
        .retain(|key, _| OLD_FIELDS.contains(&key.as_str()));
    assert_eq!(raw["version"], json!(1));
    fs::write(path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
}

fn assert_saved_edit(actual: &Project, original: &Project) {
    assert_eq!(actual.id, original.id);
    assert_eq!(actual.media[0].id, original.media[0].id);
    assert_eq!(actual.media[0].bin_id, original.media[0].bin_id);
    assert_eq!(
        actual.media[0]
            .duration
            .checked_sub(Rational::new(26, 5))
            .unwrap()
            .num,
        0
    );
    assert_eq!(actual.bins, original.bins);
    assert_eq!(actual.markers, original.markers);
    assert_eq!(
        actual.clips, original.clips,
        "Migration/relink must retain clip IDs, 156-frame edits and effects"
    );
    assert_eq!(actual.length(), 156);
    if actual.media[0].timing.is_some() {
        let compatibility = actual.media[0]
            .legacy_source
            .as_ref()
            .expect("Migrated historical source must carry explicit compatibility state");
        assert_eq!(
            compatibility
                .duration
                .checked_sub(Rational::new(11, 5))
                .unwrap()
                .num,
            0,
            "Compatibility stores the real span separately from the saved5.2s logical allowance"
        );
        let fingerprint = compatibility
            .sha256
            .as_ref()
            .expect("Available migrated sources must record content identity");
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

fn wait(manager: &JobManager, id: &str) -> Job {
    let start = Instant::now();
    loop {
        let job = manager.list().into_iter().find(|job| job.id == id).unwrap();
        if job.status != "running" {
            assert_eq!(job.status, "complete", "{job:?}");
            return job;
        }
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn decoded(ctx: &MediaContext, path: &Path, args: &[&str]) -> Vec<u8> {
    let output = media::command(&ctx.ffmpeg)
        .args(["-v", "error", "-i"])
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

fn verify_historical_render(ctx: &MediaContext, p: &Project, output: &Path, use_proxies: bool) {
    let settings = ExportSettings {
        width: 160,
        height: 90,
        fps: Rational::new(30, 1),
        codec: "ffv1".into(),
        crf: 18,
        audio_bitrate: 192,
        sample_rate: 48000,
    };
    let plan = render::compile(ctx, p, &settings, use_proxies, output, &id()).unwrap();
    assert_eq!(plan.output_frames, 156);
    assert!((plan.duration - 5.2).abs() < 1e-12);
    media::run_ffmpeg(ctx, &plan.args).unwrap();
    let raw_pcm = decoded(
        ctx,
        output,
        &["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le"],
    );
    let pcm: Vec<_> = raw_pcm
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        pcm.len(),
        249_600,
        "Historical output must keep its full 5.2-second sequence duration"
    );
    let cue = pcm.iter().position(|v| v.abs() > 0.2).unwrap();
    let tolerance = if use_proxies { 256 } else { 4 };
    assert!(
        (cue as i64 - 33_600).abs() <= tolerance,
        "Historical audio cue shifted to {cue}"
    );
    assert!(
        pcm[110_400..].iter().all(|v| v.abs() < 1e-6),
        "Historical surplus source interval must stay silent"
    );
    let pixels = decoded(
        ctx,
        output,
        &["-map", "0:v:0", "-pix_fmt", "rgb24", "-f", "rawvideo"],
    );
    let frame_bytes = 160 * 90 * 3;
    assert_eq!(pixels.len() / frame_bytes, 156);
    let center = |frame: &[u8]| frame[(45 * 160 + 80) * 3];
    assert!(
        center(&pixels[..frame_bytes]) > 40,
        "Actual footage disappeared"
    );
    assert!(
        pixels[60 * frame_bytes..]
            .chunks_exact(frame_bytes)
            .all(|frame| center(frame) <= 2),
        "Historical surplus source interval must remain neutral black"
    );
    eprintln!("VERIFIED historical5.2s timeline proxies={use_proxies}:156decodedframes,249600samples,cue{cue},neutralblack/silenttail; source offsets and all saved clip effects retained.");
}

#[test]
fn historical_absolute_duration_survives_identical_relink_and_rejects_incompatible_sources_transactionally(
) {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("historical.mkv");
    fixture(&ctx, &source, "2", "0x777777");
    let original = historical_project(&ctx, &source);
    let legacy_path = dir.path().join("legacy.monocut");
    write_old_v1(&legacy_path, &original);
    let recovery_path = dir.path().join("recovery.json");
    let mut store = ProjectStore::new(recovery_path.clone());
    let migrated = store.open_hydrated(&legacy_path, &ctx).unwrap();
    assert_saved_edit(&migrated, &original);
    assert_eq!(
        migrated.media[0].timing.as_ref().unwrap().origin,
        Rational::new(3, 1)
    );
    let same = store.apply(EditCommand::Relink { media_id: original.media[0].id.clone(), path: source.to_string_lossy().into_owned() }, &ctx).expect("Relinking the same existing legacy source must preserve its historical logical duration");
    assert_saved_edit(&same, &original);
    let saved_path = dir.path().join("migrated.monocut");
    store.save(&saved_path).unwrap();
    let mut reopened = ProjectStore::new(dir.path().join("reopened-recovery.json"));
    let reopened_project = reopened.open_hydrated(&saved_path, &ctx).unwrap();
    assert_saved_edit(&reopened_project, &original);
    let mut recovered = ProjectStore::new(recovery_path.clone());
    let recovered_project = recovered.recover_hydrated(&ctx).unwrap();
    assert_saved_edit(&recovered_project, &original);
    verify_historical_render(
        &ctx,
        &reopened_project,
        &dir.path().join("historical-original.mkv"),
        false,
    );

    let prepared = PathBuf::from(media::prepare_source(&ctx, &reopened_project.media[0]).unwrap());
    assert_eq!(media::probe(&ctx, &prepared).unwrap().duration, Rational::new(11, 5), "Source preparation must cover the actual2.2s stream span, not the historical5.2s edit allowance");
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let proxy = manager.proxy(reopened_project.media[0].clone()).unwrap();
    let proxy = wait(&manager, &proxy.id);
    let proxy_path = PathBuf::from(proxy.path.unwrap());
    assert_eq!(
        media::probe(&ctx, &proxy_path).unwrap().duration,
        Rational::new(11, 5),
        "Proxy duration must remain the real normalized stream span"
    );
    let proxied = store.set_proxy(&original.media[0].id, proxy_path).unwrap();
    assert_saved_edit(&proxied, &original);
    verify_historical_render(
        &ctx,
        &proxied,
        &dir.path().join("historical-proxy.mkv"),
        true,
    );

    // Establish an undo entry and an available redo entry, then prove that a
    // rejected relink changes neither history nor durable project/recovery bytes.
    let base_name = store.get().name;
    store
        .apply(
            EditCommand::Rename {
                name: "Redo state".into(),
            },
            &ctx,
        )
        .unwrap();
    store.undo().unwrap();
    store.save(&saved_path).unwrap();
    let before = store.get();
    let before_saved = fs::read(&saved_path).unwrap();
    let before_recovery = fs::read(&recovery_path).unwrap();
    let shorter = dir.path().join("shorter.mkv");
    fixture(&ctx, &shorter, "1", "0x777777");
    let different = dir.path().join("different-same-metadata.mkv");
    fixture(&ctx, &different, "2", "0x444444");
    assert_eq!(
        media::probe(&ctx, &different).unwrap().duration,
        Rational::new(11, 5)
    );
    for rejected in [&shorter, &different] {
        assert!(store.apply(EditCommand::Relink { media_id:original.media[0].id.clone(), path:rejected.to_string_lossy().into_owned() }, &ctx).is_err(), "A shorter or different source must not inherit a legacy source's extended edit allowance");
        assert_eq!(store.get(), before);
        assert_eq!(fs::read(&saved_path).unwrap(), before_saved);
        assert_eq!(fs::read(&recovery_path).unwrap(), before_recovery);
    }
    assert_eq!(
        store.redo().unwrap().name,
        "Redo state",
        "Rejected relinking must preserve redo history"
    );
    let after_undo = store.undo().unwrap();
    assert_eq!(after_undo.name, base_name);
    assert_eq!(after_undo, before);

    // The saved migrated project has a known fingerprint. Moving the same bytes
    // requires relinking but must preserve the exact edit and logical allowance.
    let replacement = dir.path().join("byte-identical-replacement.mkv");
    fs::rename(&source, &replacement).unwrap();
    let missing = store.open_hydrated(&saved_path, &ctx).unwrap();
    assert!(missing.media[0].missing);
    assert_saved_edit(&missing, &original);
    let relinked = store
        .apply(
            EditCommand::Relink {
                media_id: original.media[0].id.clone(),
                path: replacement.to_string_lossy().into_owned(),
            },
            &ctx,
        )
        .expect("A byte-identical missing source must retain historical edits");
    assert!(!relinked.media[0].missing);
    assert_saved_edit(&relinked, &original);
    store.save(&saved_path).unwrap();
    assert_saved_edit(
        &reopened.open_hydrated(&saved_path, &ctx).unwrap(),
        &original,
    );
    assert_saved_edit(&recovered.recover_hydrated(&ctx).unwrap(), &original);
    verify_historical_render(
        &ctx,
        &relinked,
        &dir.path().join("historical-relinked.mkv"),
        false,
    );

    // A genuinely old document that is missing at its first new-engine open
    // has no historical fingerprint. The narrow metadata migration rule accepts
    // the original bytes, records identity, and leaves IDs/edits unchanged.
    let fresh_missing_recovery = dir.path().join("fresh-missing-recovery.json");
    let mut fresh_missing = ProjectStore::new(fresh_missing_recovery.clone());
    let unknown = fresh_missing.open_hydrated(&legacy_path, &ctx).unwrap();
    assert!(unknown.media[0].missing);
    assert!(unknown.media[0].timing.is_none());
    assert_saved_edit(&unknown, &original);
    let adopted = fresh_missing.apply(EditCommand::Relink { media_id: original.media[0].id.clone(), path:replacement.to_string_lossy().into_owned() }, &ctx).expect("First-open missing legacy source must support its documented metadata-based compatibility migration");
    assert_saved_edit(&adopted, &original);
    assert!(!adopted.media[0].missing);
    let fresh_saved = dir.path().join("fresh-missing-migrated.monocut");
    fresh_missing.save(&fresh_saved).unwrap();
    let adopted_saved = fs::read(&fresh_saved).unwrap();
    let adopted_recovery = fs::read(&fresh_missing_recovery).unwrap();
    assert!(fresh_missing.apply(EditCommand::Relink { media_id: original.media[0].id.clone(), path:different.to_string_lossy().into_owned() }, &ctx).is_err(), "After first-missing adoption, recorded identity must reject a same-metadata different source");
    assert_eq!(fresh_missing.get(), adopted);
    assert_eq!(fs::read(&fresh_saved).unwrap(), adopted_saved);
    assert_eq!(fs::read(&fresh_missing_recovery).unwrap(), adopted_recovery);
    let mut fresh_recovered = ProjectStore::new(fresh_missing_recovery);
    assert_saved_edit(&fresh_recovered.recover_hydrated(&ctx).unwrap(), &original);
    verify_historical_render(
        &ctx,
        &adopted,
        &dir.path().join("historical-first-missing-relink.mkv"),
        false,
    );
    eprintln!("VERIFIED genuineoldv1 absolute5.2s duration migration: sameexisting/known-identical-missing/first-open-missing relinks retain156frame edit/effects/IDs, save/reopen/recovery; normalizedprepared/proxyremain2.2s; short and same-metadata-different sources reject transactionally, including redo and saved/recovery bytes.");
}

#[test]
fn timing_only_legacy_migration_requires_old_provenance_and_valid_exact_span() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("old-timing-only.mkv");
    fixture(&ctx, &source, "2", "0x777777");
    let original = historical_project(&ctx, &source);
    let mut earlier_migration = original.clone();
    earlier_migration.media[0].timing = media::probe(&ctx, &source).unwrap().timing;
    earlier_migration.media[0]
        .timing
        .as_mut()
        .unwrap()
        .container_duration = None;
    assert!(earlier_migration.media[0].legacy_source.is_none());
    let old_timing_path = dir.path().join("earlier-timing-migration.monocut");
    let mut old_timing_json = serde_json::to_value(&earlier_migration).unwrap();
    old_timing_json["media"][0]["timing"]
        .as_object_mut()
        .unwrap()
        .remove("container_duration");
    old_timing_json["media"][0]
        .as_object_mut()
        .unwrap()
        .remove("legacy_source");
    fs::write(
        &old_timing_path,
        serde_json::to_vec_pretty(&old_timing_json).unwrap(),
    )
    .unwrap();
    let recovery = dir.path().join("old-timing-recovery.json");
    let mut store = ProjectStore::new(recovery.clone());
    let migrated = store
        .open_hydrated(&old_timing_path, &ctx)
        .expect("An earlier timing-only migration still needs historical duration compatibility");
    assert_saved_edit(&migrated, &original);
    let same = store
        .apply(
            EditCommand::Relink {
                media_id: original.media[0].id.clone(),
                path: source.to_string_lossy().into_owned(),
            },
            &ctx,
        )
        .unwrap();
    assert_saved_edit(&same, &original);

    // Modern provenance explicitly says this is a measured-duration record.
    // Enlarging that number cannot manufacture a historical tail on relink.
    let mut modern = earlier_migration.clone();
    modern.media[0].timing = media::probe(&ctx, &source).unwrap().timing;
    assert!(modern.media[0]
        .timing
        .as_ref()
        .unwrap()
        .container_duration
        .is_some());
    assert!(modern.media[0].legacy_source.is_none());
    let modern_path = dir.path().join("modern-changed-duration.monocut");
    fs::write(&modern_path, serde_json::to_vec_pretty(&modern).unwrap()).unwrap();
    let modern_recovery = dir.path().join("modern-recovery.json");
    let mut modern_store = ProjectStore::new(modern_recovery.clone());
    let modern_opened = modern_store.open_hydrated(&modern_path, &ctx).unwrap();
    assert!(
        modern_opened.media[0].legacy_source.is_none(),
        "A modern record must not infer compatibility from an enlarged duration"
    );
    let modern_saved = fs::read(&modern_path).unwrap();
    let modern_autosave = fs::read(&modern_recovery).unwrap();
    assert!(
        modern_store
            .apply(
                EditCommand::Relink {
                    media_id: original.media[0].id.clone(),
                    path: source.to_string_lossy().into_owned()
                },
                &ctx
            )
            .is_err(),
        "The full156-frame edit must exceed this modern source's actual2.2s bound after relinking"
    );
    assert_eq!(modern_store.get(), modern_opened);
    assert_eq!(fs::read(&modern_path).unwrap(), modern_saved);
    assert_eq!(fs::read(&modern_recovery).unwrap(), modern_autosave);

    // An explicit record may not contradict measured ends, even when its
    // logical saved duration still matches the historical container duration.
    let mut contradictory = migrated.clone();
    contradictory.media[0]
        .legacy_source
        .as_mut()
        .unwrap()
        .duration = Rational::one();
    assert!(
        contradictory.validate().is_err(),
        "A1s compatibility span cannot contain known2.2s stream ends"
    );
    let contradictory_path = dir.path().join("contradictory-span.monocut");
    fs::write(
        &contradictory_path,
        serde_json::to_vec_pretty(&contradictory).unwrap(),
    )
    .unwrap();
    let before = store.get();
    let before_recovery = fs::read(&recovery).unwrap();
    assert!(store.open(&contradictory_path).is_err());
    assert_eq!(store.get(), before);
    assert_eq!(fs::read(&recovery).unwrap(), before_recovery);

    // JSON rationals need not be reduced. Equivalent source clocks and spans
    // must pass validation and relinking using exact mathematical equality.
    let mut equivalent = migrated.clone();
    equivalent.media[0].duration = Rational { num: 52, den: 10 };
    equivalent.media[0].fps = Rational { num: 60, den: 2 };
    equivalent.media[0].legacy_source.as_mut().unwrap().duration = Rational { num: 22, den: 10 };
    let timing = equivalent.media[0].timing.as_mut().unwrap();
    timing.origin = Rational { num: 6, den: 2 };
    timing.video_start = Some(Rational { num: 9, den: 3 });
    timing.audio_start = Some(Rational { num: 32, den: 10 });
    timing.video_end = Some(Rational { num: 10, den: 2 });
    timing.audio_end = Some(Rational { num: 52, den: 10 });
    timing.container_duration = Some(Rational { num: 52, den: 10 });
    equivalent
        .validate()
        .expect("Unreduced equivalent source clocks and spans must be valid");
    let equivalent_path = dir.path().join("unreduced-times.monocut");
    fs::write(
        &equivalent_path,
        serde_json::to_vec_pretty(&equivalent).unwrap(),
    )
    .unwrap();
    assert_saved_edit(&store.open(&equivalent_path).unwrap(), &original);
    let equivalent_relinked = store
        .apply(
            EditCommand::Relink {
                media_id: original.media[0].id.clone(),
                path: source.to_string_lossy().into_owned(),
            },
            &ctx,
        )
        .expect("Equivalent unreduced metadata must match the same source on relink");
    assert_saved_edit(&equivalent_relinked, &original);
    store.save(&equivalent_path).unwrap();
    assert_saved_edit(
        &store.open_hydrated(&equivalent_path, &ctx).unwrap(),
        &original,
    );
    eprintln!("VERIFIED legacy timing provenance: earlier timing-only JSON gainscompatibility; modern measured records cannot infer a historicaltail; contradictory1s span rejects withoutstate changes; unreducedequivalent origins/starts/ends/fps/spans acceptsame-file relink and save/reopen.");
}
