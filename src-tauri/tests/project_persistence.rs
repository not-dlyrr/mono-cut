use mono_cut_lib::{
    edit,
    media::{self, MediaContext},
    model::*,
    storage::{read_project, write_atomic, ProjectStore, MAX_PROJECT_BYTES},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tempfile::TempDir;

// Each boundary fixture is generated in a disposable directory. Serialize these
// tests even under the default test runner to keep their memory use bounded.
static BOUNDARY_TEST: Mutex<()> = Mutex::new(());

struct ByteCounter(usize);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn pretty_bytes(project: &Project) -> usize {
    let mut counter = ByteCounter(0);
    serde_json::to_writer_pretty(&mut counter, project).unwrap();
    counter.0
}
fn context(dir: &Path) -> MediaContext {
    // These persistence tests never launch a media tool or play audio.
    MediaContext {
        ffmpeg: dir.join("unused-ffmpeg"),
        ffprobe: dir.join("unused-ffprobe"),
        cache_dir: dir.join("cache"),
        font: dir.join("unused-font"),
        processes: Default::default(),
    }
}
fn title(track_id: &str, index: usize, text: String) -> Clip {
    Clip {
        id: format!("00000000-0000-0000-0000-{index:012}"),
        media_id: None,
        track_id: track_id.into(),
        name: "Title".into(),
        start: 0,
        duration: 30,
        source_in: Rational::zero(),
        speed: Rational::one(),
        linked_id: None,
        title: Some(text),
        transform: Transform::default(),
        opacity: 1.,
        volume: 1.,
        fade_in: 0,
        fade_out: 0,
        fade_in_start: None,
        fade_out_end: None,
        composition: None,
        render_offset: None,
        brightness: 0.,
        contrast: 1.,
        saturation: 1.,
        keyframes: vec![],
    }
}

/// Fill with valid titles and adjust only the last title to an exact JSON byte
/// boundary. Fixed-width IDs make per-clip growth independent of the index.
fn boundary_project(mut project: Project, target: usize) -> Project {
    assert!(project.clips.is_empty());
    let track = project.tracks[0].id.clone();
    let base = pretty_bytes(&project);
    project.clips.push(title(&track, 0, "x".repeat(16_384)));
    let first = pretty_bytes(&project);
    project.clips.push(title(&track, 1, "x".repeat(16_384)));
    let subsequent = pretty_bytes(&project) - first;
    let count = 1 + (target - base - (first - base)) / subsequent;
    project.clips = (0..count)
        .map(|index| title(&track, index, "x".repeat(16_384)))
        .collect();
    let mut current = pretty_bytes(&project);
    assert!(current <= target);
    if current != target {
        // Allow a final title's fixed JSON overhead even when the remaining
        // space is tiny, then distribute the remaining bytes over two titles.
        project.clips.last_mut().unwrap().title = Some(String::new());
        project.clips.push(title(&track, count, String::new()));
        current = pretty_bytes(&project);
        assert!(current <= target);
        let remaining = target - current;
        let last = project.clips.len() - 1;
        let last_size = remaining.min(16_384);
        project.clips[last].title = Some("x".repeat(last_size));
        project.clips[last - 1].title = Some("x".repeat(remaining - last_size));
    }
    assert_eq!(pretty_bytes(&project), target);
    project.validate().unwrap();
    println!(
        "Generated {} valid titles, {} serialized bytes (limit {}).",
        project.clips.len(),
        target,
        MAX_PROJECT_BYTES
    );
    project
}
fn digest(path: &Path) -> [u8; 32] {
    let mut file = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    hash.finalize().into()
}
fn marker() -> EditCommand {
    EditCommand::Marker {
        frame: 1,
        name: "Preserve redo".into(),
    }
}
fn assert_size_error(error: &str) {
    assert!(error.contains("64 MiB"), "Unexpected error: {error}");
}
fn media_fixture(path: &Path) -> Media {
    Media {
        id: "media-source".into(),
        name: "Missing-media path fixture".into(),
        path: media::normalize_path(path.canonicalize().unwrap())
            .to_string_lossy()
            .into_owned(),
        kind: "image".into(),
        duration: Rational::zero(),
        fps: Rational::zero(),
        width: 16,
        height: 16,
        has_audio: false,
        bin_id: None,
        thumbnail: None,
        waveform: vec![],
        proxy: None,
        timing: None,
        proxy_timing_version: None,
        legacy_source: None,
        missing: false,
    }
}

#[test]
fn boundary_explicit_save_and_autosave_reopen_through_the_actual_reader() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let input = dir.path().join("input.monocut");
    let saved = dir.path().join("saved.monocut");
    let recovery = dir.path().join("recovery.monocut");
    let project = boundary_project(Project::default(), MAX_PROJECT_BYTES - 1024);
    write_atomic(&input, &project).unwrap();
    let mut store = ProjectStore::new(recovery.clone());
    assert_eq!(store.open(&input).unwrap(), project);
    assert_eq!(store.save(&saved).unwrap(), project);
    assert_eq!(read_project(&saved).unwrap(), project);
    assert_eq!(
        fs::metadata(&saved).unwrap().len(),
        (MAX_PROJECT_BYTES - 1024) as u64
    );
    let edited = store.apply(marker(), &ctx).unwrap();
    assert!(pretty_bytes(&edited) <= MAX_PROJECT_BYTES);
    assert_eq!(read_project(&recovery).unwrap(), edited);
    assert!(store.recovery_available());
    store.save(&saved).unwrap();
    let mut reopened = ProjectStore::new(dir.path().join("reopened-recovery.monocut"));
    assert_eq!(reopened.open(&saved).unwrap(), edited);
    let mut recovered = ProjectStore::new(recovery);
    assert_eq!(recovered.recover().unwrap(), edited);
    println!(
        "Successful explicit save and autosave round-tripped at {} bytes.",
        pretty_bytes(&edited)
    );
}

