//! Splitting is a structural edit: inherited fades and automation must sound
//! and look the same. Synthetic fixtures are decoded silently, never played.
use mono_cut_lib::{
    edit,
    jobs::JobManager,
    media::{self, MediaContext},
    model::*,
    render,
    storage::ProjectStore,
};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const FRAME_BYTES: usize = 160 * 90 * 3;
const SAMPLES_PER_FRAME: usize = 1600;

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

fn fixture(ctx: &MediaContext, path: &Path, video: &str, seconds: u32, sine: bool) {
    let video = if video == "pattern" {
        format!("testsrc2=size=160x90:rate=30:duration={seconds}")
    } else {
        format!("color=c={video}:size=160x90:rate=30:duration={seconds}")
    };
    let audio = if sine {
        format!("aevalsrc=0.4*sin(2*PI*997*t):s=48000:d={seconds}")
    } else {
        format!("aevalsrc=0.4:s=48000:d={seconds}")
    };
    media::run_ffmpeg(
        ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            video,
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            audio,
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
}

fn project(ctx: &MediaContext, source: &Path, frames: i64) -> Project {
    let mut p = Project::new("Split envelopes".into(), 160, 90, Rational::new(30, 1));
    p.media.push(media::probe(ctx, source).unwrap());
    let media_id = p.media[0].id.clone();
    let track_id = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id,
            track_id,
            start: 0,
            source_in: None,
            duration: Some(frames),
        },
    )
    .unwrap();
    p
}

fn patch(p: &mut Project, clip: &str, value: serde_json::Value) {
    edit::apply(
        p,
        EditCommand::UpdateClip {
            id: clip.into(),
            patch: value,
        },
    )
    .unwrap();
}

fn split(p: &mut Project, frames: &[i64]) {
    for frame in frames {
        let ids: Vec<_> = p
            .clips
            .iter()
            .filter(|c| c.start < *frame && c.end() > *frame)
            .map(|c| c.id.clone())
            .collect();
        assert!(!ids.is_empty(), "No clip spans test cut {frame}");
        edit::apply(p, EditCommand::Split { ids, frame: *frame }).unwrap();
    }
}

fn settings(codec: &str) -> ExportSettings {
    ExportSettings {
        width: 160,
        height: 90,
        fps: Rational::new(30, 1),
        codec: codec.into(),
        crf: 18,
        audio_bitrate: 192,
        sample_rate: 48000,
    }
}

fn export(ctx: &MediaContext, p: &Project, path: &Path, proxies: bool, codec: &str) {
    let mut output_settings = settings(codec);
    output_settings.fps = p.fps;
    let plan = render::compile(ctx, p, &output_settings, proxies, path, &id()).unwrap();
    media::run_ffmpeg(ctx, &plan.args).unwrap();
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

struct Av {
    pixels: Vec<u8>,
    pcm: Vec<f32>,
}
fn av(ctx: &MediaContext, path: &Path) -> Av {
    let pixels = decoded(
        ctx,
        path,
        &[
            "-map",
            "0:v:0",
            "-vf",
            "scale=160:90",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
        ],
    );
    assert_eq!(pixels.len() % FRAME_BYTES, 0);
    let audio = decoded(
        ctx,
        path,
        &["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le"],
    );
    let pcm = audio
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    Av { pixels, pcm }
}

fn render_av(ctx: &MediaContext, p: &Project, path: &Path, proxies: bool) -> Av {
    export(ctx, p, path, proxies, "ffv1");
    let result = av(ctx, path);
    assert_eq!(result.pixels.len() / FRAME_BYTES, p.length() as usize);
    assert_eq!(result.pcm.len(), sample_boundary(p.length(), p.fps));
    result
}

fn sample_boundary(frames: i64, fps: Rational) -> usize {
    let numerator = frames as i128 * fps.den as i128 * 48000;
    ((numerator + fps.num as i128 - 1) / fps.num as i128) as usize
}

fn fractional_fixture(ctx: &MediaContext, source: &Path) {
    media::run_ffmpeg(
        ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=160x90:rate=30000/1001:duration=5".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "aevalsrc=0.4*sin(2*PI*997*t):s=48000:d=5".into(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "1:a:0".into(),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "pcm_f32le".into(),
            source.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
}

fn fractional_project(
    ctx: &MediaContext,
    source: &Path,
    start: i64,
    source_in: i64,
    frames: i64,
) -> Project {
    let fps = Rational::new(30000, 1001);
    let mut p = Project::new("Fractional split".into(), 160, 90, fps);
    p.media.push(media::probe(ctx, source).unwrap());
    assert_eq!(p.media[0].fps, p.fps);
    let mid = p.media[0].id.clone();
    let track = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: mid,
            track_id: track,
            start,
            source_in: Some(Rational::from_frames(source_in, fps)),
            duration: Some(frames),
        },
    )
    .unwrap();
    let clip = p.clips[0].id.clone();
    patch(
        &mut p,
        &clip,
        json!({"fade_in":18,"fade_out":22,"keyframes":[
            {"property":"volume","frame":0,"value":0.3},
            {"property":"volume","frame":27,"value":0.9},
            {"property":"volume","frame":frames-1,"value":0.5}
        ]}),
    );
    p
}

