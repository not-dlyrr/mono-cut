//! A single deterministic filter compiler drives both program preview and exports.
//! Timeline positions stay rational until FFmpeg's explicit seconds/sample boundaries.
use crate::{
    media::{self, ensure_cache, MediaContext},
    model::*,
};
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug)]
pub struct RenderPlan {
    pub args: Vec<String>,
    pub duration: f64,
    pub output_frames: i64,
    pub filter_graph: String,
    /// Half-open sequence-frame working interval. Includes bounded audio warm-up.
    pub working_region: PreviewRegion,
    pub input_ranges: Vec<InputRange>,
    pub asset_pins: Vec<crate::processes::AssetPin>,
    /// Independent audio exception: reads from the source origin when exact
    /// PCM preparation exceeds the per-file budget.
    pub audio_prefix_fallbacks: Vec<InputRange>,
    /// Source-specific video exceptions retain the original no-seek decoder clock.
    /// These spans grow with source_in and are not bounded region-only decoding.
    pub video_prefix_fallbacks: Vec<VideoPrefixFallback>,
    pub video_seek_checks: Vec<VideoSeekCheck>,
}
#[derive(Debug, serde::Serialize)]
pub struct VideoSeekCheck {
    pub clip_id: String,
    pub using_proxy: bool,
    pub inspection: crate::video_seek::Inspection,
}
#[derive(Debug, serde::Serialize)]
pub struct VideoPrefixFallback {
    pub clip_id: String,
    pub requested_seek: Rational,
    pub seek: Rational,
    /// Nominal origin-to-contributing-end span, excluding decoder read-ahead.
    pub duration: Rational,
    pub termination: InputTermination,
}
#[derive(Debug, serde::Serialize)]
pub struct InputRange {
    pub clip_id: String,
    pub seek: Rational,
    /// Nominal source window (or explicit origin-prefix span), excluding
    /// unavoidable GOP preroll and decoder read-ahead. Not a decoded-frame cap.
    pub duration: Rational,
    pub termination: InputTermination,
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTermination {
    /// Exact source/clip trims return EOF upstream; earlier keyframes must still decode.
    FilterEof,
    /// An input duration is valid for a clock-aligned still/PCM or origin-prefix read.
    InputDuration,
}
fn number(v: f64) -> String {
    format!("{v:.12}")
}
fn seconds(frames: i64, fps: Rational) -> String {
    number(Rational::from_frames(frames, fps).value())
}
fn rational_expression(value: Rational) -> String {
    format!("({}/{})", value.num, value.den)
}
fn sample_boundary(time: Rational, sample_rate: u32) -> i128 {
    let n = time.num as i128 * sample_rate as i128;
    let d = time.den as i128;
    -(-n).div_euclid(d)
}
fn frame_nearest(time: Rational, fps: Rational) -> Result<i64, String> {
    let ticks = time.checked_mul(fps)?;
    let n = ticks.num as i128;
    let d = ticks.den as i128;
    let rounded = (n.abs() * 2 + d) / (d * 2) * n.signum();
    i64::try_from(rounded).map_err(|_| "Conversion phase exceeds frame limits".into())
}
/// After source-fps normalization, choose integer ticks for every source frame,
/// retained origin, and rational speed operation. FFmpeg timebases are int32;
/// its setpts evaluator is double, so fall back to AVTB outside exact bounds.
fn exact_retime_timebase(
    source_fps: Rational,
    origin: Rational,
    speed: Rational,
    end: Rational,
) -> Option<i64> {
    fn gcd(mut a: i128, mut b: i128) -> i128 {
        while b != 0 {
            let next = a % b;
            a = b;
            b = next;
        }
        a
    }
    let a = source_fps.num as i128;
    let b = origin.den as i128;
    let ticks = (a / gcd(a, b)) * b * speed.num as i128;
    if !(1..=i32::MAX as i128).contains(&ticks) {
        return None;
    }
    for time in [end, origin] {
        if time.num as i128 * ticks * speed.den as i128 > time.den as i128 * (1i128 << 52) {
            return None;
        }
    }
    Some(ticks as i64)
}
fn composition_time(c: &Clip, fps: Rational) -> Result<Rational, String> {
    if let Some(r) = &c.retime {
        let offset = r
            .composition_source_offset
            .unwrap_or(Rational::zero())
            .checked_mul(Rational::new(c.speed.den, c.speed.num))?;
        Rational::from_frames(c.start, fps).checked_sub(offset)
    } else {
        Ok(Rational::from_frames(c.composition_start(), fps))
    }
}
fn compare_time(a: Rational, b: Rational) -> std::cmp::Ordering {
    (a.num as i128 * b.den as i128).cmp(&(b.num as i128 * a.den as i128))
}
fn has_keys(c: &Clip, property: &str) -> bool {
    c.retime.as_ref().map_or_else(
        || c.keyframes.iter().any(|k| k.property == property),
        |r| r.envelope.keyframes.iter().any(|k| k.property == property),
    )
}
/// The canonical envelope remains in source seconds across rounding, cuts and
/// repeated retimes. Legacy clips retain their original integer-frame graph.
fn fade_times(c: &Clip, fps: Rational) -> (f64, f64, f64, f64) {
    if let Some(r) = &c.retime {
        let speed = c.speed.value();
        (
            r.envelope.fade_in.value() / speed,
            r.envelope.fade_out.value() / speed,
            r.envelope.fade_in_start.value() / speed,
            r.envelope.fade_out_end.value() / speed,
        )
    } else {
        (
            Rational::from_frames(c.fade_in, fps).value(),
            Rational::from_frames(c.fade_out, fps).value(),
            Rational::from_frames(c.fade_in_start(), fps).value(),
            Rational::from_frames(c.fade_out_end(), fps).value(),
        )
    }
}
fn fade_rationals(
    c: &Clip,
    fps: Rational,
) -> Result<(Rational, Rational, Rational, Rational), String> {
    if let Some(r) = &c.retime {
        let inverse = Rational::new(c.speed.den, c.speed.num);
        Ok((
            r.envelope.fade_in.checked_mul(inverse)?,
            r.envelope.fade_out.checked_mul(inverse)?,
            r.envelope.fade_in_start.checked_mul(inverse)?,
            r.envelope.fade_out_end.checked_mul(inverse)?,
        ))
    } else {
        Ok((
            Rational::from_frames(c.fade_in, fps),
            Rational::from_frames(c.fade_out, fps),
            Rational::from_frames(c.fade_in_start(), fps),
            Rational::from_frames(c.fade_out_end(), fps),
        ))
    }
}
fn is_outgoing_dissolve(c: &Clip, p: &Project) -> Result<bool, String> {
    let (_, out, _, end) = fade_rationals(c, p.fps)?;
    if out.num == 0 {
        return Ok(false);
    }
    let finish = Rational::from_frames(c.start, p.fps).checked_add(end)?;
    let begin = finish.checked_sub(out)?;
    let composition = composition_time(c, p.fps)?;
    for next in &p.clips {
        if next.id == c.id || next.track_id != c.track_id {
            continue;
        }
        let (input, _, start, _) = fade_rationals(next, p.fps)?;
        if compare_time(input, out).is_eq()
            && compare_time(
                Rational::from_frames(next.start, p.fps).checked_add(start)?,
                begin,
            )
            .is_eq()
            && compare_time(composition_time(next, p.fps)?, composition).is_gt()
            && compare_time(Rational::from_frames(next.start, p.fps), finish).is_lt()
            && compare_time(Rational::from_frames(next.end(), p.fps), begin).is_gt()
        {
            return Ok(true);
        }
    }
    Ok(false)
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
fn escaped_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}
/// Piecewise linear interpolation, clamped at first/last keyframe, in clip-local seconds.
pub fn expression(c: &Clip, property: &str, variable: &str, fps: Rational, default: f64) -> String {
    if let Some(r) = &c.retime {
        let mut keys: Vec<_> = r
            .envelope
            .keyframes
            .iter()
            .filter(|k| k.property == property)
            .collect();
        keys.sort_by(|a, b| compare_time(a.time, b.time));
        if keys.is_empty() {
            return number(default);
        }
        let mut tail = number(keys.last().unwrap().value);
        for pair in keys.windows(2).rev() {
            let start = pair[0].time.value() / c.speed.value();
            let end = pair[1].time.value() / c.speed.value();
            let linear = format!(
                "{}+({}-{})*({}-{})/{}",
                number(pair[0].value),
                number(pair[1].value),
                number(pair[0].value),
                variable,
                number(start),
                number(end - start)
            );
            tail = format!("if(lt({variable},{}),{linear},{tail})", number(end));
        }
        return format!(
            "if(lt({variable},{}),{},{tail})",
            number(keys[0].time.value() / c.speed.value()),
            number(keys[0].value)
        );
    }
    let mut keys: Vec<_> = c
        .keyframes
        .iter()
        .filter(|k| k.property == property)
        .collect();
    keys.sort_by_key(|k| k.frame);
    if keys.is_empty() {
        return number(default);
    }
    let mut tail = number(keys.last().unwrap().value);
    for window in keys.windows(2).rev() {
        let a = window[0];
        let b = window[1];
        let start = Rational::from_frames(a.frame, fps).value();
        let end = Rational::from_frames(b.frame, fps).value();
        let linear = format!(
            "{}+({}-{})*({}-{})/{}",
            number(a.value),
            number(b.value),
            number(a.value),
            variable,
            number(start),
            number(end - start)
        );
        tail = format!("if(lt({variable},{}),{linear},{tail})", number(end));
    }
    format!(
        "if(lt({variable},{}),{},{tail})",
        seconds(keys[0].frame, fps),
        number(keys[0].value)
    )
}
fn varispeed(speed: Rational, sample_rate: u32) -> String {
    if speed.num == speed.den {
        return format!("aresample={sample_rate}");
    }
    // WSOLA/atempo can discard a window even at unity, advancing transients. Basic speed
    // uses deterministic sample-rate conversion; pitch-preserving stretching is deferred.
    let rate = ((sample_rate as i128 * speed.num as i128 + speed.den as i128 / 2)
        / speed.den as i128) as u32;
    format!("aresample={sample_rate},asetrate={rate},aresample={sample_rate}")
}
fn fade_gain(c: &Clip, variable: &str, fps: Rational) -> String {
    if c.retime.is_some() {
        let (fade_in, fade_out, start, end) = fade_times(c, fps);
        let mut gains = vec![];
        if fade_in > 0. {
            gains.push(format!(
                "clip(({variable}-{})/{},0,1)",
                number(start),
                number(fade_in)
            ));
        }
        if fade_out > 0. {
            gains.push(format!(
                "clip(({}-{variable})/{},0,1)",
                number(end),
                number(fade_out)
            ));
        }
        return if gains.is_empty() {
            "1".into()
        } else {
            gains.join("*")
        };
    }
    let mut gains = vec![];
    if c.fade_in > 0 {
        gains.push(format!(
            "clip(({variable}-{})/{},0,1)",
            seconds(c.fade_in_start(), fps),
            seconds(c.fade_in, fps)
        ));
    }
    if c.fade_out > 0 {
        gains.push(format!(
            "clip(({}-{variable})/{},0,1)",
            seconds(c.fade_out_end(), fps),
            seconds(c.fade_out, fps)
        ));
    }
    if gains.is_empty() {
        "1".into()
    } else {
        gains.join("*")
    }
}
fn video_fade(filters: &mut Vec<String>, kind: &str, start: i64, duration: i64, fps: Rational) {
    // FFmpeg's fade rejects negative starts. Temporarily shift only its clock;
    // opacity/transforms above and timeline placement below retain local time.
    if start < 0 {
        // Integer frame ticks avoid losing a tick when decimal seconds round
        // just below a frame boundary (notably on repeated 30 fps cuts).
        filters.push(format!(
            "settb={}/{},setpts=PTS-({start})",
            fps.den, fps.num
        ));
    }
    filters.push(format!(
        "fade=t={kind}:st={}:d={}:alpha=1",
        seconds(start.max(0), fps),
        seconds(duration, fps)
    ));
    if start < 0 {
        filters.push(format!("setpts=PTS+({start})"));
    }
}
fn video_fade_seconds(
    filters: &mut Vec<String>,
    kind: &str,
    start: f64,
    duration: f64,
    fps: Rational,
) {
    if start < 0. {
        filters.push(format!("settb=AVTB,setpts=PTS-({})/TB", number(start)));
    }
    filters.push(format!(
        "fade=t={kind}:st={}:d={}:alpha=1",
        number(start.max(0.)),
        number(duration)
    ));
    if start < 0. {
        filters.push(format!(
            "setpts=PTS+({})/TB,settb={}/{}",
            number(start),
            fps.den,
            fps.num
        ));
    }
}
pub fn validate_settings(settings: &ExportSettings) -> Result<(), String> {
    if settings.width < 16
        || settings.width > 16384
        || settings.height < 16
        || settings.height > 16384
        || settings.width % 2 != 0
        || settings.height % 2 != 0
    {
        return Err("Export dimensions must be even, between 16 and 16384".into());
    }
    settings.fps.validate(true)?;
    if !(1.0..=240.0).contains(&settings.fps.value()) {
        return Err("Export frame rate must be between 1 and 240".into());
    }
    if settings.codec != "h264" && settings.codec != "ffv1" {
        return Err("Supported codecs are H.264 and FFV1".into());
    }
    if settings.crf > 51
        || !(8000..=192000).contains(&settings.sample_rate)
        || !(32..=512).contains(&settings.audio_bitrate)
    {
        return Err("Invalid quality or audio settings (audio bitrate uses kb/s)".into());
    }
    Ok(())
}
pub fn compile(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    use_proxies: bool,
    output: &Path,
    job_id: &str,
) -> Result<RenderPlan, String> {
    compile_with_purpose(
        ctx,
        p,
        settings,
        use_proxies,
        output,
        job_id,
        true,
        None,
        None,
    )
}

pub fn compile_region(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    use_proxies: bool,
    output: &Path,
    job_id: &str,
    region: &PreviewRegion,
) -> Result<RenderPlan, String> {
    region.validate(p)?;
    compile_with_purpose(
        ctx,
        p,
        settings,
        use_proxies,
        output,
        job_id,
        true,
        Some(region),
        None,
    )
}
pub fn compile_region_cancelled(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    use_proxies: bool,
    output: &Path,
    job_id: &str,
    region: &PreviewRegion,
    cancel: &Arc<AtomicBool>,
) -> Result<RenderPlan, String> {
    region.validate(p)?;
    compile_with_purpose(
        ctx,
        p,
        settings,
        use_proxies,
        output,
        job_id,
        true,
        Some(region),
        Some(cancel),
    )
}

#[cfg(test)]
mod retime_grid_tests {
    use super::*;