#[test]
fn oversized_title_edit_and_atomic_write_preserve_project_history_and_files() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let saved = dir.path().join("saved.monocut");
    let recovery = dir.path().join("recovery.monocut");
    let project = boundary_project(Project::default(), MAX_PROJECT_BYTES - 512);
    write_atomic(&saved, &project).unwrap();
    let mut store = ProjectStore::new(recovery.clone());
    store.open(&saved).unwrap();
    let redo_target = store.apply(marker(), &ctx).unwrap();
    assert_eq!(store.undo().unwrap(), project);
    let saved_before = digest(&saved);
    let recovery_before = digest(&recovery);
    let command = EditCommand::AddTitle {
        track_id: project.tracks[0].id.clone(),
        start: 0,
        duration: 30,
        text: "x".repeat(16_384),
    };
    let mut oversized = project.clone();
    edit::apply(&mut oversized, command.clone()).unwrap();
    let oversized_size = pretty_bytes(&oversized);
    assert!(oversized_size > MAX_PROJECT_BYTES);
    assert_size_error(&store.apply(command, &ctx).unwrap_err());
    assert_eq!(store.get(), project);
    assert_eq!(digest(&saved), saved_before);
    assert_eq!(digest(&recovery), recovery_before);
    assert_eq!(read_project(&saved).unwrap(), project);
    assert_eq!(read_project(&recovery).unwrap(), project);
    assert!(store.recovery_available());
    let mut recovered = ProjectStore::new(recovery.clone());
    assert_eq!(recovered.recover().unwrap(), project);
    // Redo and undo remain usable after the failed mutation.
    assert_eq!(store.redo().unwrap(), redo_target);
    assert_eq!(store.undo().unwrap(), project);
    assert_size_error(&write_atomic(&saved, &oversized).unwrap_err());
    assert_eq!(digest(&saved), saved_before);
    let nonexistent = dir.path().join("must-not-create").join("oversized.monocut");
    assert_size_error(&write_atomic(&nonexistent, &oversized).unwrap_err());
    assert!(!nonexistent.parent().unwrap().exists());
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "Rejected writes left a temporary file"
    );
    println!("Rejected {}-byte edit/write; prior project, undo/redo, save and recovery remained readable.", oversized_size);
}

