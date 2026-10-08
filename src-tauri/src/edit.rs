use crate::model::*;
use serde_json::Value;
use std::collections::HashSet;

fn compare(a: Rational, b: Rational) -> std::cmp::Ordering {
    (a.num as i128 * b.den as i128).cmp(&(b.num as i128 * a.den as i128))
}
fn source_frames(frames: i64, fps: Rational, speed: Rational) -> Result<Rational, String> {
    Rational::from_frames(frames, fps).checked_mul(speed)
}
fn timeline_frames(time: Rational, speed: Rational, fps: Rational) -> Result<i64, String> {
    let time = time.checked_mul(Rational::new(speed.den, speed.num))?;
    let n = time.num as i128 * fps.num as i128;
    let d = time.den as i128 * fps.den as i128;
    i64::try_from(n.div_euclid(d)).map_err(|_| "Retiming exceeds timeline limits".into())
}
fn valid_key(property: &str, value: f64) -> bool {
    value.is_finite()
        && match property {
            "opacity" => (0.0..=1.0).contains(&value),
            "volume" => (0.0..=8.0).contains(&value),
            "scale" => (0.01..=10.).contains(&value),
            "x" | "y" => value.abs() <= 100000.,
            _ => false,
        }
}

fn require(cond: bool, msg: &str) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(msg.into())
    }
}
pub fn validate_project(p: &Project) -> Result<(), String> {
    require(p.version == 1, "Unsupported project format version")?;
    require(
        (16..=16384).contains(&p.width) && (16..=16384).contains(&p.height),
        "Resolution must be between 16 and 16384",
    )?;
    p.fps.validate(true)?;
    require(
        p.fps.value() >= 1. && p.fps.value() <= 240.,
        "Frame rate must be between 1 and 240",
    )?;
    require(
        (8000..=192000).contains(&p.sample_rate),
        "Invalid sample rate",
    )?;
    require(
        !p.name.trim().is_empty() && p.name.len() <= 512,
        "Project name must be 1–512 characters",
    )?;
    require(
        p.clips.len() <= 10000 && p.media.len() <= 10000 && p.tracks.len() <= 128,
        "Project exceeds supported limits",
    )?;
    let mut ids = HashSet::new();
    for t in &p.tracks {
        require(ids.insert(t.id.clone()), "Duplicate track id")?;
        require(t.kind == "video" || t.kind == "audio", "Invalid track kind")?;
    }
    ids.clear();
    for m in &p.media {
        require(ids.insert(m.id.clone()), "Duplicate media id")?;
        m.duration.validate(false)?;
        m.fps.validate(false)?;
        require(m.duration.num >= 0, "Negative media duration")?;
        if let Some(legacy) = &m.legacy_source {
            legacy.duration.validate(true)?;
            let timing = m
                .timing
                .as_ref()
                .ok_or("Legacy source compatibility needs measured timing")?;
            timing.origin.validate(false)?;
            if let Some(duration) = timing.container_duration {
                duration.validate(false)?;
            }
            require(
                m.kind != "image" && timing.origin.num > 0,
                "Invalid legacy source compatibility",
            )?;
            require(
                m.duration.checked_sub(legacy.duration)?.num > 0,
                "Legacy source extent must include its historical tail",
            )?;
            let historical = timing
                .container_duration
                .unwrap_or(timing.origin.checked_add(legacy.duration)?);
            require(
                m.duration.checked_sub(historical)?.num == 0,
                "Legacy extent must equal its historical container duration",
            )?;
            if let Some(hash) = &legacy.sha256 {
                require(
                    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "Invalid legacy source content hash",
                )?;
            }
        }
        if let Some(timing) = &m.timing {
            timing.origin.validate(false)?;
            if let Some(duration) = timing.container_duration {
                duration.validate(false)?;
                require(duration.num >= 0, "Negative container duration")?;
            }
            let mut starts = vec![];
            for start in [timing.video_start, timing.audio_start]
                .into_iter()
                .flatten()
            {
                start.validate(false)?;
                require(
                    start.checked_sub(timing.origin)?.num >= 0,
                    "Media stream precedes its shared origin",
                )?;
                starts.push(start);
            }
            require(
                starts.iter().any(|start| {
                    start
                        .checked_sub(timing.origin)
                        .map(|delta| delta.num == 0)
                        .unwrap_or(false)
                }),
                "Media origin must equal the earliest selected stream start",
            )?;
            for (start, end) in [
                (timing.video_start, timing.video_end),
                (timing.audio_start, timing.audio_end),
            ] {
                if let Some(end) = end {
                    end.validate(false)?;
                    require(
                        start
                            .map(|start| {
                                end.checked_sub(start)
                                    .map(|delta| delta.num >= 0)
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false),
                        "Invalid media stream end",
                    )?;
                }
            }
            if let Some(legacy) = &m.legacy_source {
                let ends = [timing.video_end, timing.audio_end];
                for end in ends.into_iter().flatten() {
                    require(
                        legacy
                            .duration
                            .checked_sub(end.checked_sub(timing.origin)?)?
                            .num
                            >= 0,
                        "Legacy measured span cannot precede a known stream end",
                    )?;
                }
                let complete = [
                    (timing.video_start, timing.video_end),
                    (timing.audio_start, timing.audio_end),
                ]
                .into_iter()
                .all(|(start, end)| start.is_none() || end.is_some());
                if complete {
                    let latest = ends.into_iter().flatten().reduce(|a, b| {
                        if a.checked_sub(b)
                            .map(|difference| difference.num >= 0)
                            .unwrap_or(false)
                        {
                            a
                        } else {
                            b
                        }
                    });
                    if let Some(end) = latest {
                        require(
                            legacy
                                .duration
                                .checked_sub(end.checked_sub(timing.origin)?)?
                                .num
                                == 0,
                            "Legacy measured span must match the selected stream ends",
                        )?;
                    }
                }
            }
            require(
                timing
                    .video_stream
                    .map(|index| index <= 65535)
                    .unwrap_or(true)
                    && timing
                        .audio_stream
                        .map(|index| index <= 65535)
                        .unwrap_or(true),
                "Invalid media stream index",
            )?;
        }
        require(
            m.proxy_timing_version
                .map(|version| version == NORMALIZED_TIMING_VERSION)
                .unwrap_or(true),
            "Unsupported proxy timing version",
        )?;
        require(
            ["video", "audio", "image"].contains(&m.kind.as_str()),
            "Invalid media kind",
        )?;
        require(
            m.waveform.iter().all(|value| value.is_finite()),
            "Media waveform values must be finite numbers",
        )?;
    }
    ids.clear();
    for c in &p.clips {
        require(ids.insert(c.id.clone()), "Duplicate clip id")?;
        validate_clip(p, c)?;
    }
    require(
        p.in_point
            .map(|v| v >= 0 && v <= p.length())
            .unwrap_or(true),
        "Invalid in point",
    )?;
    require(
        p.out_point
            .map(|v| v > 0 && v <= p.length())
            .unwrap_or(true),
        "Invalid out point",
    )?;
    require(
        p.out_point.unwrap_or(p.length()) > p.in_point.unwrap_or(0),
        "Out point must be after in point",
    )?;
    for m in &p.markers {
        require(
            m.frame >= 0 && m.frame <= 10_000_000,
            "Invalid marker frame",
        )?;
    }
    Ok(())
}
pub fn validate_clip(p: &Project, c: &Clip) -> Result<(), String> {
    require(
        c.start >= 0
            && c.start <= 10_000_000
            && c.duration > 0
            && c.duration <= 10_000_000
            && c.end() <= 10_000_000,
        "Clip must have positive duration and nonnegative start",
    )?;
    let track = p
        .tracks
        .iter()
        .find(|t| t.id == c.track_id)
        .ok_or("Clip track does not exist")?;
    c.source_in.validate(false)?;
    c.speed.validate(true)?;
    require(
        c.source_in.num >= 0 && c.speed.value() >= 0.05 && c.speed.value() <= 32.,
        "Invalid source offset or speed",
    )?;
    if let Some(ref mid) = c.media_id {
        let m = p
            .media
            .iter()
            .find(|m| &m.id == mid)
            .ok_or("Clip media does not exist")?;
        require(
            !(track.kind == "audio" && !m.has_audio),
            "This source has no audio",
        )?;
        require(
            !(track.kind == "video" && m.kind == "audio"),
            "Audio media needs an audio track",
        )?;
        if m.kind != "image" {
            let end = c
                .source_in
                .checked_add(Rational::from_frames(c.duration, p.fps).checked_mul(c.speed)?)?;
            require(
                end.value() <= m.duration.value() + 0.000_001,
                "Clip extends beyond available source media",
            )?;
        }
    } else {
        require(
            c.title.is_some() && track.kind == "video",
            "A title needs a video track",
        )?;
    }
    if let Some(t) = &c.title {
        require(t.len() <= 16384, "Title is too long")?;
    }
    let t = &c.transform;
    for v in [
        t.x,
        t.y,
        t.scale,
        t.rotation,
        t.crop_left,
        t.crop_right,
        t.crop_top,
        t.crop_bottom,
        c.opacity,
        c.volume,
        c.brightness,
        c.contrast,
        c.saturation,
    ] {
        require(v.is_finite(), "Clip properties must be finite numbers")?;
    }
    require(
        (0.01..=10.).contains(&t.scale)
            && t.x.abs() <= 100000.
            && t.y.abs() <= 100000.
            && t.rotation.abs() <= 36000.,
        "Invalid clip transform",
    )?;
    require(
        [t.crop_left, t.crop_right, t.crop_top, t.crop_bottom]
            .iter()
            .all(|v| (0.0..1.0).contains(v))
            && t.crop_left + t.crop_right < 0.99
            && t.crop_top + t.crop_bottom < 0.99,
        "Crop fractions must leave visible footage",
    )?;
    require(
        (0.0..=1.0).contains(&c.opacity)
            && (0.0..=8.0).contains(&c.volume)
            && (-1.0..=1.0).contains(&c.brightness)
            && (0.0..=4.0).contains(&c.contrast)
            && (0.0..=4.0).contains(&c.saturation),
        "Invalid opacity, volume, or color adjustment",
    )?;
    require(
        (0..=10_000_000).contains(&c.fade_in)
            && (0..=10_000_000).contains(&c.fade_out)
            && (c.fade_in_start.is_some() || c.fade_in <= c.duration)
            && (c.fade_out_end.is_some() || c.fade_out <= c.duration),
        "Invalid fade duration",
    )?;
    require(
        c.fade_in_start
            .into_iter()
            .chain(c.fade_out_end)
            .all(|anchor| (-20_000_000..=20_000_000).contains(&anchor)),
        "Invalid inherited fade anchor",
    )?;
    if let Some(origin) = &c.composition {
        require(
            !origin.group_id.is_empty()
                && origin.group_id.len() <= 512
                && (-10_000_000..=10_000_000).contains(&origin.offset),
            "Invalid compositing origin",
        )?;
    }
    if let Some(offset) = c.render_offset {
        require(
            (-10_000_000..=10_000_000).contains(&offset),
            "Invalid inherited render offset",
        )?;
        if c.retime.is_none() {
            require(
                c.source_in
                    .checked_sub(Rational::from_frames(offset, p.fps).checked_mul(c.speed)?)?
                    .num
                    >= 0,
                "Invalid inherited source origin",
            )?;
        }
    }
    let mut keys = HashSet::new();
    for k in &c.keyframes {
        require(
            k.frame >= 0 && k.frame <= c.duration && k.value.is_finite(),
            "Invalid keyframe time or value",
        )?;
        require(
            keys.insert((k.property.clone(), k.frame)),
            "Duplicate keyframe at the same frame",
        )?;
        require(
            valid_key(&k.property, k.value),
            "Invalid keyframe property or range",
        )?;
    }
    if let Some(r) = &c.retime {
        let m = c
            .media_id
            .as_ref()
            .and_then(|id| p.media.iter().find(|m| &m.id == id))
            .ok_or("Retiming requires video or audio media")?;
        require(
            m.kind != "image" && c.title.is_none(),
            "Titles and still images do not support speed changes",
        )?;
        r.source_span.validate(true)?;
        require(
            timeline_frames(r.source_span, c.speed, p.fps)? == c.duration,
            "Retained source span does not match clip duration",
        )?;
        require(
            compare(c.source_in.checked_add(r.source_span)?, m.duration).is_le(),
            "Retained interval extends beyond available source media",
        )?;
        r.render_source_origin.validate(false)?;
        let phase = timeline_frames(
            c.source_in.checked_sub(r.render_source_origin)?,
            c.speed,
            p.fps,
        )?;
        require(
            r.render_source_origin.num >= 0 && (-10_000_000..=10_000_000).contains(&phase),
            "Invalid retained source conversion origin",
        )?;
        if let Some(offset) = r.composition_source_offset {
            offset.validate(false)?;
            require(
                c.composition.is_some(),
                "Retained composition offset needs a composition group",
            )?;
        }
        for duration in [r.envelope.fade_in, r.envelope.fade_out] {
            duration.validate(false)?;
            require(duration.num >= 0, "Invalid source fade duration")?;
        }
        r.envelope.fade_in_start.validate(false)?;
        r.envelope.fade_out_end.validate(false)?;
        let mut keys = HashSet::new();
        for k in &r.envelope.keyframes {
            k.time.validate(false)?;
            require(
                k.time.num >= 0
                    && compare(k.time, r.source_span).is_le()
                    && valid_key(&k.property, k.value),
                "Invalid source-relative keyframe",
            )?;
            let normalized = Rational::new(k.time.num, k.time.den);
            require(
                keys.insert((k.property.clone(), normalized.num, normalized.den)),
                "Duplicate source-relative keyframe",
            )?;
        }
        let mut expected = c.clone();
        project_retime(&mut expected, p.fps)?;
        let mut stored_keys = c.keyframes.clone();
        let mut projected_keys = expected.keyframes.clone();
        let order =
            |a: &Keyframe, b: &Keyframe| a.property.cmp(&b.property).then(a.frame.cmp(&b.frame));
        stored_keys.sort_by(order);
        projected_keys.sort_by(order);
        require(
            stored_keys == projected_keys
                && c.fade_in == expected.fade_in
                && c.fade_out == expected.fade_out
                && c.fade_in_start() == expected.fade_in_start()
                && c.fade_out_end() == expected.fade_out_end()
                && c.composition.as_ref().map(|o| o.offset)
                    == expected.composition.as_ref().map(|o| o.offset),
            "Retiming controls disagree with their retained source envelope",
        )?;
    }
    Ok(())
}
fn ensure_unlocked(p: &Project, ids: &[String]) -> Result<(), String> {
    for id in ids {
        let c = p
            .clips
            .iter()
            .find(|c| &c.id == id)
            .ok_or("Clip not found")?;
        if p.tracks.iter().any(|t| t.id == c.track_id && t.locked) {
            return Err("Track is locked".into());
        }
    }
    Ok(())
}
fn expanded(p: &Project, ids: &[String]) -> Vec<String> {
    let links: HashSet<_> = p
        .clips
        .iter()
        .filter(|c| ids.contains(&c.id))
        .filter_map(|c| c.linked_id.clone())
        .collect();
    p.clips
        .iter()
        .filter(|c| {
            ids.contains(&c.id)
                || c.linked_id
                    .as_ref()
                    .map(|l| links.contains(l))
                    .unwrap_or(false)
        })
        .map(|c| c.id.clone())
        .collect()
}
pub fn evaluate(c: &Clip, property: &str, frame: f64, base: f64) -> f64 {
    let mut keys: Vec<_> = c
        .keyframes
        .iter()
        .filter(|k| k.property == property)
        .collect();
    keys.sort_by_key(|k| k.frame);
    if keys.is_empty() {
        return base;
    }
    if frame <= keys[0].frame as f64 {
        return keys[0].value;
    }
    for w in keys.windows(2) {
        if frame <= w[1].frame as f64 {
            let t = (frame - w[0].frame as f64) / (w[1].frame - w[0].frame) as f64;
            return w[0].value + (w[1].value - w[0].value) * t;
        }
    }
    keys.last().unwrap().value
}
fn base(c: &Clip, p: &str) -> f64 {
    match p {
        "opacity" => c.opacity,
        "volume" => c.volume,
        "scale" => c.transform.scale,
        "x" => c.transform.x,
        "y" => c.transform.y,
        _ => 0.,
    }
}
fn rebase_keys(c: &mut Clip, offset: i64, new_duration: i64) {
    let original = c.clone();
    let props: HashSet<_> = c.keyframes.iter().map(|k| k.property.clone()).collect();
    c.keyframes
        .retain(|k| k.frame >= offset && k.frame <= offset + new_duration);
    for k in &mut c.keyframes {
        k.frame -= offset;
    }
    for prop in props {
        if !c
            .keyframes
            .iter()
            .any(|k| k.property == prop && k.frame == 0)
        {
            c.keyframes.push(Keyframe {
                value: evaluate(&original, &prop, offset as f64, base(&original, &prop)),
                property: prop.clone(),
                frame: 0,
            });
        }
        // The exclusive-out control point preserves interpolation through the
        // last frame's audio samples; duration-1 would flatten that interval.
        let f = new_duration;
        if !c
            .keyframes
            .iter()
            .any(|k| k.property == prop && k.frame == f)
        {
            c.keyframes.push(Keyframe {
                value: evaluate(
                    &original,
                    &prop,
                    (offset + f) as f64,
                    base(&original, &prop),
                ),
                property: prop,
                frame: f,
            });
        }
    }
}
fn inherit_envelopes(c: &mut Clip, offset: i64) {
    c.fade_in_start = Some(c.fade_in_start() - offset);
    c.fade_out_end = Some(c.fade_out_end() - offset);
    let origin = c.composition.get_or_insert_with(|| CompositionOrigin {
        group_id: c.id.clone(),
        offset: 0,
    });
    origin.offset += offset;
    if let Some(r) = &mut c.retime {
        r.composition_source_offset.get_or_insert(Rational::zero());
    }
    if c.media_id.is_some() {
        c.render_offset = Some(c.render_offset.unwrap_or(0) + offset);
    }
}
fn default_clip(track_id: String, start: i64, duration: i64) -> Clip {
    Clip {
        id: id(),
        media_id: None,
        track_id,
        name: String::new(),
        start,
        duration,
        source_in: Rational::zero(),
        speed: Rational::one(),
        linked_id: None,
        title: None,
        transform: Transform::default(),
        opacity: 1.,
        volume: 1.,
        fade_in: 0,
        fade_out: 0,
        fade_in_start: None,
        fade_out_end: None,
        composition: None,
        render_offset: None,
        retime: None,
        brightness: 0.,
        contrast: 1.,
        saturation: 1.,
        keyframes: vec![],
    }
}
fn capture_retime(c: &Clip, fps: Rational) -> Result<ClipRetime, String> {
    Ok(ClipRetime {
        source_span: source_frames(c.duration, fps, c.speed)?,
        envelope: SourceEnvelope {
            keyframes: c
                .keyframes
                .iter()
                .map(|k| {
                    Ok(SourceKeyframe {
                        property: k.property.clone(),
                        time: source_frames(k.frame, fps, c.speed)?,
                        value: k.value,
                    })
                })
                .collect::<Result<_, String>>()?,
            fade_in: source_frames(c.fade_in, fps, c.speed)?,
            fade_out: source_frames(c.fade_out, fps, c.speed)?,
            fade_in_start: source_frames(c.fade_in_start(), fps, c.speed)?,
            fade_out_end: source_frames(c.fade_out_end(), fps, c.speed)?,
        },
        render_source_origin: c.source_in.checked_sub(source_frames(
            c.render_offset.unwrap_or(0),
            fps,
            c.speed,
        )?)?,
        composition_source_offset: c
            .composition
            .as_ref()
            .map(|o| source_frames(o.offset, fps, c.speed))
            .transpose()?,
    })
}
fn project_retime(c: &mut Clip, fps: Rational) -> Result<(), String> {
    let Some(r) = &c.retime else { return Ok(()) };
    // Coincident display keys are merged only on the integer UI grid. Exact
    // canonical control points remain available to the renderer and future rates.
    let mut keys = r.envelope.keyframes.iter().collect::<Vec<_>>();
    keys.sort_by(|a, b| compare(a.time, b.time));
    let mut projected = std::collections::BTreeMap::new();
    for k in keys {
        let frame = timeline_frames(k.time, c.speed, fps)?;
        if (0..=c.duration).contains(&frame) {
            projected.insert(
                (k.property.clone(), frame),
                Keyframe {
                    property: k.property.clone(),
                    frame,
                    value: k.value,
                },
            );
        }
    }
    c.keyframes = projected.into_values().collect();
    c.fade_in = timeline_frames(r.envelope.fade_in, c.speed, fps)?;
    c.fade_out = timeline_frames(r.envelope.fade_out, c.speed, fps)?;
    c.fade_in_start = Some(timeline_frames(r.envelope.fade_in_start, c.speed, fps)?);
    c.fade_out_end = Some(timeline_frames(r.envelope.fade_out_end, c.speed, fps)?);
    if let Some(origin) = &mut c.composition {
        origin.offset = timeline_frames(
            r.composition_source_offset.unwrap_or(Rational::zero()),
            c.speed,
            fps,
        )?;
    }
    // The exact source clock supersedes the legacy integer conversion offset.
    c.render_offset = None;
    Ok(())
}
fn evaluate_source(envelope: &SourceEnvelope, property: &str, time: Rational) -> Option<f64> {
    let mut keys: Vec<_> = envelope
        .keyframes
        .iter()
        .filter(|k| k.property == property)
        .collect();
    keys.sort_by(|a, b| compare(a.time, b.time));
    let first = keys.first()?;
    if compare(time, first.time).is_le() {
        return Some(first.value);
    }
    for pair in keys.windows(2) {
        if compare(time, pair[1].time).is_le() {
            let fraction = time.checked_sub(pair[0].time).ok()?.value()
                / pair[1].time.checked_sub(pair[0].time).ok()?.value();
            return Some(pair[0].value + (pair[1].value - pair[0].value) * fraction);
        }
    }
    Some(keys.last()?.value)
}
fn retime_interval(
    c: &mut Clip,
    offset: Rational,
    span: Rational,
    fps: Rational,
) -> Result<(), String> {
    let Some(r) = &mut c.retime else {
        return Ok(());
    };
    let original = r.envelope.clone();
    let end = offset.checked_add(span)?;
    r.envelope
        .keyframes
        .retain(|k| compare(k.time, offset).is_ge() && compare(k.time, end).is_le());
    for k in &mut r.envelope.keyframes {
        k.time = k.time.checked_sub(offset)?;
    }
    let properties: HashSet<_> = original
        .keyframes
        .iter()
        .map(|k| k.property.clone())
        .collect();
    for property in properties {
        for (time, before) in [(Rational::zero(), offset), (span, end)] {
            if !r
                .envelope
                .keyframes
                .iter()
                .any(|k| k.property == property && compare(k.time, time).is_eq())
            {
                r.envelope.keyframes.push(SourceKeyframe {
                    property: property.clone(),
                    time,
                    value: evaluate_source(&original, &property, before).unwrap(),
                });
            }
        }
    }
    r.envelope.fade_in_start = r.envelope.fade_in_start.checked_sub(offset)?;
    r.envelope.fade_out_end = r.envelope.fade_out_end.checked_sub(offset)?;
    if let Some(composition) = &mut r.composition_source_offset {
        *composition = composition.checked_add(offset)?;
    }
    r.source_span = span;
    project_retime(c, fps)
}
fn retime_clip(p: &mut Project, id: &str, speed: Rational) -> Result<(), String> {
    speed.validate(true)?;
    require(
        speed.num as i128 * 20 >= speed.den as i128 && speed.num as i128 <= speed.den as i128 * 32,
        "Speed must be between 0.05 and 32",
    )?;
    let speed = Rational::new(speed.num, speed.den);
    p.validate()?;
    let selected = p
        .clips
        .iter()
        .find(|c| c.id == id)
        .ok_or("Clip not found")?;
    let ids = expanded(p, &[id.to_owned()]);
    ensure_unlocked(p, &ids)?;
    let mut next = p.clone();
    for c in next.clips.iter_mut().filter(|c| ids.contains(&c.id)) {
        require(
            c.start == selected.start && compare(c.speed, selected.speed).is_eq(),
            "Linked clips must share a start and speed. Align them or unlink before changing speed",
        )?;
        let media = p
            .media
            .iter()
            .find(|m| c.media_id.as_ref() == Some(&m.id))
            .ok_or("Titles and still images do not support speed changes")?;
        require(
            media.kind != "image" && c.title.is_none(),
            "Titles and still images do not support speed changes",
        )?;
        require(
            !media.missing && std::path::Path::new(&media.path).is_file(),
            "Relink missing media before changing speed",
        )?;
        let mut state = c
            .retime
            .clone()
            .map(Ok)
            .unwrap_or_else(|| capture_retime(c, p.fps))?;
        if let Some(offset) = &mut state.composition_source_offset {
            // Compositing order is anchored in sequence time, independently of
            // the physical source-conversion phase retained below.
            *offset = offset
                .checked_mul(speed)?
                .checked_mul(Rational::new(c.speed.den, c.speed.num))?;
        }
        let duration = timeline_frames(state.source_span, speed, p.fps)?;
        require(
            duration >= 1
                && duration <= 10_000_000
                && c.start
                    .checked_add(duration)
                    .map(|e| e <= 10_000_000)
                    .unwrap_or(false),
            "Speed would make the clip shorter than one frame or exceed timeline limits",
        )?;
        c.speed = speed;
        c.duration = duration;
        c.retime = Some(state);
        project_retime(c, p.fps)?;
    }
    for a in next.clips.iter().filter(|c| ids.contains(&c.id)) {
        for b in &next.clips {
            if a.id == b.id || a.track_id != b.track_id {
                continue;
            }
            let old_a = p.clips.iter().find(|c| c.id == a.id).unwrap();
            let old_b = p.clips.iter().find(|c| c.id == b.id).unwrap();
            let overlap = |x: &Clip, y: &Clip| (x.end().min(y.end()) - x.start.max(y.start)).max(0);
            require(
                overlap(a, b) <= overlap(old_a, old_b),
                "Speed would overlap another clip on this track. Move the neighbor or trim first",
            )?;
        }
    }
    // Unlike ordinary edits, retiming never silently changes an explicit range.
    next.validate()?;
    *p = next;
    Ok(())
}
fn patch_clip(c: &mut Clip, patch: Value, fps: Rational) -> Result<(), String> {
    let map = patch.as_object().ok_or("Clip patch must be an object")?;
    let allowed = [
        "name",
        "start",
        "duration",
        "source_in",
        "speed",
        "transform",
        "opacity",
        "volume",
        "fade_in",
        "fade_out",
        "brightness",
        "contrast",
        "saturation",
        "keyframes",
        "title",
    ];
    let before = c.clone();
    let mut value = serde_json::to_value(&*c).map_err(|e| e.to_string())?;
    for (key, v) in map {
        require(
            allowed.contains(&key.as_str()),
            "Unknown or immutable clip property",
        )?;
        if key == "transform" {
            let fields = v.as_object().ok_or("Transform must be an object")?;
            for (k, x) in fields {
                require(
                    [
                        "x",
                        "y",
                        "scale",
                        "rotation",
                        "crop_left",
                        "crop_right",
                        "crop_top",
                        "crop_bottom",
                    ]
                    .contains(&k.as_str()),
                    "Unknown transform property",
                )?;
                value["transform"][k] = x.clone();
            }
        } else {
            value[key] = v.clone();
        }
    }
    *c = serde_json::from_value(value).map_err(|e| format!("Invalid clip patch: {e}"))?;
    // A deliberate fade edit starts a fresh envelope at this fragment's edge.
    if map.contains_key("fade_in") {
        c.fade_in_start = None;
    }
    if map.contains_key("fade_out") {
        c.fade_out_end = None;
    }
    if map.contains_key("speed") || map.contains_key("source_in") {
        c.render_offset = None;
    }
    if let Some(r) = &mut c.retime {
        if map.contains_key("keyframes") {
            let properties: HashSet<_> = before
                .keyframes
                .iter()
                .chain(&c.keyframes)
                .map(|k| k.property.clone())
                .collect();
            for property in properties {
                let mut old: Vec<_> = before
                    .keyframes
                    .iter()
                    .filter(|k| k.property == property)
                    .collect();
                let mut new: Vec<_> = c
                    .keyframes
                    .iter()
                    .filter(|k| k.property == property)
                    .collect();
                old.sort_by_key(|k| k.frame);
                new.sort_by_key(|k| k.frame);
                if old != new {
                    r.envelope.keyframes.retain(|k| k.property != property);
                    for k in new {
                        r.envelope.keyframes.push(SourceKeyframe {
                            property: k.property.clone(),
                            time: source_frames(k.frame, fps, c.speed)?,
                            value: k.value,
                        });
                    }
                }
            }
        }
        if map.contains_key("fade_in") && c.fade_in != before.fade_in {
            r.envelope.fade_in = source_frames(c.fade_in, fps, c.speed)?;
            r.envelope.fade_in_start = Rational::zero();
        }
        if map.contains_key("fade_out") && c.fade_out != before.fade_out {
            r.envelope.fade_out = source_frames(c.fade_out, fps, c.speed)?;
            r.envelope.fade_out_end = source_frames(c.duration, fps, c.speed)?;
        }
        if map.contains_key("source_in") && !compare(c.source_in, before.source_in).is_eq() {
            r.render_source_origin = c.source_in;
        }
    }
    if c.retime.is_some() {
        if map.contains_key("duration") && c.duration != before.duration {
            retime_interval(
                c,
                Rational::zero(),
                source_frames(c.duration, fps, c.speed)?,
                fps,
            )?;
        } else {
            project_retime(c, fps)?;
        }
    }
    Ok(())
}
pub fn apply(p: &mut Project, cmd: EditCommand) -> Result<(), String> {
    if let EditCommand::RetimeClip { id, speed } = &cmd {
        return retime_clip(p, id, *speed);
    }
    if let EditCommand::UpdateClip { id, patch } = &cmd {
        if let Some(map) = patch.as_object().filter(|map| map.contains_key("speed")) {
            require(
                map.len() == 1,
                "Change speed with retime_clip separately from other clip properties",
            )?;
            let speed = serde_json::from_value(map["speed"].clone())
                .map_err(|e| format!("Invalid speed: {e}"))?;
            return retime_clip(p, id, speed);
        }
    }
    let explicit_range = matches!(&cmd, EditCommand::SetRange { .. });
    match cmd {
        EditCommand::AddClip {
            media_id,
            track_id,
            start,
            source_in,
            duration,
        } => {
            let m = p
                .media
                .iter()
                .find(|m| m.id == media_id)
                .ok_or("Media not found")?;
            let offset = source_in.unwrap_or(Rational::zero());
            offset.validate(false)?;
            require(offset.num >= 0, "Source offset cannot be negative")?;
            let remaining = m
                .duration
                .checked_add(Rational::new(-offset.num, offset.den))?;
            let d = duration.unwrap_or(if m.kind == "image" {
                Rational::new(5, 1).frames_floor(p.fps)
            } else {
                remaining.frames_floor(p.fps)
            });
            if p.tracks.iter().any(|t| t.id == track_id && t.locked) {
                return Err("Track is locked".into());
            }
            let mut c = default_clip(track_id, start, d);
            c.name = m.name.clone();
            c.media_id = Some(media_id);
            c.source_in = offset;
            p.clips.push(c);
        }
        EditCommand::AddTitle {
            track_id,
            start,
            duration,
            text,
        } => {
            if p.tracks.iter().any(|t| t.id == track_id && t.locked) {
                return Err("Track is locked".into());
            }
            let mut c = default_clip(track_id, start, duration);
            c.name = "Title".into();
            c.title = Some(text);
            p.clips.push(c);
        }
        EditCommand::Move {
            ids,
            delta,
            track_id,
        } => {
            require(
                delta.unsigned_abs() <= 10_000_000,
                "Move exceeds sequence limits",
            )?;
            // The first selected clip anchors a lane change. Other selected lanes and
            // linked audio move in time without collapsing onto the anchor's lane.
            let source_track = ids.first().and_then(|id| {
                p.clips
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| c.track_id.clone())
            });
            if let Some(ref target_id) = track_id {
                let source = source_track
                    .as_ref()
                    .ok_or("Select a clip to change tracks")?;
                let source_kind = &p
                    .tracks
                    .iter()
                    .find(|t| &t.id == source)
                    .ok_or("Source track does not exist")?
                    .kind;
                let target = p
                    .tracks
                    .iter()
                    .find(|t| &t.id == target_id)
                    .ok_or("Target track does not exist")?;
                require(
                    target.kind == *source_kind,
                    "Move clips to a track of the same kind",
                )?;
            }
            let ids = expanded(p, &ids);
            ensure_unlocked(p, &ids)?;
            if track_id
                .as_ref()
                .map(|tid| p.tracks.iter().any(|t| &t.id == tid && t.locked))
                .unwrap_or(false)
            {
                return Err("Track is locked".into());
            }
            let selected: HashSet<_> = ids.iter().map(String::as_str).collect();
            let mut group_counts = std::collections::HashMap::new();
            for c in &p.clips {
                let counts = group_counts
                    .entry(c.composition_group().to_owned())
                    .or_insert((0, 0));
                counts.0 += 1;
                counts.1 += usize::from(selected.contains(c.id.as_str()));
            }
            let complete_groups: HashSet<_> = group_counts
                .into_iter()
                .filter(|(_, (total, chosen))| total == chosen)
                .map(|(group, _)| group)
                .collect();
            for c in p
                .clips
                .iter_mut()
                .filter(|c| selected.contains(c.id.as_str()))
            {
                if (delta != 0
                    || (source_track.as_ref() == Some(&c.track_id)
                        && track_id.as_ref().map(|t| t != &c.track_id).unwrap_or(false)))
                    && c.composition
                        .as_ref()
                        .map(|origin| !complete_groups.contains(&origin.group_id))
                        .unwrap_or(false)
                {
                    // A fragment moved independently becomes an independent layer.
                    c.composition = None;
                    if let Some(r) = &mut c.retime {
                        r.composition_source_offset = None;
                    }
                }
                c.start += delta;
                if source_track.as_ref() == Some(&c.track_id) {
                    if let Some(ref t) = track_id {
                        c.track_id = t.clone();
                    }
                }
            }
        }
        EditCommand::Split { ids, frame } => {
            require((0..=10_000_000).contains(&frame), "Invalid split frame")?;
            let ids = expanded(p, &ids);
            ensure_unlocked(p, &ids)?;
            let mut added = vec![];
            let mut split_links = std::collections::HashMap::new();
            for c in p
                .clips
                .iter_mut()
                .filter(|c| ids.contains(&c.id) && frame > c.start && frame < c.end())
            {
                let offset = frame - c.start;
                inherit_envelopes(c, 0);
                let mut right = c.clone();
                inherit_envelopes(&mut right, offset);
                right.id = id();
                right.start = frame;
                right.duration = c.duration - offset;
                right.source_in = c
                    .source_in
                    .checked_add(Rational::from_frames(offset, p.fps).checked_mul(c.speed)?)?;
                let right_duration = right.duration;
                if let Some(r) = &c.retime {
                    let consumed = source_frames(offset, p.fps, c.speed)?;
                    let remaining = r.source_span.checked_sub(consumed)?;
                    retime_interval(&mut right, consumed, remaining, p.fps)?;
                } else {
                    rebase_keys(&mut right, offset, right_duration);
                }
                if let Some(ref link) = c.linked_id {
                    right.linked_id =
                        Some(split_links.entry(link.clone()).or_insert_with(id).clone());
                }
                c.duration = offset;
                if c.retime.is_some() {
                    retime_interval(
                        c,
                        Rational::zero(),
                        source_frames(offset, p.fps, c.speed)?,
                        p.fps,
                    )?;
                } else {
                    rebase_keys(c, 0, offset);
                }
                added.push(right);
            }
            p.clips.extend(added);
        }
        EditCommand::Trim { id, edge, frame } => {
            require((0..=10_000_000).contains(&frame), "Invalid trim frame")?;
            let ids = expanded(p, &[id.clone()]);
            ensure_unlocked(p, &ids)?;
            let original = p
                .clips
                .iter()
                .find(|c| c.id == id)
                .ok_or("Clip not found")?
                .clone();
            require(edge == "in" || edge == "out", "Trim edge must be in or out")?;
            let delta = frame
                - if edge == "in" {
                    original.start
                } else {
                    original.end()
                };
            for c in p.clips.iter_mut().filter(|c| ids.contains(&c.id)) {
                if edge == "in" {
                    let d = c.duration - delta;
                    require(d > 0, "Trim would remove the whole clip")?;
                    inherit_envelopes(c, delta);
                    c.source_in = c
                        .source_in
                        .checked_add(Rational::from_frames(delta, p.fps).checked_mul(c.speed)?)?;
                    c.start += delta;
                    c.duration = d;
                    if let Some(r) = &c.retime {
                        let consumed = source_frames(delta, p.fps, c.speed)?;
                        let remaining = r.source_span.checked_sub(consumed)?;
                        retime_interval(c, consumed, remaining, p.fps)?;
                    } else {
                        rebase_keys(c, delta, d);
                    }
                } else {
                    let d = c.duration + delta;
                    require(d > 0, "Trim would remove the whole clip")?;
                    inherit_envelopes(c, 0);
                    c.duration = d;
                    if let Some(r) = &c.retime {
                        let span = r
                            .source_span
                            .checked_add(source_frames(delta, p.fps, c.speed)?)?;
                        retime_interval(c, Rational::zero(), span, p.fps)?;
                    } else {
                        rebase_keys(c, 0, d);
                    }
                }
            }
        }
        EditCommand::Delete { ids, ripple } => {
            let ids = expanded(p, &ids);
            ensure_unlocked(p, &ids)?;
            let mut ranges: Vec<_> = p
                .clips
                .iter()
                .filter(|c| ids.contains(&c.id))
                .map(|c| (c.start, c.end()))
                .collect();
            ranges.sort();
            let mut merged: Vec<(i64, i64)> = vec![];
            for r in ranges {
                if let Some(last) = merged.last_mut() {
                    if r.0 <= last.1 {
                        last.1 = last.1.max(r.1);
                        continue;
                    }
                }
                merged.push(r);
            }
            p.clips.retain(|c| !ids.contains(&c.id));
            if ripple {
                for c in &mut p.clips {
                    if p.tracks.iter().any(|t| t.id == c.track_id && t.locked) {
                        continue;
                    }
                    let shift: i64 = merged
                        .iter()
                        .filter(|r| r.1 <= c.start)
                        .map(|r| r.1 - r.0)
                        .sum();
                    c.start -= shift;
                }
                for m in &mut p.markers {
                    let shift: i64 = merged
                        .iter()
                        .filter(|r| r.1 <= m.frame)
                        .map(|r| r.1 - r.0)
                        .sum();
                    m.frame -= shift;
                }
            }
            p.in_point = None;
            p.out_point = None;
        }
        EditCommand::Duplicate { ids } => {
            let ids = expanded(p, &ids);
            ensure_unlocked(p, &ids)?;
            let selected: Vec<_> = p
                .clips
                .iter()
                .filter(|c| ids.contains(&c.id))
                .cloned()
                .collect();
            let shift = selected.iter().map(Clip::end).max().unwrap_or(0)
                - selected.iter().map(|c| c.start).min().unwrap_or(0);
            let mut links = std::collections::HashMap::new();
            let mut origins = std::collections::HashMap::new();
            for c in &selected {
                if let Some(origin) = &c.composition {
                    let exact_offset = c
                        .retime
                        .as_ref()
                        .and_then(|r| r.composition_source_offset)
                        .map(|s| s.checked_mul(Rational::new(c.speed.den, c.speed.num)))
                        .transpose()?
                        .unwrap_or(Rational::from_frames(origin.offset, p.fps));
                    let entry = origins
                        .entry(origin.group_id.clone())
                        .or_insert_with(|| (id(), origin.offset, exact_offset));
                    entry.1 = entry.1.min(origin.offset);
                    if compare(exact_offset, entry.2).is_lt() {
                        entry.2 = exact_offset;
                    }
                }
            }
            for mut c in selected {
                c.id = id();
                c.start += shift;
                if let Some(origin) = &mut c.composition {
                    let (group_id, first_offset, first_exact) = &origins[&origin.group_id];
                    origin.group_id = group_id.clone();
                    origin.offset -= first_offset;
                    if let Some(r) = &mut c.retime {
                        if let Some(offset) = &mut r.composition_source_offset {
                            *offset = offset.checked_sub(first_exact.checked_mul(c.speed)?)?;
                        }
                    }
                }
                if let Some(ref link) = c.linked_id {
                    c.linked_id = Some(links.entry(link.clone()).or_insert_with(id).clone());
                }
                project_retime(&mut c, p.fps)?;
                p.clips.push(c);
            }
        }
        EditCommand::Slip { id, delta } => {
            require(
                delta.unsigned_abs() <= 10_000_000,
                "Slip exceeds sequence limits",
            )?;
            let ids = expanded(p, &[id]);
            ensure_unlocked(p, &ids)?;
            for c in p.clips.iter_mut().filter(|c| ids.contains(&c.id)) {
                c.render_offset = None;
                c.source_in = c
                    .source_in
                    .checked_add(Rational::from_frames(delta, p.fps).checked_mul(c.speed)?)?;
                if let Some(r) = &mut c.retime {
                    r.render_source_origin = c.source_in;
                }
            }
        }
        EditCommand::Link { ids } => {
            ensure_unlocked(p, &ids)?;
            require(ids.len() >= 2, "Select at least two clips to link")?;
            let link = id();
            for c in p.clips.iter_mut().filter(|c| ids.contains(&c.id)) {
                c.linked_id = Some(link.clone());
            }
        }
        EditCommand::Unlink { ids } => {
            let ids = expanded(p, &ids);
            ensure_unlocked(p, &ids)?;
            let mut extracted = vec![];
            let audio_track = p
                .tracks
                .iter()
                .find(|t| t.kind == "audio" && !t.locked)
                .map(|t| t.id.clone());
            for c in p.clips.iter_mut().filter(|c| ids.contains(&c.id)) {
                if c.linked_id.is_none()
                    && c.volume > 0.
                    && p.tracks
                        .iter()
                        .any(|t| t.id == c.track_id && t.kind == "video")
                    && c.media_id
                        .as_ref()
                        .map(|mid| p.media.iter().any(|m| &m.id == mid && m.has_audio))
                        .unwrap_or(false)
                {
                    let mut audio = c.clone();
                    audio.id = id();
                    audio.composition = None;
                    if let Some(r) = &mut audio.retime {
                        r.composition_source_offset = None;
                        r.envelope.keyframes.retain(|k| k.property == "volume");
                    }
                    audio.track_id = audio_track
                        .clone()
                        .ok_or("Add an unlocked audio track before extracting audio")?;
                    audio.name = format!("{} audio", c.name);
                    audio.opacity = 1.;
                    audio.transform = Transform::default();
                    audio.keyframes.retain(|k| k.property == "volume");
                    c.volume = 0.;
                    c.keyframes.retain(|k| k.property != "volume");
                    if let Some(r) = &mut c.retime {
                        r.envelope.keyframes.retain(|k| k.property != "volume");
                    }
                    extracted.push(audio);
                }
                c.linked_id = None;
            }
            p.clips.extend(extracted);
        }
        EditCommand::UpdateClip { id, patch } => {
            ensure_unlocked(p, &[id.clone()])?;
            let c = p
                .clips
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Clip not found")?;
            patch_clip(c, patch, p.fps)?;
        }
        EditCommand::RetimeClip { .. } => unreachable!(),
        EditCommand::CrossDissolve { id, frames } => {
            require(
                frames > 0 && frames <= 10_000_000,
                "Dissolve needs a positive number of frames",
            )?;
            let incoming = p
                .clips
                .iter()
                .find(|c| c.id == id)
                .ok_or("Incoming clip not found")?
                .clone();
            require(
                p.tracks
                    .iter()
                    .any(|t| t.id == incoming.track_id && t.kind == "video"),
                "Dissolve needs a video clip",
            )?;
            let previous = p
                .clips
                .iter()
                .filter(|c| {
                    c.track_id == incoming.track_id && c.id != id && c.start < incoming.start
                })
                .max_by_key(|c| c.start)
                .ok_or("Place an earlier clip on this video track before applying a dissolve")?
                .clone();
            require(
                frames <= previous.duration
                    && frames <= incoming.duration
                    && previous.end() - frames >= 0,
                "Dissolve exceeds clip duration",
            )?;
            let previous_end = previous.end();
            let incoming_ids = expanded(p, &[incoming.id]);
            let outgoing_ids = expanded(p, &[previous.id]);
            ensure_unlocked(p, &incoming_ids)?;
            ensure_unlocked(p, &outgoing_ids)?;
            let shift = previous_end - frames - incoming.start;
            for c in &mut p.clips {
                if incoming_ids.contains(&c.id) {
                    c.start += shift;
                    c.composition = None;
                    c.fade_in = frames.min(c.duration);
                    c.fade_in_start = None;
                    if let Some(r) = &mut c.retime {
                        r.composition_source_offset = None;
                        r.envelope.fade_in = source_frames(c.fade_in, p.fps, c.speed)?;
                        r.envelope.fade_in_start = Rational::zero();
                    }
                }
                if outgoing_ids.contains(&c.id) {
                    c.fade_out = frames.min(c.duration);
                    c.fade_out_end = None;
                    if let Some(r) = &mut c.retime {
                        r.envelope.fade_out = source_frames(c.fade_out, p.fps, c.speed)?;
                        r.envelope.fade_out_end = source_frames(c.duration, p.fps, c.speed)?;
                    }
                }
                if incoming_ids.contains(&c.id) || outgoing_ids.contains(&c.id) {
                    project_retime(c, p.fps)?;
                }
            }
        }
        EditCommand::AddTrack { kind, name } => {
            require(kind == "video" || kind == "audio", "Invalid track kind")?;
            p.tracks.push(Track {
                id: id(),
                name,
                kind,
                muted: false,
                hidden: false,
                locked: false,
            });
        }
        EditCommand::UpdateTrack { id, patch } => {
            let t = p
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Track not found")?;
            let map = patch.as_object().ok_or("Track patch must be an object")?;
            let mut v = serde_json::to_value(&*t).map_err(|e| e.to_string())?;
            for (k, val) in map {
                require(
                    ["name", "muted", "hidden", "locked"].contains(&k.as_str()),
                    "Unknown track property",
                )?;
                v[k] = val.clone();
            }
            *t = serde_json::from_value(v).map_err(|e| e.to_string())?;
        }
        EditCommand::AddBin { name } => {
            require(
                !name.trim().is_empty() && name.len() <= 512,
                "Invalid bin name",
            )?;
            p.bins.push(Bin { id: id(), name });
        }
        EditCommand::AssignBin { media_id, bin_id } => {
            require(
                bin_id
                    .as_ref()
                    .map(|bid| p.bins.iter().any(|b| &b.id == bid))
                    .unwrap_or(true),
                "Bin not found",
            )?;
            p.media
                .iter_mut()
                .find(|m| m.id == media_id)
                .ok_or("Media not found")?
                .bin_id = bin_id;
        }
        EditCommand::Relink { .. } => {
            return Err("Relink must be processed by the project store".into())
        }
        EditCommand::Marker { frame, name } => p.markers.push(Marker {
            id: id(),
            frame,
            name,
        }),
        EditCommand::RemoveMarker { id } => p.markers.retain(|m| m.id != id),
        EditCommand::SetRange {
            in_point,
            out_point,
        } => {
            p.in_point = in_point;
            p.out_point = out_point;
        }
        EditCommand::Rename { name } => p.name = name,
    }
    if !explicit_range {
        if p.clips.is_empty() {
            p.in_point = None;
            p.out_point = None;
        } else {
            let end = p.length();
            p.out_point = p.out_point.map(|frame| frame.min(end));
            let range_end = p.out_point.unwrap_or(end);
            p.in_point = p.in_point.map(|frame| frame.min(range_end - 1));
        }
    }
    p.validate()
}