fn compare(
    before: &Av,
    after: &Av,
    label: &str,
    pixel_limit: u8,
    pcm_limit: f32,
    pcm_rms_limit: f64,
) -> (u8, f32, f64) {
    assert_eq!(
        before.pixels.len(),
        after.pixels.len(),
        "{label}: decoded frame count changed"
    );
    assert_eq!(
        before.pcm.len(),
        after.pcm.len(),
        "{label}: decoded audio duration changed"
    );
    let max_pixel = before
        .pixels
        .iter()
        .zip(&after.pixels)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    if max_pixel > pixel_limit {
        let first = before
            .pixels
            .iter()
            .zip(&after.pixels)
            .position(|(a, b)| a.abs_diff(*b) > pixel_limit)
            .unwrap();
        let maximum = before
            .pixels
            .iter()
            .zip(&after.pixels)
            .enumerate()
            .max_by_key(|(_, (a, b))| a.abs_diff(**b))
            .unwrap()
            .0;
        for (kind, index) in [("first", first), ("maximum", maximum)] {
            let frame = index / FRAME_BYTES;
            let offset = index % FRAME_BYTES;
            let pixel = offset / 3;
            let channel = offset % 3;
            let a = &before.pixels[frame * FRAME_BYTES..(frame + 1) * FRAME_BYTES];
            let b = &after.pixels[frame * FRAME_BYTES..(frame + 1) * FRAME_BYTES];
            let mean = |pixels: &[u8]| -> Vec<f64> {
                (0..3)
                    .map(|channel| {
                        pixels
                            .iter()
                            .skip(channel)
                            .step_by(3)
                            .map(|v| *v as f64)
                            .sum::<f64>()
                            / (160. * 90.)
                    })
                    .collect()
            };
            let center = (45 * 160 + 80) * 3;
            eprintln!("DIAGNOSTIC {label} {kind} RGB mismatch:frame{frame},pixel({},{}),channel{channel},before{},after{}; centerbefore{:?},after{:?}; frame meansbefore{:?},after{:?}",pixel%160,pixel/160,a[offset],b[offset],&a[center..center+3],&b[center..center+3],mean(a),mean(b));
        }
    }
    let max_pcm = before
        .pcm
        .iter()
        .zip(&after.pcm)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    let rms = (before
        .pcm
        .iter()
        .zip(&after.pcm)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / before.pcm.len() as f64)
        .sqrt();
    if max_pcm > pcm_limit || rms > pcm_rms_limit {
        let first = before
            .pcm
            .iter()
            .zip(&after.pcm)
            .position(|(a, b)| (a - b).abs() > pcm_limit);
        let maximum = before
            .pcm
            .iter()
            .zip(&after.pcm)
            .enumerate()
            .max_by(|(_, (a, b)), (_, (c, d))| (*a - *b).abs().total_cmp(&(*c - *d).abs()))
            .unwrap()
            .0;
        for (kind, index) in [("first", first.unwrap_or(maximum)), ("maximum", maximum)] {
            eprintln!("DIAGNOSTIC {label} {kind} PCM mismatch: sample{index},time{:.9}s,sequenceframe{:.6},before{:.9},after{:.9}",index as f64/48000.,index as f64/1600.,before.pcm[index],after.pcm[index]);
        }
    }
    if max_pixel > pixel_limit {
        eprintln!(
            "DIAGNOSTIC {label} complete metrics:maxRGB{max_pixel},maxPCM{max_pcm:.9},RMS{rms:.9}"
        );
    }
    assert!(
        max_pixel <= pixel_limit,
        "{label}: pixel discontinuity, max RGB difference {max_pixel}, allowed {pixel_limit}"
    );
    assert!(
        max_pcm <= pcm_limit && rms <= pcm_rms_limit,
        "{label}: PCM discontinuity, max {max_pcm:.9}, RMS {rms:.9}"
    );
    (max_pixel, max_pcm, rms)
}

fn around_cuts(before: &Av, after: &Av, cuts: &[i64], label: &str, tolerance: f32) {
    for cut in cuts {
        let sample = *cut as usize * SAMPLES_PER_FRAME;
        let range = sample.saturating_sub(256)..(sample + 256).min(before.pcm.len());
        let maximum = range
            .map(|i| (before.pcm[i] - after.pcm[i]).abs())
            .fold(0f32, f32::max);
        assert!(
            maximum <= tolerance,
            "{label}: envelope changed immediately around cut frame {cut}; max PCM error {maximum}"
        );
    }
}

fn independent_fade(
    ctx: &MediaContext,
    source: &Path,
    path: &Path,
    fade_in: i64,
    fade_out: i64,
) -> Av {
    independent_segment_fade(ctx, source, path, 0, 60, fade_in, fade_out)
}