#[test]
fn escaped_title_budget_counts_serialized_bytes_instead_of_raw_text() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let input = dir.path().join("input.monocut");
    let recovery = dir.path().join("recovery.monocut");
    let project = boundary_project(Project::default(), MAX_PROJECT_BYTES - 20_000);
    write_atomic(&input, &project).unwrap();
    let mut store = ProjectStore::new(recovery.clone());
    store.open(&input).unwrap();
    let command = |text: String| EditCommand::AddTitle {
        track_id: project.tracks[0].id.clone(),
        start: 0,
        duration: 30,
        text,
    };
    let ascii = command("x".repeat(16_384));
    let escaped = command("\n".repeat(16_384));
    let mut ascii_candidate = project.clone();
    edit::apply(&mut ascii_candidate, ascii.clone()).unwrap();
    let mut escaped_candidate = project.clone();
    edit::apply(&mut escaped_candidate, escaped.clone()).unwrap();
    assert!(pretty_bytes(&ascii_candidate) <= MAX_PROJECT_BYTES);
    assert!(pretty_bytes(&escaped_candidate) > MAX_PROJECT_BYTES);
    let recovery_before = digest(&recovery);
    assert_size_error(&store.apply(escaped, &ctx).unwrap_err());
    assert_eq!(store.get(), project);
    assert_eq!(digest(&recovery), recovery_before);
    let accepted = store.apply(ascii, &ctx).unwrap();
    assert_eq!(read_project(&recovery).unwrap(), accepted);
    println!(
        "Equal 16,384-byte titles: ASCII candidate {} bytes; escaped candidate {} bytes.",
        pretty_bytes(&ascii_candidate),
        pretty_bytes(&escaped_candidate)
    );
}

#[test]
fn growing_saved_relative_paths_and_proxy_updates_fail_before_replacing_files() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let source = dir.path().join("source.png");
    fs::write(&source, b"The persistence fixture never probes this file.").unwrap();
    let mut seed = Project::default();
    seed.media.push(media_fixture(&source));
    let project = boundary_project(seed, MAX_PROJECT_BYTES - 256);
    let input = dir.path().join("input.monocut");
    let recovery = dir.path().join("recovery.monocut");
    write_atomic(&input, &project).unwrap();
    let mut store = ProjectStore::new(recovery.clone());
    assert_eq!(store.open(&input).unwrap(), project);
    let redo_target = store.apply(marker(), &ctx).unwrap();
    store.undo().unwrap();
    let recovery_before = digest(&recovery);
    let input_before = digest(&input);
    assert_size_error(
        &store
            .set_proxy(&project.media[0].id, PathBuf::from("p".repeat(2048)))
            .unwrap_err(),
    );
    assert_eq!(store.get(), project);
    assert_eq!(digest(&recovery), recovery_before);

    let mut deep = dir.path().to_path_buf();
    let mut relative_source = PathBuf::new();
    for _ in 0..192 {
        deep.push("d");
        relative_source.push("..");
    }
    fs::create_dir_all(&deep).unwrap();
    relative_source.push("source.png");
    let mut serialized_saved = project.clone();
    serialized_saved.media[0].path = relative_source.to_string_lossy().into_owned();
    assert!(pretty_bytes(&serialized_saved) > MAX_PROJECT_BYTES);
    let destination = deep.join("saved.monocut");
    let previous_destination = Project::default();
    write_atomic(&destination, &previous_destination).unwrap();
    let destination_before = digest(&destination);
    assert_size_error(&store.save(&destination).unwrap_err());
    assert_eq!(store.get(), project);
    assert_eq!(digest(&destination), destination_before);
    assert_eq!(read_project(&destination).unwrap(), previous_destination);
    assert_eq!(digest(&input), input_before);
    assert_eq!(digest(&recovery), recovery_before);
    assert_eq!(read_project(&recovery).unwrap(), project);
    assert!(store.recovery_available());
    assert_eq!(store.redo().unwrap(), redo_target);
    assert_eq!(store.undo().unwrap(), project);
    println!("Autosave {} bytes fits; relative-path save {} bytes rejected without replacing destination.", pretty_bytes(&project), pretty_bytes(&serialized_saved));
}

