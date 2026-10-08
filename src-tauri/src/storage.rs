use crate::{
    edit,
    media::{self, MediaContext},
    model::*,
};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

/// Inclusive encoded JSON limit shared by project and recovery readers/writers.
pub const MAX_PROJECT_BYTES: usize = 64 * 1024 * 1024;
fn size_error() -> String {
    "Project exceeds the supported 64 MiB size limit (67,108,864 bytes). Shorten title text or remove unused clips/media before trying again.".into()
}

#[derive(Default)]
struct ProjectBuffer {
    bytes: Vec<u8>,
    exceeded: bool,
}
impl Write for ProjectBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_PROJECT_BYTES - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::InvalidData, size_error()));
        }
        let next_len = self.bytes.len() + bytes.len();
        if next_len > self.bytes.capacity() {
            let capacity = next_len
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_PROJECT_BYTES);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode_project(project: &Project) -> Result<Vec<u8>, String> {
    project.validate()?;
    let mut buffer = ProjectBuffer::default();
    if let Err(error) = serde_json::to_writer_pretty(&mut buffer, project) {
        return Err(if buffer.exceeded {
            size_error()
        } else {
            format!("Cannot encode project: {error}")
        });
    }
    Ok(buffer.bytes)
}

pub struct ProjectStore {
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    recovery_path: PathBuf,
    project_path: Option<PathBuf>,
}
fn snapshot_bytes(p: &Project) -> usize {
    std::mem::size_of::<Project>()
        + p.clips
            .iter()
            .map(|c| {
                std::mem::size_of::<Clip>()
                    + c.name.len()
                    + c.title.as_ref().map(String::len).unwrap_or(0)
                    + c.keyframes.len() * std::mem::size_of::<Keyframe>()
                    + c.retime
                        .as_ref()
                        .map(|r| {
                            r.envelope
                                .keyframes
                                .iter()
                                .map(|k| std::mem::size_of::<SourceKeyframe>() + k.property.len())
                                .sum::<usize>()
                        })
                        .unwrap_or(0)
                    + c.composition
                        .as_ref()
                        .map(|origin| origin.group_id.len())
                        .unwrap_or(0)
            })
            .sum::<usize>()
        + p.media
            .iter()
            .map(|m| {
                std::mem::size_of::<Media>()
                    + m.path.len()
                    + m.name.len()
                    + m.waveform.len() * 4
                    + m.thumbnail.as_ref().map(String::len).unwrap_or(0)
                    + m.proxy.as_ref().map(String::len).unwrap_or(0)
                    + m.legacy_source
                        .as_ref()
                        .and_then(|legacy| legacy.sha256.as_ref())
                        .map(String::len)
                        .unwrap_or(0)
            })
            .sum::<usize>()
}
fn bound_history(history: &mut Vec<Project>) {
    while history.len() > 1
        && (history.len() > 100
            || history.iter().map(snapshot_bytes).sum::<usize>() > 64 * 1024 * 1024)
    {
        history.remove(0);
    }
}
impl ProjectStore {
    pub fn new(recovery_path: PathBuf) -> Self {
        Self {
            project: Project::default(),
            undo: vec![],
            redo: vec![],
            recovery_path,
            project_path: None,
        }
    }
    pub fn get(&self) -> Project {
        self.project.clone()
    }
    fn commit(&mut self, next: Project) -> Result<Project, String> {
        next.validate()?;
        write_atomic(&self.recovery_path, &next)?;
        self.undo.push(self.project.clone());
        bound_history(&mut self.undo);
        self.redo.clear();
        self.project = next;
        Ok(self.get())
    }
    pub fn new_project(
        &mut self,
        name: String,
        width: u32,
        height: u32,
        fps: Rational,
    ) -> Result<Project, String> {
        let next = Project::new(name, width, height, fps);
        next.validate()?;
        write_atomic(&self.recovery_path, &next)?;
        self.project = next;
        self.undo.clear();
        self.redo.clear();
        self.project_path = None;
        Ok(self.get())
    }
    pub fn import(&mut self, paths: Vec<PathBuf>, ctx: &MediaContext) -> Result<Project, String> {
        if paths.is_empty() {
            return Ok(self.get());
        }
        let mut next = self.get();
        for path in paths {
            let m = media::import(ctx, &path)?;
            if !next.media.iter().any(|x| x.path == m.path) {
                next.media.push(m);
            }
        }
        self.commit(next)
    }
    pub fn apply(&mut self, command: EditCommand, ctx: &MediaContext) -> Result<Project, String> {
        let mut next = self.get();
        if let EditCommand::Relink { media_id, path } = command {
            let mut replacement = media::import(ctx, Path::new(&path))?;
            let current = next
                .media
                .iter_mut()
                .find(|m| m.id == media_id)
                .ok_or("Media not found")?;
            if current.kind != replacement.kind {
                return Err("Replacement media must be the same kind".into());
            }
            if let Some(compatibility) = legacy_compatibility(current, &replacement, ctx, true)? {
                // Preserve the historical logical extent, not an altered trim
                // or speed. The measured source clock still comes from probing.
                replacement.duration = current.duration;
                replacement.legacy_source = Some(compatibility);
            }
            let id = current.id.clone();
            let bin = current.bin_id.clone();
            *current = replacement;
            current.id = id;
            current.bin_id = bin;
        } else {
            edit::apply(&mut next, command)?;
        }
        self.commit(next)
    }
    pub fn undo(&mut self) -> Result<Project, String> {
        if let Some(prev) = self.undo.last().cloned() {
            write_atomic(&self.recovery_path, &prev)?;
            self.undo.pop();
            self.redo.push(self.project.clone());
            bound_history(&mut self.redo);
            self.project = prev;
        }
        Ok(self.get())
    }
    pub fn redo(&mut self) -> Result<Project, String> {
        if let Some(next) = self.redo.last().cloned() {
            write_atomic(&self.recovery_path, &next)?;
            self.redo.pop();
            self.undo.push(self.project.clone());
            bound_history(&mut self.undo);
            self.project = next;
        }
        Ok(self.get())
    }
    pub fn save(&mut self, path: &PathBuf) -> Result<Project, String> {
        let absolute = absolute(path)?;
        let parent = absolute.parent().ok_or("Invalid project path")?;
        if !parent.is_dir() {
            return Err("Project destination folder does not exist".into());
        }
        let base = media::normalize_path(parent.canonicalize().map_err(|e| e.to_string())?);
        let mut serialized = self.get();
        for m in &mut serialized.media {
            m.path = relative(Path::new(&m.path), &base)
                .to_string_lossy()
                .to_string();
            m.thumbnail = None;
            if let Some(ref proxy) = m.proxy {
                m.proxy = Some(
                    relative(Path::new(proxy), &base)
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
        // Both representations must fit before either valid file is replaced.
        // Relative save-as paths can grow even when the current autosave fits.
        let document_bytes = encode_project(&serialized)?;
        let recovery_bytes = encode_project(&self.project)?;
        write_bytes_atomic(&absolute, &document_bytes)?;
        write_bytes_atomic(&self.recovery_path, &recovery_bytes)?;
        self.project_path = Some(absolute);
        Ok(self.get())
    }
    pub fn open(&mut self, path: &PathBuf) -> Result<Project, String> {
        let absolute = absolute(path)?;
        let next = read_project(&absolute)?;
        self.finish_open(absolute, next)
    }
    /// Prepare the complete opened project before replacing the current project or autosave.
    pub fn open_hydrated(&mut self, path: &PathBuf, ctx: &MediaContext) -> Result<Project, String> {
        let absolute = absolute(path)?;
        let mut next = read_project(&absolute)?;
        hydrate_project_assets(&mut next, ctx)?;
        self.finish_open(absolute, next)
    }
    fn finish_open(&mut self, absolute: PathBuf, next: Project) -> Result<Project, String> {
        write_atomic(&self.recovery_path, &next)?;
        self.project = next;
        self.undo.clear();
        self.redo.clear();
        self.project_path = Some(absolute);
        Ok(self.get())
    }
    pub fn recovery_available(&self) -> bool {
        self.recovery_path.is_file() && read_recoverable_project(&self.recovery_path).is_ok()
    }
    pub fn recover(&mut self) -> Result<Project, String> {
        let next = read_recoverable_project(&self.recovery_path)?;
        self.project = next;
        self.undo.clear();
        self.redo.clear();
        self.project_path = None;
        Ok(self.get())
    }
    /// Recovery migration is prepared and bounded before active state changes.
    pub fn recover_hydrated(&mut self, ctx: &MediaContext) -> Result<Project, String> {
        let mut next = read_recoverable_project(&self.recovery_path)?;
        hydrate_project_assets(&mut next, ctx)?;
        write_atomic(&self.recovery_path, &next)?;
        self.project = next;
        self.undo.clear();
        self.redo.clear();
        self.project_path = None;
        Ok(self.get())
    }
    pub fn set_proxy(&mut self, media_id: &str, path: PathBuf) -> Result<Project, String> {
        let mut next = self.get();
        let m = next
            .media
            .iter_mut()
            .find(|m| m.id == media_id)
            .ok_or("Media for proxy is no longer in the project")?;
        m.proxy = Some(path.to_string_lossy().to_string());
        m.proxy_timing_version = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.ends_with("-proxy-v3.mp4"))
            .map(|_| NORMALIZED_TIMING_VERSION);
        write_atomic(&self.recovery_path, &next)?;
        self.project = next;
        Ok(self.get())
    }
    /// Regenerate only runtime bin assets after reopening; preserves project IDs, trims, bins and proxies.
    pub fn hydrate_assets(&mut self, ctx: &MediaContext) -> Result<Project, String> {
        let mut next = self.get();
        hydrate_project_assets(&mut next, ctx)?;
        write_atomic(&self.recovery_path, &next)?;
        self.project = next;
        Ok(self.get())
    }
}
fn read_recoverable_project(path: &Path) -> Result<Project, String> {
    let project = read_project(path)?;
    // Availability and recovery share this check. Compact external JSON or
    // resolved paths can exceed the canonical budget despite bounded input.
    encode_project(&project)?;
    Ok(project)
}
fn hydrate_project_assets(project: &mut Project, ctx: &MediaContext) -> Result<(), String> {
    for m in &mut project.media {
        if !m.missing {
            let asset = media::import(ctx, Path::new(&m.path))?;
            // Compare retained provenance before replacing timing/geometry.
            m.legacy_source = legacy_compatibility(m, &asset, ctx, false)?;
            m.width = asset.width;
            m.height = asset.height;
            // Keep saved duration and edits, including legacy projects whose
            // old container duration interpretation included an absolute origin.
            m.timing = asset.timing;
            if !media::proxy_is_current(m) {
                m.proxy = None;
                m.proxy_timing_version = None;
            }
            m.thumbnail = asset.thumbnail;
            m.waveform = asset.waveform;
        }
    }
    Ok(())
}
fn same_time(a: Rational, b: Rational) -> bool {
    a.checked_sub(b)
        .map(|difference| difference.num == 0)
        .unwrap_or(false)
}
fn source_signature_matches(saved: &Media, actual: &Media) -> bool {
    let geometry = saved.kind == actual.kind
        && saved.width == actual.width
        && saved.height == actual.height
        && saved.has_audio == actual.has_audio
        && same_time(saved.fps, actual.fps);
    if !geometry {
        return false;
    }
    match (&saved.timing, &actual.timing) {
        (None, Some(_)) => true,
        (Some(a), Some(b)) => {
            same_time(a.origin, b.origin)
                && same_optional_time(a.video_start, b.video_start)
                && same_optional_time(a.audio_start, b.audio_start)
                && a.video_end
                    .map(|end| {
                        b.video_end
                            .map(|value| same_time(end, value))
                            .unwrap_or(false)
                    })
                    .unwrap_or(true)
                && a.audio_end
                    .map(|end| {
                        b.audio_end
                            .map(|value| same_time(end, value))
                            .unwrap_or(false)
                    })
                    .unwrap_or(true)
                && a.video_stream
                    .map(|index| Some(index) == b.video_stream)
                    .unwrap_or(true)
                && a.audio_stream
                    .map(|index| Some(index) == b.audio_stream)
                    .unwrap_or(true)
        }
        _ => false,
    }
}
fn same_optional_time(a: Option<Rational>, b: Option<Rational>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_time(a, b),
        (None, None) => true,
        _ => false,
    }
}
fn historical_duration_matches(saved: &Media, actual: &Media) -> bool {
    let Some(timing) = &actual.timing else {
        return false;
    };
    if saved.kind == "image"
        || timing.origin.num <= 0
        || saved
            .duration
            .checked_sub(actual.duration)
            .map(|difference| difference.num <= 0)
            .unwrap_or(true)
    {
        return false;
    }
    // Pre-timing v1 preferred format.duration. Its decimal precision may differ
    // from nanosecond stream-end tags, so compare that exact historic value.
    let historical = timing
        .container_duration
        .or_else(|| timing.origin.checked_add(actual.duration).ok());
    historical
        .map(|duration| same_time(saved.duration, duration))
        .unwrap_or(false)
}
fn legacy_compatibility(
    saved: &Media,
    actual: &Media,
    ctx: &MediaContext,
    relinking: bool,
) -> Result<Option<LegacySourceCompatibility>, String> {
    let recorded = saved.legacy_source.as_ref();
    // New imports carry the container-duration provenance field. Only older
    // records (including the earlier timing-only migration) can infer a tail.
    if recorded.is_none()
        && saved
            .timing
            .as_ref()
            .map(|timing| timing.container_duration.is_some())
            .unwrap_or(false)
    {
        return Ok(None);
    }
    let matches = historical_duration_matches(saved, actual)
        && source_signature_matches(saved, actual)
        && recorded
            .map(|record| same_time(record.duration, actual.duration))
            .unwrap_or(true);
    if !matches {
        if recorded.is_some() {
            return Err("Replacement differs from the legacy source timing or geometry; existing edits and their saved tail were preserved".into());
        }
        return Ok(None);
    }
    let sha256 = media::content_sha256(ctx, Path::new(&actual.path))?;
    let expected = match recorded.and_then(|record| record.sha256.clone()) {
        Some(hash) => Some(hash),
        None if relinking && saved.path != actual.path && Path::new(&saved.path).is_file() => {
            Some(media::content_sha256(ctx, Path::new(&saved.path))?)
        }
        _ => None,
    };
    if expected
        .as_ref()
        .map(|hash| !hash.eq_ignore_ascii_case(&sha256))
        .unwrap_or(false)
    {
        return Err("Replacement content differs from the identified legacy source; existing edits and their saved tail were preserved".into());
    }
    Ok(Some(LegacySourceCompatibility {
        duration: actual.duration,
        sha256: Some(sha256),
    }))
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|p| p.join(path))
            .map_err(|e| e.to_string())
    }
}
fn relative(path: &Path, base: &Path) -> PathBuf {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let mut common = 0;
    while common < p.len() && common < b.len() && p[common] == b[common] {
        common += 1;
    }
    if common == 0 {
        return path.to_path_buf();
    }
    let mut result = PathBuf::new();
    for comp in &b[common..] {
        if matches!(comp, Component::Normal(_)) {
            result.push("..");
        }
    }
    for comp in &p[common..] {
        result.push(comp.as_os_str());
    }
    result
}
pub fn write_atomic(path: &Path, project: &Project) -> Result<(), String> {
    // Validate and bound actual encoded bytes before creating a folder/temp
    // file or replacing an existing readable project/recovery.
    let bytes = encode_project(project)?;
    write_bytes_atomic(path, &bytes)
}
fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Cannot create project folder: {e}"))?;
    }
    let tmp = path.with_extension(format!("{}.tmp", id()));
    let result = (|| {
        let mut file = fs::File::create(&tmp).map_err(|e| format!("Cannot write project: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("Cannot write project: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Cannot sync project: {e}"))?;
        drop(file);
        fs::rename(&tmp, path).map_err(|e| format!("Cannot save project: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
pub fn read_project(path: &Path) -> Result<Project, String> {
    let file = fs::File::open(path).map_err(|e| format!("Cannot open project: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("Cannot inspect project: {e}"))?;
    if meta.len() > MAX_PROJECT_BYTES as u64 {
        return Err(size_error());
    }
    // Bound the actual read too: the file may grow after its metadata check.
    let mut bytes = Vec::new();
    file.take(MAX_PROJECT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read project: {e}"))?;
    if bytes.len() > MAX_PROJECT_BYTES {
        return Err(size_error());
    }
    let mut p: Project =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid project file: {e}"))?;
    p.validate()?;
    let base = path.parent().unwrap_or(Path::new("."));
    for m in &mut p.media {
        let source = Path::new(&m.path);
        let source = if source.is_absolute() {
            source.to_path_buf()
        } else {
            base.join(source)
        };
        let resolved = media::normalize_path(source.canonicalize().unwrap_or(source));
        m.missing = !resolved.is_file();
        m.path = resolved.to_string_lossy().to_string();
        if let Some(ref proxy) = m.proxy {
            let candidate = Path::new(proxy);
            let resolved = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                base.join(candidate)
            };
            m.proxy = if resolved.is_file()
                && m.proxy_timing_version == Some(NORMALIZED_TIMING_VERSION)
            {
                Some(resolved.to_string_lossy().to_string())
            } else {
                None
            };
        }
        if m.proxy.is_none() {
            m.proxy_timing_version = None;
        }
        if m.thumbnail
            .as_ref()
            .map(|t| !Path::new(t).is_file())
            .unwrap_or(false)
        {
            m.thumbnail = None;
        }
    }
    Ok(p)
}