fn independent_segment_fade(
    ctx: &MediaContext,
    source: &Path,
    path: &Path,
    source_frame: i64,
    frames: i64,
    fade_in: i64,
    fade_out: i64,
) -> Av {
    let start = source_frame as f64 / 30.;
    let end = (source_frame + frames) as f64 / 30.;
    let mut video = vec![format!(
        "trim=start={start}:end={end},setpts=PTS-STARTPTS,format=yuv420p"
    )];
    let mut audio = vec![format!("atrim=start={start}:end={end},asetpts=PTS-STARTPTS,aformat=sample_fmts=fltp:channel_layouts=stereo")];
    if fade_in > 0 {
        video.push(format!("fade=t=in:st=0:d={}", fade_in as f64 / 30.));
        audio.push(format!("afade=t=in:st=0:d={}", fade_in as f64 / 30.));
    }
    if fade_out > 0 {
        video.push(format!(
            "fade=t=out:st={}:d={}",
            (frames - fade_out) as f64 / 30.,
            fade_out as f64 / 30.
        ));
        audio.push(format!(
            "afade=t=out:st={}:d={}",
            (frames - fade_out) as f64 / 30.,
            fade_out as f64 / 30.
        ));
    }
    audio.push("alimiter=limit=0.97:latency=1".into());
    // A standalone FFmpeg fade fixture, independent of the project compiler.
    // Color fading to black is equivalent to alpha fading over a black canvas.
    media::run_ffmpeg(
        ctx,
        &vec![
            "-i".into(),
            source.to_string_lossy().into_owned(),
            "-vf".into(),
            video.join(","),
            "-af".into(),
            audio.join(","),
            "-c:v".into(),
            "ffv1".into(),
            "-c:a".into(),
            "flac".into(),
            "-ar".into(),
            "48000".into(),
            "-frames:v".into(),
            frames.to_string(),
            "-t".into(),
            (frames as f64 / 30.).to_string(),
            path.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    av(ctx, path)
}

#[test]
fn fade_envelopes_match_independent_reference_across_inside_boundary_and_repeated_splits() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("red-tone.mkv");
    fixture(&ctx, &source, "red", 2, false);
    let cases: &[(&str, i64, i64, &[i64])] = &[
        ("fade-in-inside", 30, 0, &[15]),
        ("fade-out-inside", 0, 30, &[45]),
        ("fade-boundaries-outside", 18, 22, &[9, 18, 28, 38, 49]),
        ("both-repeated", 30, 30, &[15, 30, 45]),
        ("neutral-control", 0, 0, &[15, 30, 45]),
    ];
    for (name, fade_in, fade_out, cuts) in cases {
        let mut before_project = project(&ctx, &source, 60);
        let clip = before_project.clips[0].id.clone();
        patch(
            &mut before_project,
            &clip,
            json!({"fade_in":fade_in,"fade_out":fade_out}),
        );
        let before = render_av(
            &ctx,
            &before_project,
            &dir.path().join(format!("{name}-before.mkv")),
            false,
        );
        let reference = independent_fade(
            &ctx,
            &source,
            &dir.path().join(format!("{name}-reference.mkv")),
            *fade_in,
            *fade_out,
        );
        compare(
            &reference,
            &before,
            &format!("{name} independent fade reference"),
            5,
            0.000_004,
            0.000_002,
        );
        let mut after_project = before_project.clone();
        split(&mut after_project, cuts);
        let after = render_av(
            &ctx,
            &after_project,
            &dir.path().join(format!("{name}-after.mkv")),
            false,
        );
        let stats = compare(&before, &after, name, 1, 0.000_004, 0.000_002);
        around_cuts(&before, &after, cuts, name, 0.000_004);
        eprintln!("VERIFIED fade envelope {name}: cuts {cuts:?},60 frames/96000 samples; maxRGB {},maxPCM {:.9},RMS {:.9}; standalone known-duration fade reference agrees.",stats.0,stats.1,stats.2);
    }
}

fn automated_linked_project(ctx: &MediaContext, source: &Path) -> Project {
    let mut p = project(ctx, source, 60);
    let clip = p.clips[0].id.clone();
    patch(
        &mut p,
        &clip,
        json!({"fade_in":18,"fade_out":22,"brightness":0.03,"contrast":1.1,"saturation":0.8,"transform":{"scale":0.8,"rotation":8.0,"crop_left":0.08,"crop_right":0.04,"crop_top":0.05},"keyframes":[{"property":"opacity","frame":0,"value":0.25},{"property":"opacity","frame":27,"value":0.85},{"property":"opacity","frame":59,"value":0.4},{"property":"volume","frame":0,"value":0.3},{"property":"volume","frame":27,"value":0.9},{"property":"volume","frame":59,"value":0.5},{"property":"scale","frame":0,"value":0.65},{"property":"scale","frame":27,"value":0.95},{"property":"scale","frame":59,"value":0.75},{"property":"x","frame":0,"value":-10.0},{"property":"x","frame":59,"value":12.0},{"property":"y","frame":0,"value":8.0},{"property":"y","frame":59,"value":-6.0}]}),
    );
    edit::apply(&mut p, EditCommand::Unlink { ids: vec![clip] }).unwrap();
    let ids = p.clips.iter().map(|c| c.id.clone()).collect();
    edit::apply(&mut p, EditCommand::Link { ids }).unwrap();
    assert_eq!(p.clips.len(), 2);
    p
}

fn assert_link_pairs(p: &Project) {
    let mut groups: HashMap<String, Vec<&Clip>> = HashMap::new();
    for c in &p.clips {
        groups
            .entry(
                c.linked_id
                    .clone()
                    .expect("Split linked clips must stay linked"),
            )
            .or_default()
            .push(c);
    }
    for group in groups.values() {
        assert_eq!(group.len(), 2);
        assert_eq!(
            (group[0].start, group[0].duration),
            (group[1].start, group[1].duration)
        );
        assert_ne!(group[0].track_id, group[1].track_id);
    }
}

#[test]
fn linked_fades_crop_color_and_continuous_automation_preserve_pixels_and_every_audio_sample() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("pattern-sine.mkv");
    fixture(&ctx, &source, "pattern", 2, true);
    let before_project = automated_linked_project(&ctx, &source);
    let before = render_av(
        &ctx,
        &before_project,
        &dir.path().join("automation-before.mkv"),
        false,
    );
    let mut after_project = before_project.clone();
    for frame in [13, 31, 47] {
        let video = after_project
            .clips
            .iter()
            .find(|c| {
                c.start < frame
                    && c.end() > frame
                    && after_project
                        .tracks
                        .iter()
                        .any(|t| t.id == c.track_id && t.kind == "video")
            })
            .unwrap()
            .id
            .clone();
        edit::apply(
            &mut after_project,
            EditCommand::Split {
                ids: vec![video],
                frame,
            },
        )
        .unwrap();
        assert_link_pairs(&after_project);
    }
    assert_eq!(after_project.clips.len(), 8);
    let after = render_av(
        &ctx,
        &after_project,
        &dir.path().join("automation-after.mkv"),
        false,
    );
    let stats = compare(
        &before,
        &after,
        "linked fades/continuous automation",
        1,
        0.000_004,
        0.000_002,
    );
    around_cuts(
        &before,
        &after,
        &[13, 31, 47],
        "linked automation",
        0.000_004,
    );
    eprintln!("VERIFIED linkedV/A repeatedsplits13/31/47:8clips/fourlinks; inheritedfades,opacity/volume/scale/x/ykeyframes,crop/color/rotation preserved; maxRGB{},maxPCM{:.9},RMS{:.9}.",stats.0,stats.1,stats.2);
}

#[test]
fn scale_rotation_and_position_automation_use_a_fragment_independent_canvas() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("canvas-pattern.mkv");
    fixture(&ctx, &source, "pattern", 2, true);
    let mut failures = vec![];
    for (name, rotation, dynamic_scale, dynamic_position, dynamic_opacity) in [
        ("opacity-only", 0., false, false, true),
        ("rotation-only", 8., false, false, false),
        ("scale-only", 0., true, false, false),
        ("scale-and-rotation", 8., true, false, false),
        ("position-only", 0., false, true, false),
        ("all-automation", 8., true, true, true),
    ] {
        let mut before_project = automated_linked_project(&ctx, &source);
        for c in &mut before_project.clips {
            if before_project
                .tracks
                .iter()
                .any(|t| t.id == c.track_id && t.kind == "video")
            {
                c.transform.rotation = rotation;
                if !dynamic_opacity {
                    c.opacity = 1.;
                }
                c.keyframes.retain(|k| {
                    (dynamic_scale || k.property != "scale")
                        && (dynamic_position || !["x", "y"].contains(&k.property.as_str()))
                        && (dynamic_opacity || k.property != "opacity")
                });
            }
        }
        before_project.validate().unwrap();
        let before = render_av(
            &ctx,
            &before_project,
            &dir.path().join(format!("canvas-{name}-before.mkv")),
            false,
        );
        let mut after_project = before_project.clone();
        split(&mut after_project, &[13]);
        let after = render_av(
            &ctx,
            &after_project,
            &dir.path().join(format!("canvas-{name}-after.mkv")),
            false,
        );
        let max_pixel = before
            .pixels
            .iter()
            .zip(&after.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let max_pcm = before
            .pcm
            .iter()
            .zip(&after.pcm)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        let first = before
            .pixels
            .iter()
            .zip(&after.pixels)
            .position(|(a, b)| a.abs_diff(*b) > 1)
            .map(|i| i / FRAME_BYTES);
        eprintln!("VERIFIED canvas group{name}:rotation{rotation},dynamic_scale{dynamic_scale},dynamic_position{dynamic_position}; maxRGB{max_pixel},firstbadframe{first:?},maxPCM{max_pcm:.9}");
        if max_pixel > 1 || max_pcm > 0.000_004 {
            failures.push((name, max_pixel, max_pcm));
        }
    }
    assert!(
        failures.is_empty(),
        "Split changes a transform canvas or automation: {failures:?}"
    );
}

