//! A sample-exact seekable audio clock prepared once, before bounded previews.
//! Some containers round packet PTS to milliseconds. Their original decoded
//! sample count cannot be recovered accurately after an arbitrary packet seek.
use crate::{
    media::{self, MediaContext},
    model::*,
    preview::FileStamp,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    recipe: String,
    source: FileStamp,
    output: FileStamp,
    sample_rate: u32,
}
pub struct PreparedAudio {
    pub path: PathBuf,
    pub pins: Vec<crate::processes::AssetPin>,
}
const RECIPE: &str = "source-audio-clock-v1";
pub const MAX_ENTRIES: usize = 64;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
// An individual cold preparation cannot bypass the independent audio budget.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
pub fn fits(m: &Media, sample_rate: u32) -> bool {
    m.duration.value() * sample_rate as f64 * 8. <= (MAX_FILE_BYTES - 4096) as f64
}
pub fn ensure(
    ctx: &MediaContext,
    m: &Media,
    use_proxy: bool,
    sample_rate: u32,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<PreparedAudio, String> {
    media::ensure_cache(ctx)?;
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err("Audio preparation cancelled".into());
    }
    if !(8000..=192000).contains(&sample_rate) {
        return Err("Invalid audio cache sample rate".into());
    }
    if !fits(m, sample_rate) {
        return Err("This source exceeds the 256 MiB precise-audio cache budget".into());
    }
    let proxy = use_proxy && media::proxy_is_current(m);
    let path = Path::new(if proxy {
        m.proxy.as_ref().unwrap()
    } else {
        &m.path
    });
    let source = FileStamp::read(path)?;
    let timing = match &m.timing {
        Some(t) => Some(t.clone()),
        None => media::probe(ctx, Path::new(&m.path))?.timing,
    };
    let origin = if proxy {
        Rational::zero()
    } else {
        timing
            .as_ref()
            .map(|t| t.origin)
            .unwrap_or(Rational::zero())
    };
    let stream = if proxy {
        "0:a:0".into()
    } else {
        timing
            .as_ref()
            .and_then(|t| t.audio_stream)
            .map(|s| format!("0:{s}"))
            .unwrap_or("0:a:0".into())
    };
    let identity = serde_json::json!({"recipe":RECIPE,"source":source,"sample_rate":sample_rate,"origin":origin,"stream":stream,"ffmpeg":FileStamp::read(&ctx.ffmpeg)?});
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&identity).map_err(|e| e.to_string())?)
    );
    let output = ctx.cache_dir.join(format!("{key}-audio-clock-v1.wav"));
    let manifest = output.with_extension("json");
    let _source_pin = ctx.processes.pin_file(path).map_err(|e| e.to_string())?;
    let output_pin = ctx.processes.pin_file(&output).map_err(|e| e.to_string())?;
    let manifest_pin = ctx
        .processes
        .pin_file(&manifest)
        .map_err(|e| e.to_string())?;
    if let Ok(mut file) = fs::File::open(&manifest) {
        let mut bytes = vec![];
        let _ = file.by_ref().take(8193).read_to_end(&mut bytes);
        if bytes.len() <= 8192 {
            if let Ok(record) = serde_json::from_slice::<Record>(&bytes) {
                if record.recipe == RECIPE
                    && record.source == source
                    && record.sample_rate == sample_rate
                    && FileStamp::read(&output).ok().as_ref() == Some(&record.output)
                {
                    let _ = file.set_modified(std::time::SystemTime::now());
                    return Ok(PreparedAudio {
                        path: output,
                        pins: vec![output_pin, manifest_pin],
                    });
                }
            }
        }
    }
    let part = ctx.cache_dir.join(format!(".mono-audio-part-{}.wav", id()));
    let _temporary = ctx
        .processes
        .temporary_file(part.clone())
        .map_err(|e| e.to_string())?;
    let args=vec!["-hide_banner".into(),"-v".into(),"error".into(),"-nostdin".into(),"-y".into(),"-copyts".into(),"-i".into(),path.to_string_lossy().into_owned(),"-map".into(),stream,"-vn".into(),"-af".into(),format!("asetpts=PTS-({}/{})/TB,aresample={sample_rate}:first_pts=0,asetpts=N/SR/TB,aformat=sample_fmts=fltp:channel_layouts=stereo",origin.num,origin.den),"-c:a".into(),"pcm_f32le".into(),"-rf64".into(),"auto".into(),part.to_string_lossy().into_owned()];
    let mut process = media::command(&ctx.ffmpeg);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = ctx
        .processes
        .spawn(&mut process)
        .map_err(|e| e.to_string())?;
    let stderr = child.take_stderr().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        let mut stderr = stderr;
        let mut block = [0; 4096];
        loop {
            let n = stderr.read(&mut block).unwrap_or(0);
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&block[..n]);
            if bytes.len() > 65536 {
                bytes.drain(..bytes.len() - 65536);
            }
        }
        bytes
    });
    let cancelled = || cancel.is_some_and(|flag| flag.load(Ordering::Acquire));
    let status = loop {
        if cancelled() {
            let _ = child.kill();
            break child.wait().map_err(|e| e.to_string())?;
        }
        if fs::metadata(&part).is_ok_and(|meta| meta.len() > MAX_FILE_BYTES) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err("Audio preparation exceeded its 256 MiB file budget".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let error = reader.join().unwrap_or_default();
    if cancelled() {
        return Err("Audio preparation cancelled".into());
    }
    if !status.success() {
        return Err(format!(
            "Audio preparation failed: {}",
            media::diagnostic(&error)
        ));
    }
    if FileStamp::read(path)? != source {
        return Err("PREVIEW_IDENTITY_CHANGED: Audio source changed during preparation".into());
    }
    let (_actual, details) = media::probe_details(ctx, &part)?;
    let streams = details["streams"]
        .as_array()
        .ok_or("Audio cache contains no streams")?;
    let a = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .ok_or("Audio cache contains no audio")?;
    if a["codec_name"] != "pcm_f32le"
        || a["sample_rate"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            != Some(sample_rate)
        || a["channels"] != 2
    {
        return Err("Audio cache format is incorrect".into());
    }
    fs::rename(&part, &output).map_err(|e| e.to_string())?;
    let record = Record {
        recipe: RECIPE.into(),
        source,
        output: FileStamp::read(&output)?,
        sample_rate,
    };
    let temporary = manifest.with_extension("json.tmp");
    let _guard = ctx
        .processes
        .temporary_file(temporary.clone())
        .map_err(|e| e.to_string())?;
    fs::write(
        &temporary,
        serde_json::to_vec(&record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&temporary, &manifest).map_err(|e| e.to_string())?;
    prune(ctx);
    Ok(PreparedAudio {
        path: output,
        pins: vec![output_pin, manifest_pin],
    })
}

/// Retain audio files and their records as one unit. Playback/render pins are
/// respected; active consumers can temporarily exceed the retention target.
pub fn prune(ctx: &MediaContext) {
    let Ok(dir) = fs::read_dir(&ctx.cache_dir) else {
        return;
    };
    let mut records = vec![];
    let mut total = 0u64;
    for entry in dir.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with("-audio-clock-v1.json") && !path.with_extension("wav").is_file() {
            let _ = ctx.processes.remove_unprotected_file(&path);
            continue;
        }
        if !name.ends_with("-audio-clock-v1.wav") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let manifest = path.with_extension("json");
        if !manifest.is_file() && !ctx.processes.is_protected(&path) {
            let _ = ctx.processes.remove_unprotected_file(&path);
            continue;
        }
        total = total.saturating_add(meta.len());
        let accessed = fs::metadata(&manifest)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        records.push((accessed, path, manifest, meta.len()));
    }
    records.sort_by_key(|r| r.0);
    let mut count = records.len();
    for (_, path, manifest, bytes) in records {
        if count <= MAX_ENTRIES && total <= MAX_BYTES {
            break;
        }
        if ctx.processes.is_protected(&path) || ctx.processes.is_protected(&manifest) {
            continue;
        }
        if ctx
            .processes
            .remove_unprotected_file(&path)
            .unwrap_or(false)
        {
            let _ = ctx.processes.remove_unprotected_file(&manifest);
            count = count.saturating_sub(1);
            total = total.saturating_sub(bytes);
        }
    }
}
