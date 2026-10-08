use crate::{
    model::*,
    processes::{AssetPin, ProcessRegistry},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct MediaContext {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub cache_dir: PathBuf,
    pub font: PathBuf,
    pub processes: Arc<ProcessRegistry>,
}
/// Windows canonicalization uses verbatim prefixes; ordinary paths work with WebView and relative projects.
pub fn normalize_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(unc) = s.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{unc}"));
        }
        if let Some(plain) = s.strip_prefix("\\\\?\\") {
            return PathBuf::from(plain);
        }
    }
    path
}
pub fn command(path: &Path) -> Command {
    let mut c = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
pub fn ensure_cache(ctx: &MediaContext) -> Result<(), String> {
    fs::create_dir_all(&ctx.cache_dir).map_err(|e| format!("Could not create media cache: {e}"))
}
pub fn diagnostic(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.chars()
        .rev()
        .take(6000)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}
fn managed_output(ctx: &MediaContext, command: &mut Command) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ctx.processes.spawn(command).map_err(|e| e.to_string())?;
    let mut stdout = child.take_stdout().ok_or("Media stdout unavailable")?;
    let mut stderr = child.take_stderr().ok_or("Media stderr unavailable")?;
    let out_reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        stdout.read_to_end(&mut bytes).map(|_| bytes)
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
            if tail.len() > 65536 {
                tail.drain(..tail.len() - 65536);
            }
        }
        Ok::<_, std::io::Error>(tail)
    });
    let status = child.wait();
    if status.is_err() {
        let _ = child.kill();
    }
    let stdout = out_reader
        .join()
        .map_err(|_| "Media stdout reader interrupted")?
        .map_err(|e| e.to_string())?;
    let stderr = err_reader
        .join()
        .map_err(|_| "Media stderr reader interrupted")?
        .map_err(|e| e.to_string())?;
    if ctx.processes.is_closing() {
        return Err("Media processing cancelled: application is closing".into());
    }
    Ok(Output {
        status: status.map_err(|e| e.to_string())?,
        stdout,
        stderr,
    })
}
pub fn run_ffmpeg(ctx: &MediaContext, args: &[String]) -> Result<(), String> {
    let mut process = command(&ctx.ffmpeg);
    process
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args);
    let output =
        managed_output(ctx, &mut process).map_err(|e| format!("Could not run FFmpeg: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("FFmpeg failed: {}", diagnostic(&output.stderr)))
    }
}
pub fn decimal_rational(s: &str) -> Option<Rational> {
    if let Some((n, d)) = s.split_once('/') {
        let n = n.parse::<i64>().ok()?;
        let d = d.parse::<i64>().ok()?;
        if d > 0 {
            let rational = Rational::new(n, d);
            rational.validate(false).ok()?;
            return Some(rational);
        }
        return None;
    }
    // FFprobe decimal fields are decimal strings, not binary floats. Keep all
    // declared digits (up to the project's nanosecond denominator limit).
    let (negative, unsigned) = if let Some(value) = s.strip_prefix('-') {
        (true, value)
    } else {
        (false, s.strip_prefix('+').unwrap_or(s))
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let den = 10_i64.pow(fraction.len() as u32);
    let magnitude =
        whole
            .parse::<i64>()
            .ok()?
            .checked_mul(den)?
            .checked_add(if fraction.is_empty() {
                0
            } else {
                fraction.parse::<i64>().ok()?
            })?;
    let rational = Rational::new(if negative { -magnitude } else { magnitude }, den);
    rational.validate(false).ok()?;
    Some(rational)
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| {
        v.as_str()
            .map(str::to_owned)
            .or_else(|| v.as_f64().map(|n| n.to_string()))
    })
}
fn earlier(a: Rational, b: Rational) -> Rational {
    if (a.num as i128 * b.den as i128) < (b.num as i128 * a.den as i128) {
        a
    } else {
        b
    }
}
fn later(a: Rational, b: Rational) -> Rational {
    if (a.num as i128 * b.den as i128) > (b.num as i128 * a.den as i128) {
        a
    } else {
        b
    }
}
fn stream_start(stream: &Value, fallback: Rational) -> Rational {
    stream["start_pts"]
        .as_i64()
        .and_then(|pts| {
            let base = string(stream, "time_base").and_then(|v| decimal_rational(&v))?;
            base.checked_mul(Rational::new(pts, 1)).ok()
        })
        .or_else(|| string(stream, "start_time").and_then(|v| decimal_rational(&v)))
        .unwrap_or(fallback)
}
fn timestamp_rational(value: &str) -> Option<Rational> {
    let mut fields = value.split(':');
    let hours = fields.next()?.parse::<i64>().ok()?;
    let minutes = fields.next()?.parse::<i64>().ok()?;
    let seconds = decimal_rational(fields.next()?)?;
    if fields.next().is_some()
        || hours < 0
        || !(0..60).contains(&minutes)
        || seconds.num < 0
        || seconds.value() >= 60.
    {
        return None;
    }
    Rational::new(hours.checked_mul(3600)?.checked_add(minutes * 60)?, 1)
        .checked_add(seconds)
        .ok()
}
fn stream_end(stream: &Value, start: Rational, matroska: bool) -> Option<Rational> {
    let duration = stream["duration_ts"]
        .as_i64()
        .and_then(|ticks| {
            let base = string(stream, "time_base").and_then(|v| decimal_rational(&v))?;
            base.checked_mul(Rational::new(ticks, 1)).ok()
        })
        .or_else(|| string(stream, "duration").and_then(|v| decimal_rational(&v)))
        .filter(|value| value.num > 0);
    if let Some(duration) = duration {
        return start.checked_add(duration).ok();
    }
    // Matroska's DURATION tag and format duration are timeline end timestamps.
    // MP4 stream duration_ts, in contrast, measures elapsed duration.
    if matroska {
        return string(&stream["tags"], "DURATION").and_then(|v| timestamp_rational(&v));
    }
    None
}
fn probe_timing(data: &Value, video: Option<&Value>, audio: Option<&Value>) -> SourceTiming {
    let fallback = string(&data["format"], "start_time")
        .and_then(|v| decimal_rational(&v))
        .unwrap_or_default();
    let video_start = video.map(|stream| stream_start(stream, fallback));
    let audio_start = audio.map(|stream| stream_start(stream, fallback));
    let matroska = string(&data["format"], "format_name")
        .map(|v| {
            v.split(',')
                .any(|name| name == "matroska" || name == "webm")
        })
        .unwrap_or(false);
    let origin = video_start
        .into_iter()
        .chain(audio_start)
        .reduce(earlier)
        .unwrap_or_default();
    SourceTiming {
        origin,
        container_duration: string(&data["format"], "duration")
            .and_then(|value| decimal_rational(&value))
            .filter(|duration| duration.num >= 0),
        video_start,
        audio_start,
        video_end: video
            .and_then(|stream| stream_end(stream, video_start.unwrap_or(origin), matroska)),
        audio_end: audio
            .and_then(|stream| stream_end(stream, audio_start.unwrap_or(origin), matroska)),
        video_stream: video
            .and_then(|stream| stream["index"].as_u64())
            .and_then(|v| u32::try_from(v).ok()),
        audio_stream: audio
            .and_then(|stream| stream["index"].as_u64())
            .and_then(|v| u32::try_from(v).ok()),
    }
}
fn probe_duration(
    data: &Value,
    video: Option<&Value>,
    audio: Option<&Value>,
    timing: &SourceTiming,
) -> Option<Rational> {
    let matroska = string(&data["format"], "format_name")
        .map(|v| {
            v.split(',')
                .any(|name| name == "matroska" || name == "webm")
        })
        .unwrap_or(false);
    let ends: Vec<_> = video
        .into_iter()
        .chain(audio)
        .map(|stream| stream_end(stream, stream_start(stream, timing.origin), matroska))
        .collect();
    let known_end = ends.iter().flatten().copied().reduce(later);
    let end = if ends.iter().all(Option::is_some) {
        known_end
    } else {
        let container = string(&data["format"], "duration")
            .and_then(|v| decimal_rational(&v))
            .filter(|v| v.num > 0)
            .and_then(|duration| {
                if matroska {
                    Some(duration)
                } else {
                    let start = string(&data["format"], "start_time")
                        .and_then(|v| decimal_rational(&v))
                        .unwrap_or(timing.origin);
                    start.checked_add(duration).ok()
                }
            });
        known_end.into_iter().chain(container).reduce(later)
    }?;
    end.checked_sub(timing.origin)
        .ok()
        .filter(|duration| duration.num > 0)
}
/// Display geometry after FFmpeg's quarter-turn autorotation and pixel-aspect correction.
fn display_dimensions(stream: &Value) -> (u32, u32) {
    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;
    let sar = string(stream, "sample_aspect_ratio")
        .and_then(|value| {
            let (num, den) = value.split_once(':')?;
            let ratio = num.parse::<f64>().ok()? / den.parse::<f64>().ok()?;
            (ratio.is_finite() && ratio > 0.).then_some(ratio)
        })
        .unwrap_or(1.);
    let display_width = (width as f64 * sar).round().clamp(0., u32::MAX as f64) as u32;
    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|entries| entries.iter().find_map(|entry| entry["rotation"].as_f64()))
        .or_else(|| string(&stream["tags"], "rotate").and_then(|value| value.parse::<f64>().ok()))
        .unwrap_or(0.)
        .rem_euclid(360.);
    if (rotation - 90.).abs() < 0.001 || (rotation - 270.).abs() < 0.001 {
        (height, display_width)
    } else {
        (display_width, height)
    }
}
/// Bounded square-pixel media caches keep the same display aspect as their source.
pub fn square_pixel_scale(max_width: u32, max_height: u32) -> String {
    let factor = format!("min(1,min({max_width}/(iw*sar),{max_height}/ih))");
    format!("scale=w='max(2,trunc(iw*sar*({factor})/2)*2)':h='max(2,trunc(ih*({factor})/2)*2)',setsar=1")
}
pub fn probe(ctx: &MediaContext, path: &Path) -> Result<Media, String> {
    probe_details(ctx, path).map(|(media, _)| media)
}
/// Retain the same single probe's stream details for completed-output verification.
pub fn probe_details(ctx: &MediaContext, path: &Path) -> Result<(Media, Value), String> {
    let _source_pin = ctx.processes.pin_file(path).map_err(|e| e.to_string())?;
    let path = normalize_path(
        path.canonicalize()
            .map_err(|e| format!("Cannot open media {}: {e}", path.display()))?,
    );
    if !path.is_file() {
        return Err("Media path must be a regular file".into());
    }
    let mut process = command(&ctx.ffprobe);
    process
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(&path);
    let output =
        managed_output(ctx, &mut process).map_err(|e| format!("Could not run FFprobe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not probe media: {}",
            diagnostic(&output.stderr)
        ));
    }
    let data: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid FFprobe result: {e}"))?;
    let streams = data["streams"]
        .as_array()
        .ok_or("Media contains no supported streams")?;
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    if video.is_none() && audio.is_none() {
        return Err("Media contains no supported video or audio".into());
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_image = ["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"].contains(&ext.as_str())
        && video.is_some();
    let kind = if is_image {
        "image"
    } else if video.is_some() {
        "video"
    } else {
        "audio"
    };
    let timing = probe_timing(&data, video, audio);
    let duration = probe_duration(&data, video, audio, &timing).unwrap_or(if is_image {
        Rational::new(5, 1)
    } else {
        Rational::zero()
    });
    if !is_image && duration.num <= 0 {
        return Err("Media has no finite duration; live streams are not supported".into());
    }
    let fps = video
        .and_then(|s| string(s, "avg_frame_rate").and_then(|x| decimal_rational(&x)))
        .filter(|r| r.num > 0)
        .or_else(|| {
            video.and_then(|s| string(s, "r_frame_rate").and_then(|x| decimal_rational(&x)))
        })
        .unwrap_or(Rational::new(0, 1));
    let (width, height) = video.map(display_dimensions).unwrap_or((0, 0));
    let media = Media {
        id: id(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        path: path.to_string_lossy().to_string(),
        kind: kind.into(),
        duration,
        fps,
        width,
        height,
        has_audio: audio.is_some(),
        bin_id: None,
        thumbnail: None,
        waveform: vec![],
        proxy: None,
        timing: Some(timing),
        proxy_timing_version: None,
        legacy_source: None,
        missing: false,
    };
    Ok((media, data))
}
pub fn cache_key(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let stamp = meta
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut h = Sha256::new();
    h.update(path.to_string_lossy().as_bytes());
    h.update(meta.len().to_le_bytes());
    h.update(stamp.to_le_bytes());
    Ok(format!("{:x}", h.finalize()))
}
/// Full content identity is only needed for historical source-bound migration.
/// Stream it in bounded chunks on the caller's media worker, never in the UI.
pub fn content_sha256(ctx: &MediaContext, path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("Cannot identify source: {e}"))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        if ctx.processes.is_closing() {
            return Err("Media processing cancelled: application is closing".into());
        }
        let count = file
            .read(&mut block)
            .map_err(|e| format!("Cannot identify source: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&block[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn proxy_is_current(media: &Media) -> bool {
    if media.proxy_timing_version != Some(NORMALIZED_TIMING_VERSION) {
        return false;
    }
    let Some(proxy) = media
        .proxy
        .as_ref()
        .map(Path::new)
        .filter(|path| path.is_file())
    else {
        return false;
    };
    let Ok(key) = cache_key(Path::new(&media.path)) else {
        return false;
    };
    let expected = format!("{}-{key}-proxy-v3.mp4", media.id);
    proxy.file_name().and_then(|name| name.to_str()) == Some(expected.as_str())
}
pub fn import(ctx: &MediaContext, path: &Path) -> Result<Media, String> {
    ensure_cache(ctx)?;
    let _source_pin = ctx.processes.pin_file(path).map_err(|e| e.to_string())?;
    let mut asset_pins = Vec::new();
    let mut m = probe(ctx, path)?;
    let key = cache_key(Path::new(&m.path))?;
    if m.kind != "audio" {
        let thumb = ctx.cache_dir.join(format!("{key}-thumb-v3.jpg"));
        asset_pins.push(ctx.processes.pin_file(&thumb).map_err(|e| e.to_string())?);
        if !thumb.is_file() {
            let part = ctx.cache_dir.join(format!("{key}-thumb-part-{}.jpg", id()));
            let _temporary = ctx
                .processes
                .temporary_file(part.clone())
                .map_err(|e| e.to_string())?;
            run_ffmpeg(
                ctx,
                &[
                    "-i".into(),
                    m.path.clone(),
                    "-map".into(),
                    m.timing
                        .as_ref()
                        .and_then(|timing| timing.video_stream)
                        .map(|index| format!("0:{index}"))
                        .unwrap_or_else(|| "0:v:0".into()),
                    "-frames:v".into(),
                    "1".into(),
                    "-vf".into(),
                    square_pixel_scale(320, 180),
                    "-q:v".into(),
                    "3".into(),
                    part.to_string_lossy().to_string(),
                ],
            )?;
            fs::rename(&part, &thumb).map_err(|e| e.to_string())?;
        }
        m.thumbnail = Some(thumb.to_string_lossy().to_string());
    }
    if m.has_audio {
        let wave = ctx.cache_dir.join(format!("{key}-wave-v2.json"));
        asset_pins.push(ctx.processes.pin_file(&wave).map_err(|e| e.to_string())?);
        if wave.is_file() {
            m.waveform = fs::read(&wave)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
        }
        if m.waveform.is_empty() {
            m.waveform = waveform(ctx, &m)?;
            fs::write(
                wave,
                serde_json::to_vec(&m.waveform).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    prune_cache(ctx, 2 * 1024 * 1024 * 1024)?;
    Ok(m)
}
fn waveform(ctx: &MediaContext, m: &Media) -> Result<Vec<f32>, String> {
    let timing = m.timing.as_ref().ok_or("Source timing is unknown")?;
    let audio_index = timing
        .audio_stream
        .map(|index| format!("0:{index}"))
        .unwrap_or_else(|| "0:a:0".into());
    let filter = format!(
        "asetpts=PTS-({}/{})/TB,aresample=4000:first_pts=0,atrim=duration={:.12}",
        timing.origin.num,
        timing.origin.den,
        m.duration.value()
    );
    let mut process = command(&ctx.ffmpeg);
    process
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-copyts",
            "-i",
        ])
        .arg(&m.path)
        .args(["-map", &audio_index, "-af", &filter])
        .args(["-vn", "-ac", "1", "-ar", "4000", "-f", "f32le", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = ctx
        .processes
        .spawn(&mut process)
        .map_err(|e| e.to_string())?;
    let mut err = child.take_stderr().unwrap();
    let drain = std::thread::spawn(move || {
        let mut tail = vec![];
        let mut block = [0u8; 4096];
        loop {
            let n = err.read(&mut block).unwrap_or(0);
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
    let mut out = child.take_stdout().unwrap();
    let mut wave = vec![0f32; 1024];
    let expected = (m.duration.value() * 4000.).ceil().max(1.) as usize;
    let mut sample = 0usize;
    let mut bytes = [0u8; 4096];
    let mut remainder = vec![];
    loop {
        let n = out.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        remainder.extend_from_slice(&bytes[..n]);
        let complete = remainder.len() / 4 * 4;
        for chunk in remainder[..complete].chunks_exact(4) {
            let value = f32::from_le_bytes(chunk.try_into().unwrap());
            let bin = (sample.saturating_mul(1024) / expected).min(1023);
            if value.is_finite() {
                wave[bin] = wave[bin].max(value.abs().min(1.));
            }
            sample += 1;
        }
        remainder.drain(..complete);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = drain.join().unwrap_or_default();
    if ctx.processes.is_closing() {
        return Err("Media processing cancelled: application is closing".into());
    }
    if !status.success() {
        return Err(format!("Waveform generation failed: {}", diagnostic(&err)));
    }
    Ok(wave)
}
pub fn prepare_source(ctx: &MediaContext, m: &Media) -> Result<String, String> {
    prepare_source_pinned(ctx, m).map(|(path, _pin)| path)
}
/// The caller retains the returned guard until playback adopts or releases this
/// source. This closes the gap between a cache hit and frontend asset adoption.
pub fn prepare_source_pinned(ctx: &MediaContext, m: &Media) -> Result<(String, AssetPin), String> {
    let source_pin = ctx.processes.pin_file(&m.path).map_err(|e| e.to_string())?;
    if m.missing || !Path::new(&m.path).is_file() {
        return Err("Source media is missing; relink it in the media bin".into());
    }
    if m.kind == "image" {
        return Ok((m.path.clone(), source_pin));
    }
    ensure_cache(ctx)?;
    let key = cache_key(Path::new(&m.path))?;
    let dest = ctx.cache_dir.join(format!("{key}-source-v5.mp4"));
    let destination_pin = ctx.processes.pin_file(&dest).map_err(|e| e.to_string())?;
    if dest.is_file() {
        return Ok((dest.to_string_lossy().to_string(), destination_pin));
    }
    let part = ctx
        .cache_dir
        .join(format!("{key}-source-part-{}.mp4", id()));
    let _temporary = ctx
        .processes
        .temporary_file(part.clone())
        .map_err(|e| e.to_string())?;
    let job_id = id();
    let args = normalized_cache_args(ctx, m, 1920, 1080, 20, &part, &job_id)?;
    if let Err(error) = run_ffmpeg(ctx, &args) {
        cleanup_cache_sidecars(ctx, &job_id);
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    cleanup_cache_sidecars(ctx, &job_id);
    fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    prune_cache(ctx, 2 * 1024 * 1024 * 1024)?;
    Ok((dest.to_string_lossy().to_string(), destination_pin))
}
/// Source monitors and proxies use the sequence renderer, including its shared
/// origin, leading black/silence and explicit frame/sample conversion.
pub fn normalized_cache_args(
    ctx: &MediaContext,
    media: &Media,
    max_width: u32,
    max_height: u32,
    crf: u32,
    output: &Path,
    job_id: &str,
) -> Result<Vec<String>, String> {
    let _source_pin = ctx
        .processes
        .pin_file(&media.path)
        .map_err(|e| e.to_string())?;
    let _output_pin = ctx.processes.pin_file(output).map_err(|e| e.to_string())?;
    let mut source = media.clone();
    // Cache coverage is the physical stream span, not a saved legacy tail.
    source.legacy_source = None;
    if source.timing.is_none() {
        let probed = probe(ctx, Path::new(&source.path))?;
        source.timing = probed.timing;
        source.duration = probed.duration;
    }
    if let Some(timing) = &source.timing {
        // Legacy saved durations may include a container's absolute origin.
        // Caches cover the measured stream span; saved timeline edits stay intact.
        let complete = (source.kind == "audio" || timing.video_end.is_some())
            && (!source.has_audio || timing.audio_end.is_some());
        if complete {
            if let Some(end) = timing
                .video_end
                .into_iter()
                .chain(timing.audio_end)
                .reduce(later)
            {
                let measured = end.checked_sub(timing.origin)?;
                if measured.num > 0 {
                    source.duration = measured;
                }
            }
        }
    }
    source.proxy = None;
    source.proxy_timing_version = None;
    let fps = if source.fps.num > 0 && (1.0..=240.0).contains(&source.fps.value()) {
        source.fps
    } else {
        Rational::new(30, 1)
    };
    let (width, height) = if source.kind == "audio" {
        (640, 360)
    } else {
        let factor = (max_width as f64 / source.width.max(1) as f64)
            .min(max_height as f64 / source.height.max(1) as f64)
            .min(1.);
        (
            ((source.width as f64 * factor) as u32 / 2 * 2).max(16),
            ((source.height as f64 * factor) as u32 / 2 * 2).max(16),
        )
    };
    let frame_numerator = source.duration.num as i128 * fps.num as i128;
    let frame_denominator = source.duration.den as i128 * fps.den as i128;
    let duration =
        i64::try_from(((frame_numerator + frame_denominator - 1) / frame_denominator).max(1))
            .map_err(|_| "Media duration exceeds cache limits")?;
    // A cache may pad its final partial video frame; do not truncate audio tail.
    // This duration extension belongs only to the temporary cache project.
    source.duration = Rational::from_frames(duration, fps);
    let mut project = Project::new("Media cache".into(), width, height, fps);
    let track = project
        .tracks
        .iter()
        .find(|track| {
            track.kind
                == if source.kind == "audio" {
                    "audio"
                } else {
                    "video"
                }
        })
        .unwrap()
        .id
        .clone();
    let media_id = source.id.clone();
    project.media.push(source);
    crate::edit::apply(
        &mut project,
        EditCommand::AddClip {
            media_id,
            track_id: track,
            start: 0,
            source_in: Some(Rational::zero()),
            duration: Some(duration),
        },
    )?;
    let settings = ExportSettings {
        width,
        height,
        fps,
        codec: "h264".into(),
        crf,
        audio_bitrate: 160,
        sample_rate: 48000,
    };
    let mut args = crate::render::compile_cache(ctx, &project, &settings, output, job_id)?.args;
    let key_interval = ((fps.num + fps.den * 2 - 1) / (fps.den * 2))
        .max(1)
        .to_string();
    let before_output = args.len() - 1;
    args.splice(
        before_output..before_output,
        [
            "-g".into(),
            key_interval.clone(),
            "-keyint_min".into(),
            key_interval,
            "-sc_threshold".into(),
            "0".into(),
        ],
    );
    Ok(args)
}
fn cleanup_cache_sidecars(ctx: &MediaContext, job_id: &str) {
    let _ = fs::remove_file(ctx.cache_dir.join(format!("{job_id}-filters.txt")));
}
pub fn capabilities(ctx: &MediaContext) -> Result<Capabilities, String> {
    let mut process = command(&ctx.ffmpeg);
    process.arg("-version");
    let version =
        managed_output(ctx, &mut process).map_err(|e| format!("FFmpeg unavailable: {e}"))?;
    if !version.status.success() {
        return Err("FFmpeg failed to start".into());
    }
    let mut process = command(&ctx.ffmpeg);
    process.args(["-hide_banner", "-encoders"]);
    let enc = managed_output(ctx, &mut process)?;
    let mut process = command(&ctx.ffmpeg);
    process.args(["-hide_banner", "-hwaccels"]);
    let hw = managed_output(ctx, &mut process)?;
    let encoders = String::from_utf8_lossy(&enc.stdout)
        .lines()
        .filter_map(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            if f.len() > 2
                && f[0].len() == 6
                && f[0]
                    .chars()
                    .next()
                    .map(|c| c == 'V' || c == 'A')
                    .unwrap_or(false)
            {
                Some(f[1].into())
            } else {
                None
            }
        })
        .collect();
    let hardware = String::from_utf8_lossy(&hw.stdout)
        .lines()
        .skip(1)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().into())
        .collect();
    Ok(Capabilities {
        ffmpeg: ctx.ffmpeg.to_string_lossy().to_string(),
        ffprobe: ctx.ffprobe.to_string_lossy().to_string(),
        version: String::from_utf8_lossy(&version.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .into(),
        encoders,
        hardware,
        gpu_devices: gpu_devices(ctx),
        cache_dir: ctx.cache_dir.to_string_lossy().to_string(),
    })
}
fn gpu_devices(ctx: &MediaContext) -> Vec<String> {
    #[cfg(windows)]
    let mut probe = {
        let mut c = command(Path::new("powershell.exe"));
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut probe = {
        let mut c = command(Path::new("lspci"));
        c.arg("-mm");
        c
    };
    #[cfg(target_os = "macos")]
    let mut probe = {
        let mut c = command(Path::new("system_profiler"));
        c.arg("SPDisplaysDataType");
        c
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        return vec![];
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        probe
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(child) = ctx.processes.spawn(&mut probe) else {
            return vec![];
        };
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return vec![];
                    }
                    break;
                }
                Err(_) => return vec![],
                Ok(None) => {
                    if started.elapsed().as_secs() >= 4 {
                        let _ = child.kill();
                        let _ = child.wait();
                        return vec![];
                    }
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
            }
        }
        let mut bytes = vec![];
        if let Some(stdout) = child.take_stdout() {
            let _ = stdout.take(16384).read_to_end(&mut bytes);
        }
        let text = String::from_utf8_lossy(&bytes);
        #[cfg(windows)]
        {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect()
        }
        #[cfg(target_os = "linux")]
        {
            text.lines()
                .filter(|l| {
                    l.contains("VGA")
                        || l.contains("3D controller")
                        || l.contains("Display controller")
                })
                .map(|l| l.trim().to_string())
                .collect()
        }
        #[cfg(target_os = "macos")]
        {
            text.lines()
                .filter_map(|l| {
                    l.trim()
                        .strip_prefix("Chipset Model:")
                        .map(|v| v.trim().to_string())
                })
                .collect()
        }
    }
}
/// Retain at most max_bytes of cache, except active assets, in-progress files and
/// the short creation grace period. Only ownership pins determine active use;
/// file age alone never makes a playback asset eligible for removal.
pub fn prune_cache(ctx: &MediaContext, max_bytes: u64) -> Result<(), String> {
    // Preview records and outputs are one retention unit. The preview subcache
    // accounts for both; generic cleanup must not unlink a pinned output's record.
    crate::preview::prune(ctx);
    let mut files = vec![];
    let mut total = 0u64;
    for entry in fs::read_dir(&ctx.cache_dir)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        total = total.saturating_add(meta.len());
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        files.push((modified, path, meta.len()));
    }
    files.sort_by_key(|f| f.0);
    for (modified, path, size) in files {
        if total <= max_bytes {
            break;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("preview-") && (name.ends_with(".mp4") || name.ends_with(".json")) {
            continue;
        }
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            .as_secs()
            < 60
            || path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .contains("part")
        {
            continue;
        }
        if ctx.processes.remove_unprotected_file(path).unwrap_or(false) {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn probe_timestamps_keep_signed_decimal_and_stream_tick_precision() {
        assert_eq!(
            decimal_rational("-0.021333333"),
            Some(Rational::new(-21_333_333, 1_000_000_000))
        );
        assert_eq!(decimal_rational("+5.200000000"), Some(Rational::new(26, 5)));
        assert_eq!(decimal_rational("1000000000.000000001"), None);
        assert_eq!(decimal_rational("--1"), None);
        assert_eq!(decimal_rational("NaN"), None);
        let data = json!({"format": {"start_time": "5.000000"}, "streams": [
            {"index": 2, "start_pts": -1024, "time_base": "1/48000", "start_time": "0.000000"},
            {"index": 3, "start_pts": 5000, "time_base": "1/1000"}
        ]});
        let timing = probe_timing(&data, Some(&data["streams"][1]), Some(&data["streams"][0]));
        assert_eq!(timing.origin, Rational::new(-8, 375));
        assert_eq!(timing.video_start, Some(Rational::new(5, 1)));
        assert_eq!(timing.video_stream, Some(3));
        assert_eq!(timing.audio_stream, Some(2));
    }

    #[test]
    fn duration_distinguishes_matroska_absolute_end_from_mp4_elapsed_ticks() {
        let matroska = json!({"format": {"format_name": "matroska,webm", "start_time": "5.000000", "duration": "7.000000"}, "streams": [
            {"index": 0, "start_pts": 5000, "time_base": "1/1000", "tags": {"DURATION": "00:00:07.000000000"}},
            {"index": 1, "start_pts": 5200, "time_base": "1/1000", "tags": {"DURATION": "00:00:07.000000000"}}
        ]});
        let video = Some(&matroska["streams"][0]);
        let audio = Some(&matroska["streams"][1]);
        let timing = probe_timing(&matroska, video, audio);
        assert_eq!(timing.origin, Rational::new(5, 1));
        assert_eq!(timing.video_end, Some(Rational::new(7, 1)));
        assert_eq!(
            probe_duration(&matroska, video, audio, &timing),
            Some(Rational::new(2, 1))
        );

        let mp4 = json!({"format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "start_time": "5.000000", "duration": "2.000000"}, "streams": [
            {"index": 0, "start_pts": 76800, "time_base": "1/15360", "duration_ts": 30720},
            {"index": 1, "start_pts": 249600, "time_base": "1/48000", "duration_ts": 86400}
        ]});
        let video = Some(&mp4["streams"][0]);
        let audio = Some(&mp4["streams"][1]);
        let timing = probe_timing(&mp4, video, audio);
        assert_eq!(timing.video_end, Some(Rational::new(7, 1)));
        assert_eq!(timing.audio_start, Some(Rational::new(26, 5)));
        assert_eq!(
            probe_duration(&mp4, video, audio, &timing),
            Some(Rational::new(2, 1))
        );
    }
}