#[test]
fn fractional_sequence_split_boundaries_preserve_every_sample_and_inherited_gain_phase() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("fractional-pattern-sine.mkv");
    fractional_fixture(&ctx, &source);
    for (start, source_in, speed) in [(0, 0, 1), (7, 5, 1), (7, 5, 2)] {
        let mut p = fractional_project(&ctx, &source, start, source_in, 60);
        let clip = p.clips[0].id.clone();
        // Preserve the legacy fixed-duration renderer fixture at this rate.
        p.clips[0].speed=Rational::new(speed,1); p.clips[0].render_offset=None;
        p.validate().unwrap();
        let before = render_av(
            &ctx,
            &p,
            &dir.path().join(format!("fractional-{start}-before.mkv")),
            false,
        );
        assert_eq!(before.pcm.len(), sample_boundary(start + 60, p.fps));
        assert!(before.pcm[..sample_boundary(start, p.fps)]
            .iter()
            .all(|v| v.abs() < 0.000_001));
        let mut after_project = p.clone();
        let cuts = [1, 2, 7, 13, 17, 31, 47].map(|f| start + f);
        split(&mut after_project, &cuts);
        let after = render_av(
            &ctx,
            &after_project,
            &dir.path().join(format!("fractional-{start}-after.mkv")),
            false,
        );
        let audio_before = Av {
            pixels: vec![],
            pcm: before.pcm.clone(),
        };
        let audio_after = Av {
            pixels: vec![],
            pcm: after.pcm.clone(),
        };
        let audio_stats = compare(
            &audio_before,
            &audio_after,
            &format!("Fractional2997samplegrid+gainphase start={start},source_in={source_in},speed={speed}"),
            0,
            0.000_001,
            0.000_000_4,
        );
        for cut in cuts {
            let sample = sample_boundary(cut, p.fps);
            let range = sample.saturating_sub(256)..(sample + 256).min(before.pcm.len());
            let maximum = range
                .map(|i| (before.pcm[i] - after.pcm[i]).abs())
                .fold(0f32, f32::max);
            assert!(
                maximum <= 0.000_001,
                "Fractional cut{cut}/sample{sample} changed localPCMphase by{maximum}"
            );
        }
        let full_stats = compare(
            &before,
            &after,
            &format!(
                "Fractional2997decodedframe+samplecontinuity start={start},source_in={source_in},speed={speed}"
            ),
            1,
            0.000_001,
            0.000_000_4,
        );
        eprintln!("VERIFIED 30000/1001 sequence start={start}, source_in={source_in}, speed={speed}, cuts={cuts:?}: {} frames/{} samples; maxRGB {}, maxPCM {:.9}, RMS {:.9}; absolute rational boundaries and inherited gain clock agree.",p.length(),before.pcm.len(),full_stats.0,audio_stats.1,audio_stats.2);
        if start == 0 {
            let mut ranged = after_project;
            ranged.in_point = Some(2);
            ranged.out_point = Some(7);
            let path = dir.path().join("fractional-range.mkv");
            export(&ctx, &ranged, &path, false, "ffv1");
            let actual = av(&ctx, &path);
            assert_eq!(
                actual.pcm.len(),
                8008,
                "Range must subtract absolute ceil sample boundaries 11212-3204"
            );
            let expected = Av {
                pixels: before.pixels[2 * FRAME_BYTES..7 * FRAME_BYTES].to_vec(),
                pcm: before.pcm[sample_boundary(2, p.fps)..sample_boundary(7, p.fps)].to_vec(),
            };
            let stats = compare(
                &expected,
                &actual,
                "Fractional export range frames 2..7",
                1,
                0.000_001,
                0.000_000_4,
            );
            eprintln!("VERIFIED fractional export range 2..7: 5 frames/8008 samples, maxRGB {}, maxPCM {:.9}.",stats.0,stats.1);
        }
    }
}