    #[test]
    fn inherited_phase_quantizes_once_with_symmetric_half_ties() {
        let fps = Rational::new(30, 1);
        assert_eq!(frame_nearest(Rational::new(1, 60), fps).unwrap(), 1);
        assert_eq!(frame_nearest(Rational::new(-1, 60), fps).unwrap(), -1);
        assert_eq!(frame_nearest(Rational::new(49, 3000), fps).unwrap(), 0);
        assert_eq!(frame_nearest(Rational::new(51, 3000), fps).unwrap(), 1);
        // A source continuation at 7/60 s becomes 3 sequence ticks at 7/6×.
        let phase = Rational::new(7, 60)
            .checked_mul(Rational::new(6, 7))
            .unwrap();
        assert_eq!(frame_nearest(phase, fps).unwrap(), 3);
        assert_eq!(
            frame_nearest(Rational::new(1001, 60000), Rational::new(30000, 1001)).unwrap(),
            1
        );
    }
    #[test]
    fn exact_speed_ticks_cover_fractional_origins_and_fall_back_at_ffmpeg_limits() {
        let fps = Rational::new(30, 1);
        assert_eq!(
            exact_retime_timebase(
                fps,
                Rational::zero(),
                Rational::new(2, 1),
                Rational::new(16, 1)
            ),
            Some(60)
        );
        assert_eq!(
            exact_retime_timebase(
                fps,
                Rational::new(7, 13),
                Rational::new(7, 6),
                Rational::new(16, 1)
            ),
            Some(2730)
        );
        assert_eq!(
            exact_retime_timebase(
                fps,
                Rational::new(1, 999_999_937),
                Rational::new(7, 6),
                Rational::new(16, 1)
            ),
            None
        );
        assert_eq!(
            exact_retime_timebase(
                Rational::new(240, 1),
                Rational::zero(),
                Rational::new(32, 1),
                Rational::new(1_000_000_000_000, 1)
            ),
            None
        );
    }
}

/// Neutral source preparation uses the same clocks and composition rules, but
/// must not apply the sequence mix limiter before a later program render.
pub fn compile_cache(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    output: &Path,
    job_id: &str,
) -> Result<RenderPlan, String> {
    compile_with_purpose(ctx, p, settings, false, output, job_id, false, None, None)
}

fn compile_with_purpose(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    use_proxies: bool,
    output: &Path,
    job_id: &str,
    limit_mix: bool,
    region: Option<&PreviewRegion>,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<RenderPlan, String> {
    p.validate()?;
    validate_settings(settings)?;
    ensure_cache(ctx)?;
    if p.clips.is_empty() {
        return Err("Add clips to the timeline before rendering".into());
    }
    // A short bounded history warms resampling and the mix limiter. The entire
    // graph uses this local working clock; effect phases retain their original
    // clip clocks. No sequence-length generated base or delay is constructed.
    let history = (p.fps.value() * 0.5).ceil() as i64;
    let origin = region
        .map(|r| (r.start_frame - history).max(0))
        .unwrap_or(0);
    let working_end = region
        .map(|r| (r.end_frame + history).min(p.length()))
        .unwrap_or(p.length());
    let sequence_duration = Rational::from_frames(working_end - origin, p.fps).value();
    let fps = format!("{}/{}", p.fps.num, p.fps.den);
    let export_fps = format!("{}/{}", settings.fps.num, settings.fps.den);
    let start = region
        .map(|r| r.start_frame)
        .unwrap_or(p.in_point.unwrap_or(0));
    let end = region
        .map(|r| r.end_frame)
        .unwrap_or(p.out_point.unwrap_or(p.length()));
    let duration = Rational::from_frames(end - start, p.fps).value();
    let output_frames = (Rational::from_frames(end - start, p.fps).num as i128
        * settings.fps.num as i128
        + Rational::from_frames(end - start, p.fps).den as i128 * settings.fps.den as i128 / 2)
        / (Rational::from_frames(end - start, p.fps).den as i128 * settings.fps.den as i128);
    if output_frames < 1 {
        return Err("Export range is shorter than one output frame".into());
    }
    // Pixel operations are stateless, so slice threading preserves the shared
    // image/sample model. Cap the pool rather than spawning one filter thread
    // per logical processor; software-only and small-core systems still work.
    let graph_threads = std::thread::available_parallelism()
        .map(|count| count.get().min(4))
        .unwrap_or(1);
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        // Keep demuxed presentation timestamps: one shared source origin below
        // removes the container epoch without removing the relative A/V delay.
        "-copyts".into(),
        "-filter_complex_threads".into(),
        graph_threads.to_string(),
    ];
    let mut graph = vec![
        format!(
            "color=c=black:s={}x{}:r={fps}:d={},format=rgba[basev]",
            settings.width,
            settings.height,
            number(sequence_duration)
        ),
        format!(
            "anullsrc=r={}:cl=stereo,aformat=sample_fmts=dblp:channel_layouts=stereo,atrim=end_sample={}[basea]",
            settings.sample_rate,
            sample_boundary(
                Rational::from_frames(working_end - origin, p.fps),
                settings.sample_rate
            )
        ),
    ];
    let mut overlay = "basev".to_string();
    let mut audio_labels = vec!["basea".to_string()];
    let mut input = 0;
    let mut input_ranges = vec![];
    let mut asset_pins = vec![];
    let mut audio_prefix_fallbacks = vec![];
    let mut video_prefix_fallbacks = vec![];
    let mut video_seek_checks = vec![];
    let mut ordered: Vec<_> = p
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(ti, t)| {
            p.clips
                .iter()
                .filter(move |c| c.track_id == t.id)
                .map(move |c| composition_time(c, p.fps).map(|time| (ti, c, time)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(compare_time(a.2, b.2))
            .then(a.1.composition_group().cmp(b.1.composition_group()))
            .then(a.1.start.cmp(&b.1.start))
            .then(a.1.id.cmp(&b.1.id))
    });
    for (index, clip) in ordered.into_iter().enumerate() {
        let (_track_index, c, _composition_time) = clip;
        let clipped_start = c.start.max(origin);
        let clipped_end = c.end().min(working_end);
        if clipped_start >= clipped_end {
            continue;
        }
        let clip_offset = clipped_start - c.start;
        let track = p
            .tracks
            .iter()
            .find(|t| t.id == c.track_id)
            .ok_or("Track missing")?;
        let video = track.kind == "video" && !track.hidden;
        let media = c
            .media_id
            .as_ref()
            .and_then(|id| p.media.iter().find(|m| &m.id == id));
        let audio = !track.muted
            && media.map(|m| m.has_audio).unwrap_or(false)
            && (c.volume > 0.
                || c.retime.as_ref().map_or_else(
                    || {
                        c.keyframes
                            .iter()
                            .any(|k| k.property == "volume" && k.value > 0.)
                    },
                    |r| {
                        r.envelope
                            .keyframes
                            .iter()
                            .any(|k| k.property == "volume" && k.value > 0.)
                    },
                ));
        if !video && !audio {
            continue;
        }
        let local = Rational::from_frames(c.duration, p.fps).value();
        let render_origin = match &c.retime {
            Some(r) => r.render_source_origin,
            None => c.source_in.checked_sub(
                Rational::from_frames(c.render_offset.unwrap_or(0), p.fps).checked_mul(c.speed)?,
            )?,
        };
        let phase = c
            .source_in
            .checked_sub(render_origin)?
            .checked_mul(Rational::new(c.speed.den, c.speed.num))?;
        // The source conversion origin remains exact. Only the discrete final
        // video grid is quantized; returning to the original speed restores it.
        let render_offset = if c.retime.is_some() {
            frame_nearest(phase, p.fps)?
        } else {
            c.render_offset.unwrap_or(0)
        };
        let source_label: String;
        let mut audio_label = String::new();
        let mut source_origin = Rational::zero();
        let mut video_start = Rational::zero();
        let mut video_end = media
            .map(|m| m.duration)
            .unwrap_or(Rational::from_frames(c.duration, p.fps));
        let mut source_fps = fps.clone();
        let mut source_rate = p.fps;
        let mut seek_sample = 0i128;
        let mut audio_origin = Rational::zero();
        let mut audio_seek_sample = 0i128;
        let mut prepared_audio = false;
        let (sw, sh) = if let Some(m) = media {
            if m.kind == "image" {
                // A looped still has no finite source-end limit.
                video_end = c.source_in.checked_add(
                    Rational::from_frames(c.duration + 1, p.fps).checked_mul(c.speed)?,
                )?;
            }
            if m.kind != "image" {
                let rate = if m.fps.num > 0 && (1.0..=240.0).contains(&m.fps.value()) {
                    m.fps
                } else {
                    Rational::new(30, 1)
                };
                source_fps = format!("{}/{}", rate.num, rate.den);
                source_rate = Rational::new(rate.num, rate.den);
            }
            if m.missing || !Path::new(&m.path).is_file() {
                return Err(format!(
                    "Missing source '{}'; relink it before rendering",
                    m.name
                ));
            }
            let use_proxy = use_proxies && media::proxy_is_current(m);
            let path = if use_proxy {
                m.proxy.as_ref().unwrap()
            } else {
                &m.path
            };
            let timing = if m.kind == "image" {
                None
            } else {
                // Old version-1 documents can also enter through the engine API
                // without runtime hydration. Resolve unknown timing here too.
                match &m.timing {
                    Some(timing) => Some(timing.clone()),
                    None => media::probe(ctx, Path::new(&m.path))?.timing,
                }
            };
            if let Some(timing) = &timing {
                if !use_proxy {
                    source_origin = timing.origin;
                }
                // Normalized caches contain black in absent-video intervals.
                // Drop those frames so a proxy cannot cover an underlying track.
                if let Some(start) = timing.video_start {
                    video_start = later(video_start, start.checked_sub(timing.origin)?);
                }
                if let Some(end) = timing.video_end {
                    video_end = earlier(video_end, end.checked_sub(timing.origin)?);
                }
            }
            if m.kind == "image" {
                args.extend(["-loop".into(), "1".into(), "-framerate".into(), fps.clone()]);
                // A still's pixels have no source-time phase. Generate only the
                // contributing frames, then place them on the original clip
                // clock below; a large slipped/trimmed source_in must not make
                // us repeatedly decode a still prefix.
                args.extend(["-t".into(), seconds(clipped_end - clipped_start + 1, p.fps)]);
                input_ranges.push(InputRange {
                    clip_id: c.id.clone(),
                    seek: Rational::zero(),
                    duration: Rational::from_frames(clipped_end - clipped_start + 1, p.fps),
                    termination: InputTermination::InputDuration,
                });
            }
            if region.is_some() && m.kind != "image" {
                // Input-side seeking bounds the source start. Exact graph trim EOF
                // stops decoding at the contributing end, plus unavoidable GOP
                // preroll. An input -t with -noaccurate_seek would count from
                // the earlier decoded keyframe and could cut before the target.
                // Preserve source
                // timestamps and allow keyframe preroll; the exact trim below
                // retains original mixed-rate conversion and automation phase.
                let first_source = c
                    .source_in
                    .checked_add(Rational::from_frames(clip_offset, p.fps).checked_mul(c.speed)?)?;
                let seek = later(
                    first_source.checked_sub(Rational::new(1, 2))?,
                    Rational::zero(),
                );
                seek_sample = sample_boundary(seek, settings.sample_rate);
                // The varispeed resampler starts on the same rational sample
                // phase as a render from zero. Its input cadence numerator is
                // the smallest exact reset interval (e.g. seven at 7/6 speed).
                let speed_rate = ((settings.sample_rate as i128 * c.speed.num as i128
                    + c.speed.den as i128 / 2)
                    / c.speed.den as i128) as i64;
                let cadence = Rational::new(speed_rate, settings.sample_rate as i64).num as i128;
                seek_sample = seek_sample.div_euclid(cadence) * cadence;
                let seek = Rational::new(seek_sample as i64, settings.sample_rate as i64);
                let source_end = c
                    .source_in
                    .checked_add(
                        Rational::from_frames(clipped_end - c.start, p.fps).checked_mul(c.speed)?,
                    )?
                    .checked_add(Rational::new(1, 2))?;
                let prefix = if video && m.kind == "video" {
                    let stream = timing
                        .as_ref()
                        .filter(|_| !use_proxy)
                        .and_then(|timing| timing.video_stream)
                        .map(|stream| stream.to_string())
                        .unwrap_or_else(|| "v:0".into());
                    let inspection =
                        crate::video_seek::inspect(ctx, Path::new(path), &stream, cancel)?;
                    let prefix = inspection.needs_prefix();
                    video_seek_checks.push(VideoSeekCheck {
                        clip_id: c.id.clone(),
                        using_proxy: use_proxy,
                        inspection,
                    });
                    prefix
                } else {
                    false
                };
                // Missing initial packet PTS can make demuxer seeking discard
                // the initial GOP. Preserve the full-render decoder clock for
                // that physical source; the unchanged exact graph trim ends it.
                // Keep seek_sample above for the independent prepared PCM path.
                let video_seek = if prefix { Rational::zero() } else { seek };
                let span = source_end.checked_sub(video_seek)?;
                if prefix {
                    video_prefix_fallbacks.push(VideoPrefixFallback {
                        clip_id: c.id.clone(),
                        requested_seek: seek,
                        seek: Rational::zero(),
                        duration: span,
                        termination: InputTermination::FilterEof,
                    });
                } else {
                    args.extend([
                        "-seek_timestamp".into(),
                        "1".into(),
                        "-ss".into(),
                        number(seek.checked_add(source_origin)?.value()),
                        "-noaccurate_seek".into(),
                    ]);
                }
                args.extend(["-threads".into(), "2".into()]);
                input_ranges.push(InputRange {
                    clip_id: c.id.clone(),
                    seek: video_seek,
                    duration: span,
                    termination: InputTermination::FilterEof,
                });
            }
            args.extend(["-i".into(), path.clone()]);
            source_label = timing
                .as_ref()
                .filter(|_| !use_proxy)
                .and_then(|t| t.video_stream)
                .map(|stream| format!("{input}:{stream}"))
                .unwrap_or_else(|| format!("{input}:v:0"));
            audio_label = timing
                .as_ref()
                .filter(|_| !use_proxy)
                .and_then(|t| t.audio_stream)
                .map(|stream| format!("{input}:{stream}"))
                .unwrap_or_else(|| format!("{input}:a:0"));
            input += 1;
            audio_origin = source_origin;
            if region.is_some() && audio {
                let source_end = c
                    .source_in
                    .checked_add(
                        Rational::from_frames(clipped_end - c.start, p.fps).checked_mul(c.speed)?,
                    )?
                    .checked_add(Rational::new(1, 2))?;
                if crate::audio_clock::fits(m, settings.sample_rate) {
                    let prepared = crate::audio_clock::ensure(
                        ctx,
                        m,
                        use_proxies,
                        settings.sample_rate,
                        cancel,
                    )?;
                    asset_pins.extend(prepared.pins);
                    let seek = Rational::new(seek_sample as i64, settings.sample_rate as i64);
                    args.extend([
                        "-ss".into(),
                        number(seek.value()),
                        "-noaccurate_seek".into(),
                        "-t".into(),
                        number(source_end.checked_sub(seek)?.value()),
                        "-i".into(),
                        prepared.path.to_string_lossy().into_owned(),
                    ]);
                    audio_label = format!("{input}:a:0");
                    audio_origin = Rational::zero();
                    audio_seek_sample = seek_sample;
                    prepared_audio = true;
                } else {
                    // Keep long footage editable without allocating its entire
                    // PCM representation. Only audio decodes from the origin;
                    // its full-render sample-count clock remains exact.
                    args.extend([
                        "-t".into(),
                        number(source_end.value()),
                        "-i".into(),
                        path.clone(),
                    ]);
                    audio_label = timing
                        .as_ref()
                        .filter(|_| !use_proxy)
                        .and_then(|t| t.audio_stream)
                        .map(|stream| format!("{input}:{stream}"))
                        .unwrap_or_else(|| format!("{input}:a:0"));
                    audio_prefix_fallbacks.push(InputRange {
                        clip_id: c.id.clone(),
                        seek: Rational::zero(),
                        duration: source_end,
                        termination: InputTermination::InputDuration,
                    });
                }
                input += 1;
            }
            // Proxies preserve aspect ratio; scaling uses original metadata only for fit factors.
            (m.width.max(1), m.height.max(1))
        } else {
            if !ctx.font.is_file() {
                return Err("Bundled Inter font is missing; titles cannot render".into());
            }
            let text_file = ctx.cache_dir.join(format!("{job_id}-title-{index}.txt"));
            fs::write(&text_file, c.title.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
            source_label = format!("title{index}");
            graph.push(format!("color=c=black@0:s={}x{}:r={fps}:d={},format=rgba,settb={}/{},setpts=PTS+{clip_offset},drawtext=fontfile='{}':textfile='{}':fontcolor=white:fontsize={}:x=(w-text_w)/2:y=(h-text_h)/2:expansion=none[{source_label}]",settings.width,settings.height,seconds(clipped_end-clipped_start,p.fps),p.fps.den,p.fps.num,escaped_path(&ctx.font),escaped_path(&text_file),number(settings.height as f64/12.)));
            (settings.width, settings.height)
        };
        if video {
            let t = &c.transform;
            let cropw = (sw as f64 * (1. - t.crop_left - t.crop_right)).max(1.);
            let croph = (sh as f64 * (1. - t.crop_top - t.crop_bottom)).max(1.);
            let fit = (settings.width as f64 / cropw).min(settings.height as f64 / croph);
            let mut filters = vec![];
            if media.is_some_and(|m| m.kind == "image") {
                filters.push(format!("fps={fps}:round=near,settb={}/{},setpts=PTS+({}),trim=end_pts={},setpts=PTS-({render_offset})",p.fps.den,p.fps.num,render_offset+clip_offset,render_offset+clipped_end-c.start));
            } else if media.is_some() {
                let clock = if c.retime.is_some() {
                    match exact_retime_timebase(source_rate, render_origin, c.speed, video_end) {
                        Some(ticks) => format!(
                            "settb=1/{ticks},trim=start={}:end={},setpts=(PTS-({}))*{}/{}",
                            number(video_start.value()),
                            number(video_end.value()),
                            render_origin.num as i128 * ticks as i128 / render_origin.den as i128,
                            c.speed.den,
                            c.speed.num
                        ),
                        None => format!(
                            "settb=AVTB,trim=start={}:end={},setpts=(PTS-{}/TB)/{}",
                            number(video_start.value()),
                            number(video_end.value()),
                            rational_expression(render_origin),
                            rational_expression(c.speed)
                        ),
                    }
                } else {
                    format!(
                        "settb=AVTB,trim=start={}:end={},setpts=(PTS-{}/TB)/{}",
                        number(video_start.value()),
                        number(video_end.value()),
                        rational_expression(render_origin),
                        rational_expression(c.speed)
                    )
                };
                filters.push(format!(
                    "settb=AVTB,setpts=PTS-{}/TB,fps={source_fps}:round=near,{clock},fps={fps}:round=near,settb={}/{},trim=start_pts={}:end_pts={},setpts=PTS-({render_offset})",
                    rational_expression(source_origin),
                    p.fps.den, p.fps.num, render_offset+clip_offset, render_offset+clipped_end-c.start
                ));
            }
            filters.push(format!(
                "fps={fps}:round=near,trim=start=0:end={}",
                number(local)
            ));
            filters.push(format!(
                "crop=w=iw*{}:h=ih*{}:x=iw*{}:y=ih*{}",
                number(1. - t.crop_left - t.crop_right),
                number(1. - t.crop_top - t.crop_bottom),
                number(t.crop_left),
                number(t.crop_top)
            ));
            // Normalize originals and proxies to the same even fit before RGBA conversion.
            // Odd-width YUV-to-RGBA conversion can leave a transparent final column in scalar swscale.
            let even_fit = |value: f64| ((value.round().max(2.) as u32) / 2) * 2;
            filters.push(format!(
                "scale=w={}:h={}:flags=bicubic,setsar=1",
                even_fit(cropw * fit),
                even_fit(croph * fit)
            ));
            filters.push(format!(
                "eq=brightness={}:contrast={}:saturation={}",
                number(c.brightness),
                number(c.contrast),
                number(c.saturation)
            ));
            filters.push("format=rgba".into());
            if t.rotation != 0. {
                filters.push(format!(
                    "rotate={}:ow=rotw({}):oh=roth({}):c=none",
                    number(t.rotation.to_radians()),
                    number(t.rotation.to_radians()),
                    number(t.rotation.to_radians())
                ));
            }
            let opacity = expression(c, "opacity", "T", p.fps, c.opacity);
            if !has_keys(c, "opacity") {
                filters.push(format!("colorchannelmixer=aa={}", number(c.opacity)));
            } else {
                filters.push(format!(
                    "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='clip(alpha(X,Y)*({opacity})+0.000000001,0,255)'"
                ));
            }
            let (fade_in, fade_out, fade_start, fade_end) = fade_times(c, p.fps);
            let outgoing_dissolve = if c.retime.is_some()
                || p.clips
                    .iter()
                    .any(|next| next.track_id == c.track_id && next.retime.is_some())
            {
                is_outgoing_dissolve(c, p)?
            } else {
                c.fade_out > 0
                    && p.clips.iter().any(|next| {
                        next.id != c.id
                            && next.track_id == c.track_id
                            && next.fade_in == c.fade_out
                            && next.start + next.fade_in_start()
                                == c.start + c.fade_out_end() - c.fade_out
                            && next.composition_start() > c.composition_start()
                            && next.start < c.start + c.fade_out_end()
                            && next.end() > c.start + c.fade_out_end() - c.fade_out
                    })
            };
            if fade_in > 0. {
                if c.retime.is_some() {
                    video_fade_seconds(&mut filters, "in", fade_start, fade_in, p.fps);
                } else {
                    video_fade(&mut filters, "in", c.fade_in_start(), c.fade_in, p.fps);
                }
            }
            if fade_out > 0. && !outgoing_dissolve {
                if c.retime.is_some() {
                    video_fade_seconds(&mut filters, "out", fade_end - fade_out, fade_out, p.fps);
                } else {
                    video_fade(
                        &mut filters,
                        "out",
                        c.fade_out_end() - c.fade_out,
                        c.fade_out,
                        p.fps,
                    );
                }
            }
            // Pixel filters above use a stable canvas. FFmpeg's rotation and
            // geq filters cannot safely follow a scale with changing dimensions.
            // Scaling last also makes the canvas independent of fragment edges.
            let scale = expression(c, "scale", "t", p.fps, t.scale);
            filters.push(format!("scale=w='max(2,trunc(iw*({scale})/2+0.000000001)*2)':h='max(2,trunc(ih*({scale})/2+0.000000001)*2)':eval=frame"));
            filters.push(format!(
                "settb={}/{},setpts=PTS+{}",
                p.fps.den,
                p.fps.num,
                c.start - origin
            ));
            let label = format!("v{index}");
            graph.push(format!("[{source_label}]{}[{label}]", filters.join(",")));
            let position_clock = if c.retime.is_some() {
                format!(
                    "((round(t*{}/{})-{})*{}/{})",
                    p.fps.num,
                    p.fps.den,
                    c.start - origin,
                    p.fps.den,
                    p.fps.num
                )
            } else {
                format!(
                    "(round(t*{}/{})-{})",
                    p.fps.num,
                    p.fps.den,
                    c.start - origin
                )
            };
            let x = expression(c, "x", &position_clock, Rational::one(), t.x);
            let y = expression(c, "y", &position_clock, Rational::one(), t.y);
            let composed = format!("composite{index}");
            graph.push(format!("[{overlay}][{label}]overlay=x='(W-w)/2+({x})*{}':y='(H-h)/2+({y})*{}':eval=frame:eof_action=pass:repeatlast=0:format=rgb:enable='gte(n,{})*lt(n,{})'[{composed}]",number(settings.width as f64/p.width as f64),number(settings.height as f64/p.height as f64),clipped_start-origin,clipped_end-origin));
            overlay = composed;
        }
        if audio {
            let timeline_start = sample_boundary(
                Rational::from_frames(clipped_start, p.fps),
                settings.sample_rate,
            );
            let timeline_end = sample_boundary(
                Rational::from_frames(clipped_end, p.fps),
                settings.sample_rate,
            );
            let root_start = sample_boundary(
                Rational::from_frames(c.start, p.fps).checked_sub(phase)?,
                settings.sample_rate,
            );
            let output_origin =
                render_origin.checked_mul(Rational::new(c.speed.den, c.speed.num))?;
            let source_start =
                sample_boundary(output_origin, settings.sample_rate) + timeline_start - root_start;
            let source_end =
                sample_boundary(output_origin, settings.sample_rate) + timeline_end - root_start;
            let phase = Rational::new(timeline_start as i64, settings.sample_rate as i64)
                .checked_sub(Rational::from_frames(c.start, p.fps))?;
            let gain_time = format!("(t+{})", rational_expression(phase));
            let volume = expression(c, "volume", &gain_time, p.fps, c.volume);
            // Pad from the common origin before selecting samples. Only then
            // reset to the clip clock. Stream-local STARTPTS would erase delay.
            let resampled_clock = if prepared_audio {
                "anull"
            } else {
                "asetpts=N/SR/TB"
            };
            let mut filters = vec![format!(
                "asetpts=PTS-{}/TB,aresample={}:first_pts={audio_seek_sample},{resampled_clock},{},aformat=sample_fmts=fltp:channel_layouts=stereo,asettb=1/{},atrim=start_pts={source_start}:end_pts={source_end},asetpts=N",
                rational_expression(audio_origin),
                settings.sample_rate,
                varispeed(c.speed, settings.sample_rate),
                settings.sample_rate
            )];
            // This timebase already counts individual output samples. Dividing
            // N by SR and TB again can truncate an integral PTS by one sample,
            // changing automated gain when region/full decoder blocks differ.
            let (fade_in, fade_out, _, _) = fade_times(c, p.fps);
            if fade_in > 0. || fade_out > 0. || has_keys(c, "volume") {
                // Per-sample gain makes automation independent of decoder block
                // boundaries and of how many fragments a clip is split into.
                let gain = format!("({volume})*({})", fade_gain(c, &gain_time, p.fps));
                filters.push(format!(
                    "aeval=exprs='val(0)*({gain})|val(1)*({gain})':channel_layout=stereo"
                ));
            } else {
                filters.push(format!("volume={volume}"));
            }
            // Preserve float static gain, then use one common mix precision.
            // Otherwise inactive automation can change FFmpeg's negotiated
            // accumulator format between the full graph and a bounded region.
            filters.push("aformat=sample_fmts=dblp:channel_layouts=stereo".into());
            let origin_sample =
                sample_boundary(Rational::from_frames(origin, p.fps), settings.sample_rate);
            filters.push(format!("adelay={}S:all=1", timeline_start - origin_sample));
            let label = format!("a{index}");
            graph.push(format!("[{audio_label}]{}[{label}]", filters.join(",")));
            audio_labels.push(label);
        }
    }
    graph.push(format!("[{overlay}]trim=start_frame={}:end_frame={},setpts=PTS-STARTPTS,fps={export_fps},trim=end_frame={output_frames},scale={}:{}:flags=bicubic,setsar=1,format=yuv420p[vout]",start-origin,end-origin,settings.width,settings.height));
    let audio_inputs = audio_labels
        .iter()
        .map(|l| format!("[{l}]"))
        .collect::<String>();
    let mix_filter = if limit_mix {
        "alimiter=limit=0.97:latency=1"
    } else {
        "anull"
    };
    let origin_sample = sample_boundary(Rational::from_frames(origin, p.fps), settings.sample_rate);
    graph.push(format!("{audio_inputs}amix=inputs={}:duration=longest:normalize=0,{mix_filter},atrim=start_sample={}:end_sample={},asetpts=N/SR/TB[aout]",audio_labels.len(),sample_boundary(Rational::from_frames(start,p.fps), settings.sample_rate)-origin_sample,sample_boundary(Rational::from_frames(end,p.fps), settings.sample_rate)-origin_sample));
    let filter_graph = graph.join(";\n");
    let graph_path = ctx.cache_dir.join(format!("{job_id}-filters.txt"));
    fs::write(&graph_path, &filter_graph).map_err(|e| e.to_string())?;
    // A CLI video-frame limit can stop the muxer before its final audio block.
    // Bound both graph branches instead, and leave the time guard beyond both
    // exact EOFs so encoder/muxer draining cannot truncate a ceil sample tail.
    let audio_samples = sample_boundary(Rational::from_frames(end, p.fps), settings.sample_rate)
        - sample_boundary(Rational::from_frames(start, p.fps), settings.sample_rate);
    let mux_duration = (audio_samples as f64 / settings.sample_rate as f64)
        .max(output_frames as f64 * settings.fps.den as f64 / settings.fps.num as f64);
    let mux_guard = (mux_duration * 1_000_000.).ceil() / 1_000_000. + 0.000_001;
    args.extend([
        "-filter_complex_script".into(),
        graph_path.to_string_lossy().to_string(),
        "-map".into(),
        "[vout]".into(),
        "-map".into(),
        "[aout]".into(),
        "-t".into(),
        number(mux_guard),
        "-r".into(),
        export_fps,
    ]);
    if settings.codec == "h264" {
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            settings.crf.to_string(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{}k", settings.audio_bitrate),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    } else {
        args.extend([
            "-c:v".into(),
            "ffv1".into(),
            "-level".into(),
            "3".into(),
            "-coder".into(),
            "1".into(),
            "-context".into(),
            "1".into(),
            "-c:a".into(),
            "flac".into(),
        ]);
    }
    args.extend([
        "-ar".into(),
        settings.sample_rate.to_string(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        output.to_string_lossy().to_string(),
    ]);
    Ok(RenderPlan {
        args,
        duration,
        output_frames: output_frames as i64,
        filter_graph,
        working_region: PreviewRegion {
            start_frame: origin,
            end_frame: working_end,
        },
        input_ranges,
        asset_pins,
        audio_prefix_fallbacks,
        video_prefix_fallbacks,
        video_seek_checks,
    })
}
