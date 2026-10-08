//! An initial packet-clock check before optimized video input seeking.
//!
//! Some accepted containers omit early packet PTS. A decoder from the physical
//! beginning reconstructs their presentation clock, while demuxer seeking can
//! lose the initial GOP completely. Those sources retain the full-render clock
//! by reading a prefix and stopping at the existing exact filter EOF. This is
//! an initial 256-packet assessment, not certification of all later timestamps.
use crate::{media, media::MediaContext, preview::FileStamp};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

pub const RECIPE: &str = "video-initial-packet-clock-v1";
pub const SAMPLE_PACKETS: usize = 256;
pub const MAX_RECORDS: usize = 128;
pub const MAX_CACHE_BYTES: usize = 16 * 1024;
const MAX_PROBE_BYTES: usize = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InitialClock {
    /// The sampled packets have PTS. Normal B-frame PTS reordering is allowed.
    SampledPtsPresent,
    /// A sampled packet lacks usable PTS; preserve the no-seek decoder clock.
    MissingPacketPts,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Inspection {
    pub recipe: &'static str,
    pub identity_key: String,
    pub selected_stream: String,
    pub packet_limit: usize,
    pub sampled_packets: usize,
    pub missing_pts: usize,
    pub first_missing_packet: Option<usize>,
    pub clock: InitialClock,
    pub cache_hit: bool,
    /// Includes source/tool metadata, packet inspection and cache admission.
    pub inspection_ms: f64,
}
impl Inspection {
    pub fn needs_prefix(&self) -> bool {
        self.clock == InitialClock::MissingPacketPts
    }
}

#[derive(Clone, Copy, Debug)]
struct InitialPts {
    sampled_packets: usize,
    missing_pts: usize,
    first_missing_packet: Option<usize>,
}
type CacheKey = [u8; 32];
type Entry = (CacheKey, InitialPts);
#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
}
impl Cache {
    fn get(&mut self, key: &CacheKey) -> Option<InitialPts> {
        let i = self.entries.iter().position(|entry| &entry.0 == key)?;
        let entry = self.entries.remove(i)?;
        self.entries.push_back(entry);
        Some(entry.1)
    }
    fn insert(&mut self, key: CacheKey, value: InitialPts) {
        if let Some(i) = self.entries.iter().position(|entry| entry.0 == key) {
            self.entries.remove(i);
        }
        // Entries contain only fixed-size hashes/counts, never source paths,
        // packet payloads or media. Both retained records and storage are capped.
        while self.entries.len() >= MAX_RECORDS
            || (self.entries.len() + 1) * std::mem::size_of::<Entry>() > MAX_CACHE_BYTES
        {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value));
    }
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// Resolve a PATH-provided tool before stamping and launching that exact file.
/// Bundled/absolute paths are used directly; a changed PATH resolution changes
/// preview identity as well as the packet-assessment cache key.
pub fn tool_path(ctx: &MediaContext) -> Result<PathBuf, String> {
    if ctx.ffprobe.is_file() {
        return Ok(ctx.ffprobe.clone());
    }
    if ctx.ffprobe.components().count() == 1 {
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths) {
                let candidate = directory.join(&ctx.ffprobe);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                #[cfg(windows)]
                if candidate.extension().is_none() {
                    let executable = candidate.with_extension("exe");
                    if executable.is_file() {
                        return Ok(executable);
                    }
                }
            }
        }
    }
    Err("Cannot locate FFprobe for the initial video timestamp check".into())
}
pub fn tool_stamp(ctx: &MediaContext) -> Result<FileStamp, String> {
    FileStamp::read(&tool_path(ctx)?)
}
fn ensure_live(ctx: &MediaContext, cancel: Option<&Arc<AtomicBool>>) -> Result<(), String> {
    if ctx.processes.is_closing() || cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err("Video timestamp inspection cancelled".into())
    } else {
        Ok(())
    }
}