#[test]
fn compact_input_with_oversized_canonical_recovery_is_transactionally_rejected() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    let candidate = boundary_project(Project::default(), MAX_PROJECT_BYTES + 512);
    let compact = serde_json::to_vec(&candidate).unwrap();
    assert!(compact.len() <= MAX_PROJECT_BYTES);
    let input = dir.path().join("compact.monocut");
    fs::write(&input, &compact).unwrap();
    assert_eq!(read_project(&input).unwrap(), candidate);
    let recovery = dir.path().join("recovery.monocut");
    let mut store = ProjectStore::new(recovery.clone());
    let baseline = store
        .new_project("Keep this project".into(), 1920, 1080, Rational::new(30, 1))
        .unwrap();
    let redo_target = store.apply(marker(), &ctx).unwrap();
    store.undo().unwrap();
    let recovery_before = digest(&recovery);
    assert_size_error(&store.open(&input).unwrap_err());
    assert_eq!(store.get(), baseline);
    assert_eq!(digest(&recovery), recovery_before);
    assert_eq!(store.redo().unwrap(), redo_target);
    assert_eq!(store.undo().unwrap(), baseline);
    fs::write(&recovery, &compact).unwrap();
    let compact_recovery_before = digest(&recovery);
    assert!(!store.recovery_available());
    assert_size_error(&store.recover().unwrap_err());
    assert_eq!(store.get(), baseline);
    assert_eq!(digest(&recovery), compact_recovery_before);
    assert_eq!(store.redo().unwrap(), redo_target);
    assert_eq!(store.undo().unwrap(), baseline);
    println!("Readable compact input {} bytes would require {} recovery bytes; open/recover rejected transactionally.", compact.len(), pretty_bytes(&candidate));
}

#[test]
fn exact_limit_is_reopenable_and_one_extra_serialized_byte_is_rejected() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let saved = dir.path().join("exact-limit.monocut");
    let recovery = dir.path().join("recovery.monocut");
    let mut project = boundary_project(Project::default(), MAX_PROJECT_BYTES);
    write_atomic(&saved, &project).unwrap();
    assert_eq!(
        fs::metadata(&saved).unwrap().len(),
        MAX_PROJECT_BYTES as u64
    );
    assert_eq!(read_project(&saved).unwrap(), project);
    let mut store = ProjectStore::new(recovery.clone());
    assert_eq!(store.open(&saved).unwrap(), project);
    store.save(&saved).unwrap();
    assert_eq!(read_project(&saved).unwrap(), project);
    assert_eq!(read_project(&recovery).unwrap(), project);
    assert!(store.recovery_available());
    let saved_before = digest(&saved);
    let recovery_before = digest(&recovery);
    project
        .clips
        .iter_mut()
        .find_map(|clip| clip.title.as_mut().filter(|text| text.len() < 16_384))
        .expect("Boundary fixture must have an adjustable partial title")
        .push('x');
    project.validate().unwrap();
    assert_eq!(pretty_bytes(&project), MAX_PROJECT_BYTES + 1);
    assert_size_error(&write_atomic(&saved, &project).unwrap_err());
    assert_eq!(digest(&saved), saved_before);
    assert_eq!(digest(&recovery), recovery_before);
    assert_eq!(
        fs::metadata(&saved).unwrap().len(),
        MAX_PROJECT_BYTES as u64
    );
    let oversized_input = dir.path().join("one-byte-too-large.monocut");
    let mut writer = io::BufWriter::new(fs::File::create(&oversized_input).unwrap());
    serde_json::to_writer_pretty(&mut writer, &project).unwrap();
    writer.flush().unwrap();
    assert_size_error(&read_project(&oversized_input).unwrap_err());
    println!(
        "Accepted exact {}-byte save/recovery; rejected {} bytes without replacing them.",
        MAX_PROJECT_BYTES,
        MAX_PROJECT_BYTES + 1
    );
}

#[test]
fn nonfinite_waveform_values_cannot_replace_a_readable_project() {
    let _serial = BOUNDARY_TEST.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("source.png");
    fs::write(&source, b"No media tools are launched.").unwrap();
    let mut project = Project::default();
    project.media.push(media_fixture(&source));
    let saved = dir.path().join("saved.monocut");
    write_atomic(&saved, &project).unwrap();
    let before = digest(&saved);
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut invalid = project.clone();
        invalid.media[0].waveform = vec![value];
        let error = write_atomic(&saved, &invalid).unwrap_err();
        assert!(
            error.to_lowercase().contains("waveform"),
            "Unexpected error: {error}"
        );
        assert_eq!(digest(&saved), before);
        assert_eq!(read_project(&saved).unwrap(), project);
    }
    println!("NaN and positive/negative infinity rejected without replacing a readable save.");
}