#[test]
fn splitting_dissolve_participants_preserves_compositing_order_and_opaque_midpoint() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let mut p = Project::new("Dissolve split".into(), 160, 90, Rational::new(30, 1));
    for (name, color, duration) in [
        ("underlay", "0x777777", 3),
        ("outgoing", "red", 2),
        ("incoming", "blue", 2),
    ] {
        let path = dir.path().join(format!("{name}.mkv"));
        fixture(&ctx, &path, color, duration, false);
        p.media.push(media::probe(&ctx, &path).unwrap());
    }
    let lower_media = p.media[0].id.clone();
    let lower_track = p.tracks[0].id.clone();
    edit::apply(
        &mut p,
        EditCommand::AddClip {
            media_id: lower_media,
            track_id: lower_track,
            start: 0,
            source_in: None,
            duration: Some(90),
        },
    )
    .unwrap();
    let lower = p.clips[0].id.clone();
    patch(&mut p, &lower, json!({"volume":0.0}));
    edit::apply(
        &mut p,
        EditCommand::AddTrack {
            kind: "video".into(),
            name: "Picture".into(),
        },
    )
    .unwrap();
    let top = p.tracks.last().unwrap().id.clone();
    for (index, start) in [(1, 0), (2, 60)] {
        let mid = p.media[index].id.clone();
        edit::apply(
            &mut p,
            EditCommand::AddClip {
                media_id: mid,
                track_id: top.clone(),
                start,
                source_in: None,
                duration: Some(60),
            },
        )
        .unwrap();
    }
    let incoming = p.clips[2].id.clone();
    edit::apply(
        &mut p,
        EditCommand::CrossDissolve {
            id: incoming,
            frames: 30,
        },
    )
    .unwrap();
    assert_eq!(p.clips[2].start, 30);
    let before = render_av(&ctx, &p, &dir.path().join("dissolve-before.mkv"), false);
    let center = (45 * 160 + 80) * 3;
    let midpoint = &before.pixels[45 * FRAME_BYTES + center..45 * FRAME_BYTES + center + 3];
    assert!((midpoint[0] as i16-127).abs()<=8 && midpoint[1]<8 && (midpoint[2] as i16-127).abs()<=8,"Opaque dissolve midpoint must be halfred/halfblue without revealing gray underlay: {midpoint:?}");
    for (name, cuts) in [
        ("outgoing-inside", vec![(1usize, 45)]),
        ("incoming-inside", vec![(2usize, 45)]),
        ("both-repeated", vec![(1usize, 15), (1, 45), (2, 51)]),
    ] {
        let mut after_project = p.clone();
        for (media_index, frame) in &cuts {
            let mid = &p.media[*media_index].id;
            let clip = after_project
                .clips
                .iter()
                .find(|c| c.media_id.as_ref() == Some(mid) && c.start < *frame && c.end() > *frame)
                .unwrap()
                .id
                .clone();
            edit::apply(
                &mut after_project,
                EditCommand::Split {
                    ids: vec![clip],
                    frame: *frame,
                },
            )
            .unwrap();
        }
        let after = render_av(
            &ctx,
            &after_project,
            &dir.path().join(format!("{name}.mkv")),
            false,
        );
        let stats = compare(&before, &after, name, 1, 0.000_004, 0.000_002);
        let frames: Vec<_> = cuts.iter().map(|(_, f)| *f).collect();
        around_cuts(&before, &after, &frames, name, 0.000_004);
        eprintln!("VERIFIED splitdissolve {name}:90frames/144000samples; opaque midpoint{midpoint:?}; lowertrack andparticipantorder retained; maxRGB{},maxPCM{:.9}.",stats.0,stats.1);
    }
    // Retaining only the incoming tail beyond its dissolve window must allow
    // the outgoing clip's own fade again. Its old anchors alone are insufficient.
    let mut tail_only = p.clone();
    let incoming_id = tail_only.clips[2].id.clone();
    edit::apply(
        &mut tail_only,
        EditCommand::Split {
            ids: vec![incoming_id.clone()],
            frame: 70,
        },
    )
    .unwrap();
    edit::apply(
        &mut tail_only,
        EditCommand::Delete {
            ids: vec![incoming_id],
            ripple: false,
        },
    )
    .unwrap();
    let retained = tail_only
        .clips
        .iter()
        .find(|c| c.media_id.as_ref() == Some(&p.media[2].id))
        .unwrap();
    assert_eq!(
        (retained.start, retained.duration, retained.fade_in_start()),
        (70, 20, -40)
    );
    let actual = render_av(
        &ctx,
        &tail_only,
        &dir.path().join("dissolve-tail-only.mkv"),
        false,
    );
    let mut reference = p.clone();
    reference
        .clips
        .retain(|c| c.media_id.as_ref() != Some(&p.media[2].id));
    let incoming_media = p.media[2].id.clone();
    let incoming_track = p.clips[2].track_id.clone();
    edit::apply(
        &mut reference,
        EditCommand::AddClip {
            media_id: incoming_media,
            track_id: incoming_track,
            start: 70,
            source_in: Some(Rational::from_frames(40, p.fps)),
            duration: Some(20),
        },
    )
    .unwrap();
    let expected = render_av(
        &ctx,
        &reference,
        &dir.path().join("dissolve-tail-reference.mkv"),
        false,
    );
    let fade_midpoint = &actual.pixels[45 * FRAME_BYTES + center..45 * FRAME_BYTES + center + 3];
    assert!(fade_midpoint[0]>170 && fade_midpoint[1]>40 && fade_midpoint[2]>40,
        "Outgoing fade must reveal the gray underlay when only a non-overlapping blue tail remains: {fade_midpoint:?}");
    let stats = compare(
        &expected,
        &actual,
        "Non-overlapping dissolve tail cannot suppress outgoing fade",
        1,
        0.000_004,
        0.000_002,
    );
    eprintln!("VERIFIED retained incoming tail70..90 after deleting30..70: ordinary outgoing fade restored, gray/red midpoint {fade_midpoint:?}; maxRGB {}, maxPCM {:.9}.",stats.0,stats.1);
}

#[test]
fn negative_render_offset_trim_extension_retains_fractional_source_and_envelope_clocks() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("fractional-extension.mkv");
    fractional_fixture(&ctx, &source);
    let original = fractional_project(&ctx, &source, 10, 10, 50);
    let before = render_av(
        &ctx,
        &original,
        &dir.path().join("extension-before.mkv"),
        false,
    );
    let mut extended = original.clone();
    let clip = extended.clips[0].id.clone();
    edit::apply(
        &mut extended,
        EditCommand::Trim {
            id: clip,
            edge: "in".into(),
            frame: 7,
        },
    )
    .unwrap();
    let c = &extended.clips[0];
    assert_eq!(
        (c.start, c.duration, c.render_offset, c.fade_in_start()),
        (7, 53, Some(-3), 3)
    );
    assert_eq!(c.source_in, Rational::from_frames(7, extended.fps));
    assert_eq!(c.composition.as_ref().unwrap().offset, -3);
    let actual = render_av(
        &ctx,
        &extended,
        &dir.path().join("extension-after.mkv"),
        false,
    );
    let stats = compare(
        &before,
        &actual,
        "Negative inherited clock extends before original fade",
        1,
        0.000_001,
        0.000_000_4,
    );
    split(&mut extended, &[8, 12, 23, 39, 55]);
    assert!(extended.clips.iter().any(|c| c.render_offset == Some(-3)));
    let after_split = render_av(
        &ctx,
        &extended,
        &dir.path().join("extension-split.mkv"),
        false,
    );
    compare(
        &before,
        &after_split,
        "Repeated splits of signed inherited clock",
        1,
        0.000_001,
        0.000_000_4,
    );
    let path = dir.path().join("signed-clock.monocut");
    fs::write(&path, serde_json::to_vec_pretty(&extended).unwrap()).unwrap();
    let recovery = dir.path().join("signed-clock-recovery.json");
    let mut store = ProjectStore::new(recovery.clone());
    let opened = store.open_hydrated(&path, &ctx).unwrap();
    assert_eq!(opened.clips, extended.clips);
    store.save(&path).unwrap();
    let recovered = ProjectStore::new(recovery).recover_hydrated(&ctx).unwrap();
    assert_eq!(recovered.clips, extended.clips);
    let reopened = ProjectStore::new(dir.path().join("signed-clock-reopened.json"))
        .open_hydrated(&path, &ctx)
        .unwrap();
    let reopened_av = render_av(
        &ctx,
        &reopened,
        &dir.path().join("extension-reopened.mkv"),
        false,
    );
    compare(
        &before,
        &reopened_av,
        "Signed clock survives save/reopen/recovery",
        1,
        0.000_001,
        0.000_000_4,
    );
    eprintln!("VERIFIED negative render offset -3 at 30000/1001 with timeline/source origins 10: retained 60 frames/96096 samples and inherited fades after extension, repeated splits, save/reopen/recovery; maxRGB {}, maxPCM {:.9}.",stats.0,stats.1);
}