fn packet_probe(
    ctx: &MediaContext,
    tool: &Path,
    source: &Path,
    stream: &str,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<InitialPts, String> {
    ensure_live(ctx, cancel)?;
    let mut command = media::command(tool);
    command
        .args([
            "-v",
            "error",
            "-select_streams",
            stream,
            "-read_intervals",
            &format!("%+#{SAMPLE_PACKETS}"),
            "-show_packets",
            "-show_entries",
            "packet=pts",
            "-of",
            "json",
        ])
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ctx
        .processes
        .spawn(&mut command)
        .map_err(|e| e.to_string())?;
    let stdout = child
        .take_stdout()
        .ok_or("Video timestamp stdout unavailable")?;
    let mut stderr = child
        .take_stderr()
        .ok_or("Video timestamp stderr unavailable")?;
    let oversized = Arc::new(AtomicBool::new(false));
    let overflow = oversized.clone();
    let out_reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        let result = stdout
            .take((MAX_PROBE_BYTES + 1) as u64)
            .read_to_end(&mut bytes);
        if bytes.len() > MAX_PROBE_BYTES {
            overflow.store(true, Ordering::Release);
        }
        result.map(|_| bytes)
    });
    let err_reader = std::thread::spawn(move || {
        let mut tail = vec![];
        let mut block = [0u8; 4096];
        loop {
            let n = stderr.read(&mut block)?;
            if n == 0 {
                break;
            }
            tail.extend_from_slice(&block[..n]);
            if tail.len() > MAX_PROBE_BYTES {
                tail.drain(..tail.len() - MAX_PROBE_BYTES);
            }
        }
        Ok::<_, std::io::Error>(tail)
    });
    let started = Instant::now();
    let status = loop {
        if let Err(error) = ensure_live(ctx, cancel) {
            break Err(error);
        }
        if oversized.load(Ordering::Acquire) {
            break Err("Video timestamp inspection exceeded its 64 KiB output budget".into());
        }
        if started.elapsed() > PROBE_TIMEOUT {
            break Err("Video timestamp inspection timed out; retry or relink the source".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => break Err(error.to_string()),
        }
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Readers always join after normal EOF or kill/reap, including cancellation,
    // malformed output and budgets. No classification sidecar is ever written.
    let stdout = out_reader
        .join()
        .map_err(|_| "Video timestamp reader interrupted".to_string())
        .and_then(|result| result.map_err(|e| e.to_string()));
    let stderr = err_reader
        .join()
        .map_err(|_| "Video timestamp error reader interrupted".to_string())
        .and_then(|result| result.map_err(|e| e.to_string()));
    ensure_live(ctx, cancel)?;
    let status = status?;
    let stdout = stdout?;
    let stderr = stderr?;
    if stdout.len() > MAX_PROBE_BYTES {
        return Err("Video timestamp inspection exceeded its 64 KiB output budget".into());
    }
    if !status.success() {
        return Err(format!(
            "Video timestamp inspection failed: {}",
            media::diagnostic(&stderr)
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&stdout)
        .map_err(|e| format!("Invalid video timestamp inspection: {e}"))?;
    let packets = value["packets"]
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= SAMPLE_PACKETS)
        .ok_or("Video timestamp inspection returned no usable packet sample")?;
    let missing: Vec<_> = packets
        .iter()
        .enumerate()
        .filter_map(|(i, packet)| packet["pts"].as_i64().is_none().then_some(i))
        .collect();
    Ok(InitialPts {
        sampled_packets: packets.len(),
        missing_pts: missing.len(),
        first_missing_packet: missing.first().copied(),
    })
}

pub fn inspect(
    ctx: &MediaContext,
    source: &Path,
    selected_stream: &str,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Inspection, String> {
    let started = Instant::now();
    ensure_live(ctx, cancel)?;
    let tool = tool_path(ctx)?;
    let source_stamp = FileStamp::read(source)?;
    let assessor_stamp = FileStamp::read(&tool)?;
    let descriptor = serde_json::json!({
        "recipe":RECIPE, "source":source_stamp, "stream":selected_stream,
        "ffprobe":assessor_stamp, "packet_limit":SAMPLE_PACKETS,
    });
    let key: CacheKey =
        Sha256::digest(serde_json::to_vec(&descriptor).map_err(|e| e.to_string())?).into();
    let _source_pin = ctx.processes.pin_file(source).map_err(|e| e.to_string())?;
    let _tool_pin = ctx.processes.pin_file(&tool).map_err(|e| e.to_string())?;
    let cached = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key);
    let result = match cached {
        Some(value) => value,
        None => packet_probe(ctx, &tool, source, selected_stream, cancel)?,
    };
    ensure_live(ctx, cancel)?;
    if FileStamp::read(source)? != source_stamp || FileStamp::read(&tool)? != assessor_stamp {
        return Err("PREVIEW_IDENTITY_CHANGED: Video source or timestamp assessor changed during inspection".into());
    }
    // This is the final admission point. A cancellation error must not commit
    // a new assessment; no fallible/live checks occur after cache insertion.
    ensure_live(ctx, cancel)?;
    if cached.is_none() {
        cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, result);
    }
    Ok(Inspection {
        recipe: RECIPE,
        identity_key: key.iter().map(|byte| format!("{byte:02x}")).collect(),
        selected_stream: selected_stream.into(),
        packet_limit: SAMPLE_PACKETS,
        sampled_packets: result.sampled_packets,
        missing_pts: result.missing_pts,
        first_missing_packet: result.first_missing_packet,
        clock: if result.missing_pts > 0 {
            InitialClock::MissingPacketPts
        } else {
            InitialClock::SampledPtsPresent
        },
        cache_hit: cached.is_some(),
        inspection_ms: started.elapsed().as_secs_f64() * 1000.,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_lru_caps_records_and_storage_and_retains_recent_use() {
        let mut cache = Cache::default();
        let value = InitialPts {
            sampled_packets: 256,
            missing_pts: 0,
            first_missing_packet: None,
        };
        for i in 0..MAX_RECORDS {
            let mut key = [0; 32];
            key[..8].copy_from_slice(&(i as u64).to_le_bytes());
            cache.insert(key, value);
        }
        let mut recent = [0; 32];
        recent[..8].copy_from_slice(&0u64.to_le_bytes());
        assert!(cache.get(&recent).is_some());
        let mut extra = [0; 32];
        extra[..8].copy_from_slice(&(MAX_RECORDS as u64).to_le_bytes());
        cache.insert(extra, value);
        let mut oldest = [0; 32];
        oldest[..8].copy_from_slice(&1u64.to_le_bytes());
        assert!(cache.get(&oldest).is_none());
        assert!(cache.get(&recent).is_some());
        for i in MAX_RECORDS + 1..MAX_RECORDS * 10 {
            let mut key = [0; 32];
            key[..8].copy_from_slice(&(i as u64).to_le_bytes());
            cache.insert(key, value);
        }
        assert!(cache.entries.len() <= MAX_RECORDS);
        assert!(cache.entries.capacity() * std::mem::size_of::<Entry>() <= MAX_CACHE_BYTES);
    }
}
