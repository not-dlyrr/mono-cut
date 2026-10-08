//! A single deterministic filter compiler drives both program preview and exports.
//! Timeline positions stay rational until FFmpeg's explicit seconds/sample boundaries.
use crate::{
    media::{self, ensure_cache, MediaContext},
    model::*,
};
use std::{fs, path::Path};

#[derive(Debug)]
pub struct RenderPlan {
    pub args: Vec<String>,
    pub duration: f64,
    pub output_frames: i64,
    pub filter_graph: String,
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
    compile_with_purpose(ctx, p, settings, use_proxies, output, job_id, true)
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
    compile_with_purpose(ctx, p, settings, false, output, job_id, false)
}

fn compile_with_purpose(
    ctx: &MediaContext,
    p: &Project,
    settings: &ExportSettings,
    use_proxies: bool,
    output: &Path,
    job_id: &str,
    limit_mix: bool,
) -> Result<RenderPlan, String> {
    p.validate()?;
    validate_settings(settings)?;
    ensure_cache(ctx)?;
    if p.clips.is_empty() {
        return Err("Add clips to the timeline before rendering".into());
    }
    let sequence_duration = Rational::from_frames(p.length(), p.fps).value();
    let fps = format!("{}/{}", p.fps.num, p.fps.den);
    let export_fps = format!("{}/{}", settings.fps.num, settings.fps.den);
    let start = p.in_point.unwrap_or(0);
    let end = p.out_point.unwrap_or(p.length());
    let duration = Rational::from_frames(end - start, p.fps).value();
    let output_frames = (Rational::from_frames(end - start, p.fps).num as i128
        * settings.fps.num as i128
        + Rational::from_frames(end - start, p.fps).den as i128 * settings.fps.den as i128 / 2)
        / (Rational::from_frames(end - start, p.fps).den as i128 * settings.fps.den as i128);
    if output_frames < 1 {
        return Err("Export range is shorter than one output frame".into());
    }
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
        "1".into(),
    ];
    let mut graph = vec![
        format!(
            "color=c=black:s={}x{}:r={fps}:d={},format=rgba[basev]",
            p.width,
            p.height,
            number(sequence_duration)
        ),
        format!(
            "anullsrc=r={}:cl=stereo,atrim=end_sample={}[basea]",
            settings.sample_rate,
            sample_boundary(
                Rational::from_frames(p.length(), p.fps),
                settings.sample_rate
            )
        ),
    ];
    let mut overlay = "basev".to_string();
    let mut audio_labels = vec!["basea".to_string()];
    let mut input = 0;
    let mut ordered: Vec<_> = p
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(ti, t)| {
            p.clips
                .iter()
                .filter(move |c| c.track_id == t.id)
                .map(move |c| (ti, c))
        })
        .collect();
    ordered.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.composition_start().cmp(&b.1.composition_start()))
            .then(a.1.composition_group().cmp(b.1.composition_group()))
            .then(a.1.start.cmp(&b.1.start))
            .then(a.1.id.cmp(&b.1.id))
    });
    for (index, clip) in ordered.into_iter().enumerate() {
        let (_track_index, c) = clip;
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
                || c.keyframes
                    .iter()
                    .any(|k| k.property == "volume" && k.value > 0.));
        if !video && !audio {
            continue;
        }
        let local = Rational::from_frames(c.duration, p.fps).value();
        let render_offset = c.render_offset.unwrap_or(0);
        let render_origin = c
            .source_in
            .checked_sub(Rational::from_frames(render_offset, p.fps).checked_mul(c.speed)?)?;
        let source_label: String;
        let mut audio_label = String::new();
        let mut source_origin = Rational::zero();
        let mut video_start = Rational::zero();
        let mut video_end = media
            .map(|m| m.duration)
            .unwrap_or(Rational::from_frames(c.duration, p.fps));
        let mut source_fps = fps.clone();
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
            // Proxies preserve aspect ratio; scaling uses original metadata only for fit factors.
            (m.width.max(1), m.height.max(1))
        } else {
            if !ctx.font.is_file() {
                return Err("Bundled Inter font is missing; titles cannot render".into());
            }
            let text_file = ctx.cache_dir.join(format!("{job_id}-title-{index}.txt"));
            fs::write(&text_file, c.title.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
            source_label = format!("title{index}");
            graph.push(format!("color=c=black@0:s={}x{}:r={fps}:d={},format=rgba,drawtext=fontfile='{}':textfile='{}':fontcolor=white:fontsize={}:x=(w-text_w)/2:y=(h-text_h)/2:expansion=none[{source_label}]",p.width,p.height,number(local),escaped_path(&ctx.font),escaped_path(&text_file),number(p.height as f64/12.)));
            (p.width, p.height)
        };
        if video {
            let t = &c.transform;
            let cropw = (sw as f64 * (1. - t.crop_left - t.crop_right)).max(1.);
            let croph = (sh as f64 * (1. - t.crop_top - t.crop_bottom)).max(1.);
            let fit = (p.width as f64 / cropw).min(p.height as f64 / croph);
            let mut filters = vec![];
            if media.is_some() {
                filters.push(format!(
                    "settb=AVTB,setpts=PTS-{}/TB,fps={source_fps}:round=near,settb=AVTB,trim=start={}:end={},setpts=(PTS-{}/TB)/{},fps={fps}:round=near,settb={}/{},trim=start_pts={}:end_pts={},setpts=PTS-({render_offset})",
                    rational_expression(source_origin),
                    number(video_start.value()),
                    number(video_end.value()),
                    rational_expression(render_origin),
                    rational_expression(c.speed),
                    p.fps.den, p.fps.num, render_offset, render_offset+c.duration
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
            // Normalize dimensions first so original and proxy inputs use the identical transform model.
            filters.push(format!(
                "scale=w={}:h={}:flags=bicubic,setsar=1",
                (cropw * fit).round().max(2.) as u32,
                (croph * fit).round().max(2.) as u32
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
            if !c.keyframes.iter().any(|k| k.property == "opacity") {
                filters.push(format!("colorchannelmixer=aa={}", number(c.opacity)));
            } else {
                filters.push(format!(
                    "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='clip(alpha(X,Y)*({opacity})+0.000000001,0,255)'"
                ));
            }
            let outgoing_dissolve = c.fade_out > 0
                && p.clips.iter().any(|next| {
                    next.id != c.id
                        && next.track_id == c.track_id
                        && next.fade_in == c.fade_out
                        && next.start + next.fade_in_start()
                            == c.start + c.fade_out_end() - c.fade_out
                        && next.composition_start() > c.composition_start()
                        && next.start < c.start + c.fade_out_end()
                        && next.end() > c.start + c.fade_out_end() - c.fade_out
                });
            if c.fade_in > 0 {
                video_fade(&mut filters, "in", c.fade_in_start(), c.fade_in, p.fps);
            }
            if c.fade_out > 0 && !outgoing_dissolve {
                video_fade(
                    &mut filters,
                    "out",
                    c.fade_out_end() - c.fade_out,
                    c.fade_out,
                    p.fps,
                );
            }
            // Pixel filters above use a stable canvas. FFmpeg's rotation and
            // geq filters cannot safely follow a scale with changing dimensions.
            // Scaling last also makes the canvas independent of fragment edges.
            let scale = expression(c, "scale", "t", p.fps, t.scale);
            filters.push(format!("scale=w='max(2,trunc(iw*({scale})/2+0.000000001)*2)':h='max(2,trunc(ih*({scale})/2+0.000000001)*2)':eval=frame"));
            filters.push(format!(
                "settb={}/{},setpts=PTS+{}",
                p.fps.den, p.fps.num, c.start
            ));
            let label = format!("v{index}");
            graph.push(format!("[{source_label}]{}[{label}]", filters.join(",")));
            let x = expression(
                c,
                "x",
                &format!("(round(t*{}/{})-{})", p.fps.num, p.fps.den, c.start),
                Rational::one(),
                t.x,
            );
            let y = expression(
                c,
                "y",
                &format!("(round(t*{}/{})-{})", p.fps.num, p.fps.den, c.start),
                Rational::one(),
                t.y,
            );
            let composed = format!("composite{index}");
            graph.push(format!("[{overlay}][{label}]overlay=x='(W-w)/2+({x})':y='(H-h)/2+({y})':eval=frame:eof_action=pass:repeatlast=0:format=rgb:enable='gte(n,{})*lt(n,{})'[{composed}]",c.start,c.end()));
            overlay = composed;
        }
        if audio {
            let timeline_start =
                sample_boundary(Rational::from_frames(c.start, p.fps), settings.sample_rate);
            let timeline_end =
                sample_boundary(Rational::from_frames(c.end(), p.fps), settings.sample_rate);
            let root_start = sample_boundary(
                Rational::from_frames(c.start - render_offset, p.fps),
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
            let mut filters = vec![format!(
                "asetpts=PTS-{}/TB,aresample={}:first_pts=0,asetpts=N/SR/TB,{},aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=start_sample={source_start}:end_sample={source_end},asetpts=N/SR/TB",
                rational_expression(source_origin),
                settings.sample_rate,
                varispeed(c.speed, settings.sample_rate)
            )];
            if c.fade_in > 0 || c.fade_out > 0 || c.keyframes.iter().any(|k| k.property == "volume")
            {
                // Per-sample gain makes automation independent of decoder block
                // boundaries and of how many fragments a clip is split into.
                let gain = format!("({volume})*({})", fade_gain(c, &gain_time, p.fps));
                filters.push(format!(
                    "aeval=exprs='val(0)*({gain})|val(1)*({gain})':channel_layout=stereo"
                ));
            } else {
                filters.push(format!("volume={volume}"));
            }
            filters.push(format!("adelay={timeline_start}S:all=1"));
            let label = format!("a{index}");
            graph.push(format!("[{audio_label}]{}[{label}]", filters.join(",")));
            audio_labels.push(label);
        }
    }
    graph.push(format!("[{overlay}]trim=start_frame={start}:end_frame={end},setpts=PTS-STARTPTS,fps={export_fps},trim=end_frame={output_frames},scale={}:{}:flags=bicubic,setsar=1,format=yuv420p[vout]",settings.width,settings.height));
    let audio_inputs = audio_labels
        .iter()
        .map(|l| format!("[{l}]"))
        .collect::<String>();
    let mix_filter = if limit_mix {
        "alimiter=limit=0.97:latency=1"
    } else {
        "anull"
    };
    graph.push(format!("{audio_inputs}amix=inputs={}:duration=longest:normalize=0,{mix_filter},atrim=start_sample={}:end_sample={},asetpts=N/SR/TB[aout]",audio_labels.len(),sample_boundary(Rational::from_frames(start,p.fps), settings.sample_rate),sample_boundary(Rational::from_frames(end,p.fps), settings.sample_rate)));
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
    })
}