#[test]
fn composition_lifecycle_moves_and_duplicates_preserve_real_fragment_envelopes() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("lifecycle-red.mkv");
    fixture(&ctx, &source, "red", 2, true);
    let mut original = project(&ctx, &source, 60);
    let clip = original.clips[0].id.clone();
    patch(
        &mut original,
        &clip,
        json!({"fade_in":30,"fade_out":30,"keyframes":[
            {"property":"volume","frame":0,"value":0.3},
            {"property":"volume","frame":59,"value":0.8}
        ]}),
    );
    let baseline = render_av(
        &ctx,
        &original,
        &dir.path().join("lifecycle-before.mkv"),
        false,
    );
    let mut fragments = original;
    split(&mut fragments, &[15, 45]);
    let all_ids: Vec<_> = fragments.clips.iter().map(|c| c.id.clone()).collect();
    let group = fragments.clips[0].composition_group().to_owned();
    assert!(fragments
        .clips
        .iter()
        .all(|c| c.composition_group() == group));
    let mut moved = fragments.clone();
    edit::apply(
        &mut moved,
        EditCommand::Move {
            ids: all_ids.clone(),
            delta: 30,
            track_id: None,
        },
    )
    .unwrap();
    for (before, after) in fragments.clips.iter().zip(&moved.clips) {
        assert_eq!(
            before.composition, after.composition,
            "Moving a complete original layer retains its order and origin"
        );
        assert_eq!(before.render_offset, after.render_offset);
    }
    let actual = render_av(
        &ctx,
        &moved,
        &dir.path().join("lifecycle-whole-move.mkv"),
        false,
    );
    let mut expected = Av {
        pixels: vec![0; 90 * FRAME_BYTES],
        pcm: vec![0.; 90 * SAMPLES_PER_FRAME],
    };
    expected.pixels[30 * FRAME_BYTES..].copy_from_slice(&baseline.pixels);
    expected.pcm[30 * SAMPLES_PER_FRAME..].copy_from_slice(&baseline.pcm);
    compare(
        &expected,
        &actual,
        "Whole composition move retains inherited envelopes",
        1,
        0.000_004,
        0.000_002,
    );

    let middle = fragments
        .clips
        .iter()
        .find(|c| c.start == 15)
        .unwrap()
        .clone();
    let mut subset = fragments.clone();
    edit::apply(
        &mut subset,
        EditCommand::Move {
            ids: vec![middle.id.clone()],
            delta: 60,
            track_id: None,
        },
    )
    .unwrap();
    assert!(subset
        .clips
        .iter()
        .find(|c| c.id == middle.id)
        .unwrap()
        .composition
        .is_none());
    for c in subset.clips.iter().filter(|c| c.id != middle.id) {
        assert_eq!(c.composition_group(), group);
    }
    let actual = render_av(
        &ctx,
        &subset,
        &dir.path().join("lifecycle-subset-move.mkv"),
        false,
    );
    let mut expected = Av {
        pixels: vec![0; 105 * FRAME_BYTES],
        pcm: vec![0.; 105 * SAMPLES_PER_FRAME],
    };
    for (from, to, length) in [(0, 0, 15), (45, 45, 15), (15, 75, 30)] {
        expected.pixels[to * FRAME_BYTES..(to + length) * FRAME_BYTES]
            .copy_from_slice(&baseline.pixels[from * FRAME_BYTES..(from + length) * FRAME_BYTES]);
        expected.pcm[to * SAMPLES_PER_FRAME..(to + length) * SAMPLES_PER_FRAME].copy_from_slice(
            &baseline.pcm[from * SAMPLES_PER_FRAME..(from + length) * SAMPLES_PER_FRAME],
        );
    }
    compare(
        &expected,
        &actual,
        "Independent fragment move detaches composition only",
        1,
        0.000_004,
        0.000_002,
    );

    let mut duplicated = fragments.clone();
    edit::apply(
        &mut duplicated,
        EditCommand::Duplicate {
            ids: vec![middle.id.clone()],
        },
    )
    .unwrap();
    let duplicate = duplicated
        .clips
        .iter()
        .find(|c| !all_ids.contains(&c.id))
        .unwrap()
        .clone();
    assert_eq!((duplicate.start, duplicate.duration), (45, 30));
    assert_ne!(duplicate.composition_group(), group);
    assert_eq!(
        duplicate.composition.as_ref().unwrap().offset,
        0,
        "A duplicate subset gets a rebased independent group"
    );
    assert_eq!(
        (
            duplicate.fade_in_start,
            duplicate.fade_out_end,
            duplicate.render_offset
        ),
        (
            middle.fade_in_start,
            middle.fade_out_end,
            middle.render_offset
        )
    );
    edit::apply(
        &mut duplicated,
        EditCommand::Delete {
            ids: all_ids,
            ripple: false,
        },
    )
    .unwrap();
    let actual = render_av(
        &ctx,
        &duplicated,
        &dir.path().join("lifecycle-duplicate.mkv"),
        false,
    );
    let mut expected = Av {
        pixels: vec![0; 75 * FRAME_BYTES],
        pcm: vec![0.; 75 * SAMPLES_PER_FRAME],
    };
    expected.pixels[45 * FRAME_BYTES..]
        .copy_from_slice(&baseline.pixels[15 * FRAME_BYTES..45 * FRAME_BYTES]);
    expected.pcm[45 * SAMPLES_PER_FRAME..]
        .copy_from_slice(&baseline.pcm[15 * SAMPLES_PER_FRAME..45 * SAMPLES_PER_FRAME]);
    compare(
        &expected,
        &actual,
        "Duplicate subset retains envelope while rebasing composition",
        1,
        0.000_004,
        0.000_002,
    );
    eprintln!("VERIFIED decoded composition lifecycle: whole group move retains origin, subset move detaches only moved layer, duplicate subset receives a new rebased group; every inherited fade/volume sample and pixel preserved.");
}

