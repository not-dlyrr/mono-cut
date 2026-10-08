//! Render identity and validated, bounded program-preview records.
use crate::{
    media::{self, MediaContext},
    model::*,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const RECIPE: &str = "program-preview-v1";
pub const MAX_ENTRIES: usize = 32;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileStamp {
    pub path: String,
    pub bytes: u64,
    pub modified_ns: u128,
    pub created_ns: Option<u128>,
}
impl FileStamp {
    pub fn read(path: &Path) -> Result<Self, String> {
        let meta =
            fs::metadata(path).map_err(|e| format!("Cannot inspect '{}': {e}", path.display()))?;
        if !meta.is_file() {
            return Err("Media reference is not a file".into());
        }
        Ok(Self {
            path: media::normalize_path(path.canonicalize().map_err(|e| e.to_string())?)
                .to_string_lossy()
                .into_owned(),
            bytes: meta.len(),
            modified_ns: meta
                .modified()
                .map_err(|e| e.to_string())?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            created_ns: meta
                .created()
                .ok()
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()),
        })
    }
}
pub fn settings(p: &Project, height: u32) -> Result<ExportSettings, String> {
    if !(120..=2160).contains(&height) {
        return Err("Preview height must be between 120 and 2160".into());
    }
    let height = height / 2 * 2;
    let width =
        ((height as f64 * p.width as f64 / p.height as f64 / 2.).round() as u32 * 2).max(16);
    let result = ExportSettings {
        width,
        height,
        fps: p.fps,
        codec: "h264".into(),
        crf: 22,
        audio_bitrate: 160,
        sample_rate: p.sample_rate,
    };
    crate::render::validate_settings(&result)?;
    Ok(result)
}
/// Preserve all non-metadata fields, including future additive render fields.
pub fn descriptor(p: &Project, use_proxies: bool) -> Value {
    let mut value = serde_json::to_value(p).expect("validated project serializes");
    let object = value.as_object_mut().unwrap();
    for field in ["id", "name", "bins", "markers", "in_point", "out_point"] {
        object.remove(field);
    }
    let tracks: HashSet<_> = p.clips.iter().map(|c| c.track_id.as_str()).collect();
    let media: HashSet<_> = p
        .clips
        .iter()
        .filter_map(|c| c.media_id.as_deref())
        .collect();
    let lanes = object["tracks"].as_array_mut().unwrap();
    lanes.retain(|t| tracks.contains(t["id"].as_str().unwrap_or("")));
    for track in lanes {
        let t = track.as_object_mut().unwrap();
        t.remove("name");
        t.remove("locked");
    }
    let sources = object["media"].as_array_mut().unwrap();
    sources.retain(|m| media.contains(m["id"].as_str().unwrap_or("")));
    for source in sources.iter_mut() {
        let m = source.as_object_mut().unwrap();
        for f in ["name", "bin_id", "thumbnail", "waveform"] {
            m.remove(f);
        }
        if !use_proxies {
            m.remove("proxy");
            m.remove("proxy_timing_version");
        }
    }
    sources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    let clips = object["clips"].as_array_mut().unwrap();
    for clip in clips.iter_mut() {
        let c = clip.as_object_mut().unwrap();
        c.remove("name");
        c.remove("linked_id");
    }
    clips.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    value
}
pub fn source_paths(p: &Project, use_proxies: bool) -> Vec<PathBuf> {
    let used: HashSet<_> = p
        .clips
        .iter()
        .filter_map(|c| c.media_id.as_deref())
        .collect();
    let mut paths = vec![];
    for m in p.media.iter().filter(|m| used.contains(m.id.as_str())) {
        paths.push(PathBuf::from(&m.path));
        if use_proxies && media::proxy_is_current(m) {
            if let Some(path) = &m.proxy {
                paths.push(PathBuf::from(path));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
pub fn identity(
    ctx: &MediaContext,
    p: &Project,
    height: u32,
    use_proxies: bool,
) -> Result<String, String> {
    p.validate()?;
    let settings = settings(p, height)?;
    let stamps: Vec<_> = source_paths(p, use_proxies)
        .iter()
        .map(|path| FileStamp::read(path).ok())
        .collect();
    let font = if p.clips.iter().any(|c| c.title.is_some()) {
        FileStamp::read(&ctx.font).ok()
    } else {
        None
    };
    let value = serde_json::json!({"recipe":RECIPE,"project":descriptor(p,use_proxies),"settings":settings,"proxies":use_proxies,"sources":stamps,"font":font,"ffmpeg":FileStamp::read(&ctx.ffmpeg).ok()});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(|e| e.to_string())?)
    ))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub recipe: String,
    pub key: String,
    pub file: String,
    pub stamp: FileStamp,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub sample_rate: u32,
    pub frames: i64,
    pub duration: Rational,
}
fn manifest_path(ctx: &MediaContext, key: &str) -> PathBuf {
    ctx.cache_dir.join(format!("preview-{key}.json"))
}
pub fn touch(ctx: &MediaContext, key: &str) {
    if let Ok(file) = fs::OpenOptions::new()
        .write(true)
        .open(manifest_path(ctx, key))
    {
        let _ = file.set_modified(SystemTime::now());
    }
}
pub fn discard(ctx: &MediaContext, key: &str, output: &Path) {
    let path = manifest_path(ctx, key);
    if load(&path).is_some_and(|m| {
        output
            .file_name()
            .is_some_and(|name| name == m.file.as_str())
    }) {
        let _ = ctx.processes.remove_unprotected_file(path);
    }
}
fn load(path: &Path) -> Option<Manifest> {
    let mut bytes = vec![];
    fs::File::open(path)
        .ok()?
        .take(8193)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 8192 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
pub fn cached(ctx: &MediaContext, p: &Project, height: u32, key: &str) -> Option<PathBuf> {
    let manifest = load(&manifest_path(ctx, key))?;
    let settings = settings(p, height).ok()?;
    if manifest.recipe != RECIPE
        || manifest.key != key
        || manifest.width != settings.width
        || manifest.height != settings.height
        || manifest.fps != settings.fps
        || manifest.sample_rate != settings.sample_rate
        || manifest.frames != p.length()
        || manifest.duration != Rational::from_frames(p.length(), p.fps)
    {
        return None;
    }
    if !manifest.file.starts_with(&format!("preview-{key}-"))
        || !manifest.file.ends_with(".mp4")
        || Path::new(&manifest.file).components().count() != 1
    {
        return None;
    }
    let path = ctx.cache_dir.join(&manifest.file);
    let _pin = ctx.processes.pin_file(&path).ok()?;
    if FileStamp::read(&path).ok()? != manifest.stamp || manifest.stamp.bytes == 0 {
        return None;
    }
    Some(path)
}
pub fn complete(
    ctx: &MediaContext,
    p: &Project,
    height: u32,
    use_proxies: bool,
    key: &str,
    path: &Path,
) -> Result<(), String> {
    if identity(ctx, p, height, use_proxies)? != key {
        return Err("Preview source changed while rendering; refresh the preview.".into());
    }
    let settings = settings(p, height)?;
    let (actual, details) = media::probe_details(ctx, path)?;
    let streams = details["streams"]
        .as_array()
        .ok_or("Preview output contains no streams")?;
    let video = streams.iter().find(|stream| {
        stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
    });
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio");
    let integer = |value: &Value| {
        value
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| value.as_u64())
    };
    let actual_frames = video.and_then(|stream| integer(&stream["nb_frames"]));
    let actual_sample_rate = audio.and_then(|stream| integer(&stream["sample_rate"]));
    let expected = Rational::from_frames(p.length(), p.fps);
    if actual.width != settings.width
        || actual.height != settings.height
        || actual.fps.num as i128 * settings.fps.den as i128
            != settings.fps.num as i128 * actual.fps.den as i128
        || !actual.has_audio
        || actual_frames != Some(p.length() as u64)
        || actual_sample_rate != Some(settings.sample_rate as u64)
        || (actual.duration.value() - expected.value()).abs() > 0.1
    {
        return Err("Preview output metadata does not match the sequence.".into());
    }
    let manifest = Manifest {
        recipe: RECIPE.into(),
        key: key.into(),
        file: path
            .file_name()
            .ok_or("Missing cache filename")?
            .to_string_lossy()
            .into_owned(),
        stamp: FileStamp::read(path)?,
        width: settings.width,
        height: settings.height,
        fps: settings.fps,
        sample_rate: settings.sample_rate,
        frames: p.length(),
        duration: expected,
    };
    let destination = manifest_path(ctx, key);
    let temporary = destination.with_extension("json.tmp");
    let _guard = ctx
        .processes
        .temporary_file(temporary.clone())
        .map_err(|e| e.to_string())?;
    fs::write(
        &temporary,
        serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&temporary, &destination).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn prune(ctx: &MediaContext) {
    let Ok(dir) = fs::read_dir(&ctx.cache_dir) else {
        return;
    };
    let mut records = HashMap::new();
    let mut outputs = vec![];
    for entry in dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("preview-") {
            continue;
        }
        if name.ends_with(".mp4") {
            if let Ok(meta) = entry.metadata() {
                outputs.push((path, meta.len(), meta.modified().unwrap_or(UNIX_EPOCH)));
            }
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        let Some(record) = load(&path) else {
            let _ = ctx.processes.remove_unprotected_file(&path);
            continue;
        };
        if Path::new(&record.file).components().count() != 1
            || !record.file.starts_with("preview-")
            || !record.file.ends_with(".mp4")
        {
            let _ = ctx.processes.remove_unprotected_file(&path);
            continue;
        }
        let output = ctx.cache_dir.join(&record.file);
        if !output.is_file() && !ctx.processes.is_protected(&output) {
            let _ = ctx.processes.remove_unprotected_file(&path);
            continue;
        }
        records.insert(
            output,
            (
                entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(UNIX_EPOCH),
                path,
            ),
        );
    }
    let mut total = outputs.iter().map(|(_, bytes, _)| *bytes).sum::<u64>();
    let mut entries = vec![];
    for (output, bytes, modified) in outputs {
        if let Some((accessed, manifest)) = records.remove(&output) {
            entries.push((accessed, Some(manifest), output, bytes));
        } else if ctx
            .processes
            .remove_unprotected_file(&output)
            .unwrap_or(false)
        {
            // Repaired/replaced manifests must not leave unindexed files outside the budget.
            total = total.saturating_sub(bytes);
        } else {
            entries.push((modified, None, output, bytes));
        }
    }
    entries.sort_by_key(|e| e.0);
    let mut count = entries.iter().filter(|e| e.1.is_some()).count();
    for (_, manifest, output, bytes) in entries {
        if count <= MAX_ENTRIES && total <= MAX_BYTES {
            break;
        }
        if ctx.processes.is_protected(&output) {
            continue;
        }
        if ctx
            .processes
            .remove_unprotected_file(&output)
            .unwrap_or(false)
            || !output.exists()
        {
            if let Some(manifest) = manifest {
                let _ = ctx.processes.remove_unprotected_file(&manifest);
                count = count.saturating_sub(1);
            }
            total = total.saturating_sub(bytes);
        }
    }
}