#[test]
fn still_image_extended_source_offsets_survive_long_trims_and_repeated_splits() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("long-still.png");
    media::run_ffmpeg(
        &ctx,
        &vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=160x90:rate=30".into(),
            "-frames:v".into(),
            "1".into(),
            source.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    let mut original = Project::new("Long still image".into(), 160, 90, Rational::new(30, 1));
    original.media.push(media::probe(&ctx, &source).unwrap());
    assert_eq!(original.media[0].kind, "image");
    let mid = original.media[0].id.clone();
    let tid = original.tracks[0].id.clone();
    edit::apply(
        &mut original,
        EditCommand::AddClip {
            media_id: mid,
            track_id: tid,
            start: 0,
            source_in: Some(Rational::new(10, 1)),
            duration: Some(360),
        },
    )
    .unwrap();
    let base = original;
    let mut failures = vec![];
    for (name, with_fades, with_opacity) in [
        ("fades-only", true, false),
        ("opacity-only", false, true),
        ("fades-and-opacity", true, true),
    ] {
        let mut original = base.clone();
        let clip = original.clips[0].id.clone();
        patch(
            &mut original,
            &clip,
            json!({"fade_in":90,"fade_out":90,"keyframes":[
                {"property":"opacity","frame":0,"value":0.4},
                {"property":"opacity","frame":270,"value":0.9},
                {"property":"opacity","frame":359,"value":0.5}
            ]}),
        );
        if !with_fades {
            original.clips[0].fade_in = 0;
            original.clips[0].fade_out = 0;
        }
        if !with_opacity {
            original.clips[0].keyframes.clear();
        }
        let before = render_av(
            &ctx,
            &original,
            &dir.path().join(format!("still-{name}-before.mkv")),
            false,
        );
        assert!(before.pixels[300*FRAME_BYTES..301*FRAME_BYTES].iter().any(|v|*v>64),
        "An image with a 10-second source offset must remain visible beyond a nominal 5-second metadata duration");
        let mut edited = original;
        edit::apply(
            &mut edited,
            EditCommand::Trim {
                id: clip.clone(),
                edge: "in".into(),
                frame: 60,
            },
        )
        .unwrap();
        edit::apply(
            &mut edited,
            EditCommand::Trim {
                id: clip,
                edge: "out".into(),
                frame: 330,
            },
        )
        .unwrap();
        split(&mut edited, &[75, 180, 315]);
        let actual = render_av(
            &ctx,
            &edited,
            &dir.path().join(format!("still-{name}-after.mkv")),
            false,
        );
        let mut expected = Av {
            pixels: before.pixels[..330 * FRAME_BYTES].to_vec(),
            pcm: before.pcm[..330 * SAMPLES_PER_FRAME].to_vec(),
        };
        expected.pixels[..60 * FRAME_BYTES].fill(0);
        assert_eq!(expected.pixels.len(), actual.pixels.len());
        assert_eq!(expected.pcm.len(), actual.pcm.len());
        let max_pixel = expected
            .pixels
            .iter()
            .zip(&actual.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let first = expected
            .pixels
            .iter()
            .zip(&actual.pixels)
            .position(|(a, b)| a.abs_diff(*b) > 1)
            .map(|i| i / FRAME_BYTES);
        assert!(actual.pcm.iter().all(|s| s.abs() < 0.000_001));
        eprintln!("VERIFIED still image {name}, source_in=10s, original 12s hold, trims to frames60..330 and repeated splits75/180/315:330 frames/528000 silent samples, maxRGB {max_pixel},firstbadframe{first:?}.");
        if max_pixel > 1 {
            failures.push((name, max_pixel, first));
        }
    }
    assert!(
        failures.is_empty(),
        "Long image split/trim changes envelopes: {failures:?}"
    );
}

fn wait(manager: &JobManager, id: &str) -> Job {
    let start = Instant::now();
    loop {
        let job = manager.list().into_iter().find(|j| j.id == id).unwrap();
        if job.status != "running" {
            assert_eq!(job.status, "complete", "{job:?}");
            return job;
        }
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn trims_preserve_inherited_clock_explicit_fades_restart_and_invalid_metadata_is_transactional() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("trim-fade-red.mkv");
    fixture(&ctx, &source, "red", 2, false);
    let mut original = project(&ctx, &source, 60);
    let id = original.clips[0].id.clone();
    patch(&mut original, &id, json!({"fade_in":30,"fade_out":30}));
    let before = render_av(&ctx, &original, &dir.path().join("trim-base.mkv"), false);
    for (name, trim_in, trim_out) in [
        ("trim-in", true, false),
        ("trim-out", false, true),
        ("trim-both", true, true),
    ] {
        let mut trimmed = original.clone();
        let clip = trimmed.clips[0].id.clone();
        if trim_in {
            edit::apply(
                &mut trimmed,
                EditCommand::Trim {
                    id: clip.clone(),
                    edge: "in".into(),
                    frame: 15,
                },
            )
            .unwrap();
        }
        if trim_out {
            edit::apply(
                &mut trimmed,
                EditCommand::Trim {
                    id: clip,
                    edge: "out".into(),
                    frame: 45,
                },
            )
            .unwrap();
        }
        let actual = render_av(
            &ctx,
            &trimmed,
            &dir.path().join(format!("{name}.mkv")),
            false,
        );
        let frames = trimmed.length() as usize;
        let mut expected = Av {
            pixels: before.pixels[..frames * FRAME_BYTES].to_vec(),
            pcm: before.pcm[..frames * SAMPLES_PER_FRAME].to_vec(),
        };
        if trim_in {
            expected.pixels[..15 * FRAME_BYTES].fill(0);
            expected.pcm[..15 * SAMPLES_PER_FRAME].fill(0.);
        }
        compare(&expected, &actual, name, 1, 0.000_004, 0.000_002);
        eprintln!("VERIFIED {name}: clipped originalfade clock retained; no envelope retiming at retained edges.");
    }

    let mut split_project = project(&ctx, &source, 60);
    let first = split_project.clips[0].id.clone();
    patch(&mut split_project, &first, json!({"fade_in":30}));
    split(&mut split_project, &[15]);
    let inherited = render_av(
        &ctx,
        &split_project,
        &dir.path().join("explicit-before.mkv"),
        false,
    );
    let right = split_project
        .clips
        .iter()
        .find(|c| c.start == 15)
        .unwrap()
        .id
        .clone();
    patch(&mut split_project, &right, json!({"fade_in":15}));
    let explicit = render_av(
        &ctx,
        &split_project,
        &dir.path().join("explicit-after.mkv"),
        false,
    );
    assert_eq!(
        &inherited.pixels[..15 * FRAME_BYTES],
        &explicit.pixels[..15 * FRAME_BYTES],
        "Editing the right fade must preserve the left fragment"
    );
    assert_eq!(
        &inherited.pcm[..15 * SAMPLES_PER_FRAME],
        &explicit.pcm[..15 * SAMPLES_PER_FRAME]
    );
    let right_actual = Av {
        pixels: explicit.pixels[15 * FRAME_BYTES..].to_vec(),
        pcm: explicit.pcm[15 * SAMPLES_PER_FRAME..].to_vec(),
    };
    let right_reference = independent_segment_fade(
        &ctx,
        &source,
        &dir.path().join("explicit-known-fade.mkv"),
        15,
        45,
        15,
        0,
    );
    compare(
        &right_reference,
        &right_actual,
        "Explicit right fade restartsatfragmentlocalzero",
        5,
        0.000_004,
        0.000_002,
    );

    let valid_path = dir.path().join("valid-envelopes.monocut");
    fs::write(
        &valid_path,
        serde_json::to_vec_pretty(&split_project).unwrap(),
    )
    .unwrap();
    let recovery = dir.path().join("envelope-recovery.json");
    let mut store = ProjectStore::new(recovery.clone());
    store.open(&valid_path).unwrap();
    let before_model = store.get();
    let before_recovery = fs::read(&recovery).unwrap();
    let before_saved = fs::read(&valid_path).unwrap();
    for (name, field, value) in [
        ("overflow-fade-in", "fade_in_start", json!(i64::MAX)),
        ("overflow-fade-out", "fade_out_end", json!(i64::MIN)),
        (
            "overflow-composition",
            "composition",
            json!({"group_id":"overflow-test","offset":i64::MAX}),
        ),
        (
            "overflow-render-offset-max",
            "render_offset",
            json!(i64::MAX),
        ),
        (
            "overflow-render-offset-min",
            "render_offset",
            json!(i64::MIN),
        ),
        (
            "negative-inherited-source-origin",
            "render_offset",
            json!(1),
        ),
    ] {
        let mut raw = serde_json::to_value(&split_project).unwrap();
        raw["clips"][0][field] = value;
        let invalid = dir.path().join(format!("{name}.monocut"));
        fs::write(&invalid, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
        assert!(
            store.open(&invalid).is_err(),
            "Invalid inherited clock metadata must be rejected: {name}"
        );
        assert_eq!(store.get(), before_model);
        assert_eq!(fs::read(&recovery).unwrap(), before_recovery);
        assert_eq!(fs::read(&valid_path).unwrap(), before_saved);
    }
    eprintln!("VERIFIED inheritedtrim clocks,explicit fragmentfade reset againstindependentfixture,invalidanchor/compositionmetadata rejection preservesmodel/saved/recoverybytes.");
}

#[test]
fn inherited_envelopes_roundtrip_history_projects_recovery_proxies_and_actual_previews() {
    let dir = TempDir::new().unwrap();
    let ctx = context(dir.path());
    media::ensure_cache(&ctx).unwrap();
    let source = dir.path().join("roundtrip-pattern.mkv");
    fixture(&ctx, &source, "pattern", 2, true);
    let original = automated_linked_project(&ctx, &source);
    let before = render_av(
        &ctx,
        &original,
        &dir.path().join("roundtrip-before.mkv"),
        false,
    );
    let path = dir.path().join("envelopes.monocut");
    fs::write(&path, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    let recovery = dir.path().join("recovery.json");
    let mut store = ProjectStore::new(recovery.clone());
    store.open(&path).unwrap();
    let selected = store.get().clips[0].id.clone();
    let split_project = store
        .apply(
            EditCommand::Split {
                ids: vec![selected],
                frame: 15,
            },
            &ctx,
        )
        .unwrap();
    assert_link_pairs(&split_project);
    let undone = store.undo().unwrap();
    assert_eq!(undone, original);
    let undo_render = render_av(&ctx, &undone, &dir.path().join("undo.mkv"), false);
    compare(
        &before,
        &undo_render,
        "undo restoresoriginalenvelopes",
        1,
        0.000_004,
        0.000_002,
    );
    assert_eq!(store.redo().unwrap(), split_project);
    store.save(&path).unwrap();
    let mut reopened = ProjectStore::new(dir.path().join("reopened-recovery.json"));
    let reopened_project = reopened.open_hydrated(&path, &ctx).unwrap();
    assert_eq!(reopened_project.clips, split_project.clips);
    let mut recovered = ProjectStore::new(recovery);
    let recovered_project = recovered.recover_hydrated(&ctx).unwrap();
    assert_eq!(recovered_project.clips, split_project.clips);
    for (name, p) in [
        ("redo", split_project.clone()),
        ("reopen", reopened_project),
        ("recover", recovered_project),
    ] {
        let actual = render_av(&ctx, &p, &dir.path().join(format!("{name}.mkv")), false);
        compare(&before, &actual, name, 1, 0.000_004, 0.000_002);
        around_cuts(&before, &actual, &[15], name, 0.000_004);
    }
    let manager = JobManager::new(ctx.clone(), Arc::new(|_| {}));
    let job = manager.proxy(original.media[0].clone()).unwrap();
    let proxy = wait(&manager, &job.id).path.unwrap();
    let proxied = store
        .set_proxy(&original.media[0].id, PathBuf::from(proxy))
        .unwrap();
    let mut original_proxy = original.clone();
    original_proxy.media = proxied.media.clone();
    let proxy_before = render_av(
        &ctx,
        &original_proxy,
        &dir.path().join("proxy-before.mkv"),
        true,
    );
    let proxy_after = render_av(&ctx, &proxied, &dir.path().join("proxy-after.mkv"), true);
    compare(
        &proxy_before,
        &proxy_after,
        "proxy split inheritedenvelopes",
        1,
        0.000_004,
        0.000_002,
    );
    around_cuts(&proxy_before, &proxy_after, &[15], "proxy", 0.000_004);
    for use_proxies in [false, true] {
        let job = manager
            .preview(original_proxy.clone(), 180, use_proxies)
            .unwrap();
        let before_preview = av(
            &ctx,
            Path::new(wait(&manager, &job.id).path.as_ref().unwrap()),
        );
        let job = manager.preview(proxied.clone(), 180, use_proxies).unwrap();
        let after_preview = av(
            &ctx,
            Path::new(wait(&manager, &job.id).path.as_ref().unwrap()),
        );
        let stats = compare(
            &before_preview,
            &after_preview,
            &format!("actualAAC/H264preview proxies={use_proxies}"),
            8,
            0.003,
            0.000_5,
        );
        around_cuts(&before_preview, &after_preview, &[15], "AACpreview", 0.003);
        eprintln!("VERIFIED actualpreview inheritedenvelopes proxies={use_proxies}:maxRGB{},maxPCM{:.9},RMS{:.9}; codecaware limits8RGB/.003PCM/.0005RMS.",stats.0,stats.1,stats.2);
    }
    let export_before_path = dir.path().join("h264-before.mp4");
    let export_after_path = dir.path().join("h264-after.mp4");
    export(&ctx, &original, &export_before_path, false, "h264");
    export(&ctx, &proxied, &export_after_path, false, "h264");
    compare(
        &av(&ctx, &export_before_path),
        &av(&ctx, &export_after_path),
        "H264/AACexport splitconsistency",
        8,
        0.003,
        0.000_5,
    );
    eprintln!("VERIFIED inheritedenvelopes undo/redo/save/reopen/recovery,actualoriginal/proxyFFV1andAAC/H264previews/exports; all decodedcutadjacentPCM continuity checks pass.");
}
