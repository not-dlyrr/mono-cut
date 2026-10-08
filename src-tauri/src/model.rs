use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}

/// Reduced, exact rational. Project times are sequence frames; source times are seconds.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}
impl Rational {
    pub fn new(num: i64, den: i64) -> Self {
        assert_ne!(den, 0);
        let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1) as i64;
        Self {
            num: num / g * if den < 0 { -1 } else { 1 },
            den: den.abs() / g,
        }
    }
    pub fn zero() -> Self {
        Self { num: 0, den: 1 }
    }
    pub fn one() -> Self {
        Self { num: 1, den: 1 }
    }
    pub fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }
    pub fn add(self, other: Self) -> Self {
        Self::from_i128(
            self.num as i128 * other.den as i128 + other.num as i128 * self.den as i128,
            self.den as i128 * other.den as i128,
        )
    }
    pub fn mul(self, other: Self) -> Self {
        Self::from_i128(
            self.num as i128 * other.num as i128,
            self.den as i128 * other.den as i128,
        )
    }
    pub fn checked_add(self, other: Self) -> Result<Self, String> {
        Self::checked_i128(
            self.num as i128 * other.den as i128 + other.num as i128 * self.den as i128,
            self.den as i128 * other.den as i128,
        )
    }
    pub fn checked_mul(self, other: Self) -> Result<Self, String> {
        Self::checked_i128(
            self.num as i128 * other.num as i128,
            self.den as i128 * other.den as i128,
        )
    }
    pub fn checked_sub(self, other: Self) -> Result<Self, String> {
        Self::checked_i128(
            self.num as i128 * other.den as i128 - other.num as i128 * self.den as i128,
            self.den as i128 * other.den as i128,
        )
    }
    fn checked_i128(mut n: i128, mut d: i128) -> Result<Self, String> {
        if d <= 0 {
            return Err("Invalid rational denominator".into());
        }
        let (mut a, mut b) = (n.abs(), d);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1);
        n /= g;
        d /= g;
        if n < i64::MIN as i128 || n > i64::MAX as i128 || d > i64::MAX as i128 {
            return Err("Time precision exceeds the project format limits".into());
        }
        Ok(Self {
            num: n as i64,
            den: d as i64,
        })
    }
    pub fn from_frames(frames: i64, fps: Self) -> Self {
        Self::from_i128(frames as i128 * fps.den as i128, fps.num as i128)
    }
    pub fn frames_floor(self, fps: Self) -> i64 {
        ((self.num as i128 * fps.num as i128).div_euclid(self.den as i128 * fps.den as i128)) as i64
    }
    pub fn from_i128(mut n: i128, mut d: i128) -> Self {
        let (mut a, mut b) = (n.abs(), d.abs());
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1);
        n /= g;
        d /= g;
        assert!(
            n >= i64::MIN as i128 && n <= i64::MAX as i128 && d <= i64::MAX as i128 && d > 0,
            "rational overflow"
        );
        Self {
            num: n as i64,
            den: d as i64,
        }
    }
    pub fn validate(self, positive: bool) -> Result<(), String> {
        if self.den <= 0
            || self.den > 1_000_000_000
            || self.num.unsigned_abs() > 1_000_000_000_000
            || (positive && self.num <= 0)
        {
            Err("Invalid rational value".into())
        } else {
            Ok(())
        }
    }
}
impl Default for Rational {
    fn default() -> Self {
        Self::zero()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceTiming {
    /// Shared source zero is the earliest selected stream's presentation start.
    pub origin: Rational,
    pub video_start: Option<Rational>,
    pub audio_start: Option<Rational>,
    #[serde(default)]
    pub video_end: Option<Rational>,
    #[serde(default)]
    pub audio_end: Option<Rational>,
    #[serde(default)]
    pub video_stream: Option<u32>,
    #[serde(default)]
    pub audio_stream: Option<u32>,
    /// Original format.duration decimal, retained for pre-timing v1 migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_duration: Option<Rational>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacySourceCompatibility {
    /// Measured span on the shared source clock, excluding the old saved tail.
    pub duration: Rational,
    /// Available legacy originals are identified by streamed content SHA-256.
    pub sha256: Option<String>,
}
/// Normalized caches contain leading black/silence and start at shared source zero.
pub const NORMALIZED_TIMING_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Media {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub duration: Rational,
    pub fps: Rational,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub bin_id: Option<String>,
    pub thumbnail: Option<String>,
    pub waveform: Vec<f32>,
    pub proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<SourceTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_timing_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_source: Option<LegacySourceCompatibility>,
    pub missing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bin {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub crop_left: f64,
    pub crop_right: f64,
    pub crop_top: f64,
    pub crop_bottom: f64,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            scale: 1.,
            rotation: 0.,
            crop_left: 0.,
            crop_right: 0.,
            crop_top: 0.,
            crop_bottom: 0.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    pub property: String,
    pub frame: i64,
    pub value: f64,
}
/// Retain the original compositing order when one layer becomes several clips.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CompositionOrigin {
    pub group_id: String,
    pub offset: i64,
}
/// Automation positions are exact source-progress seconds, relative to source_in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceKeyframe {
    pub property: String,
    pub time: Rational,
    pub value: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceEnvelope {
    pub keyframes: Vec<SourceKeyframe>,
    pub fade_in: Rational,
    pub fade_out: Rational,
    pub fade_in_start: Rational,
    pub fade_out_end: Rational,
}
/// Durable retiming state. Integer timeline fields are projections of this state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipRetime {
    pub source_span: Rational,
    pub envelope: SourceEnvelope,
    pub render_source_origin: Rational,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_source_offset: Option<Rational>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: String,
    pub media_id: Option<String>,
    pub track_id: String,
    pub name: String,
    pub start: i64,
    pub duration: i64,
    pub source_in: Rational,
    pub speed: Rational,
    pub linked_id: Option<String>,
    pub title: Option<String>,
    pub transform: Transform,
    pub opacity: f64,
    pub volume: f64,
    pub fade_in: i64,
    pub fade_out: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_end: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<CompositionOrigin>,
    /// Sequence-frame offset from the source conversion clock inherited by cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_offset: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retime: Option<ClipRetime>,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub keyframes: Vec<Keyframe>,
}
impl Clip {
    pub fn end(&self) -> i64 {
        self.start.saturating_add(self.duration)
    }
    pub fn fade_in_start(&self) -> i64 {
        self.fade_in_start.unwrap_or(0)
    }
    pub fn fade_out_end(&self) -> i64 {
        self.fade_out_end.unwrap_or(self.duration)
    }
    pub fn composition_start(&self) -> i64 {
        self.start - self.composition.as_ref().map(|c| c.offset).unwrap_or(0)
    }
    pub fn composition_group(&self) -> &str {
        self.composition
            .as_ref()
            .map(|c| c.group_id.as_str())
            .unwrap_or(&self.id)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    pub id: String,
    pub frame: i64,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub sample_rate: u32,
    pub media: Vec<Media>,
    pub bins: Vec<Bin>,
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub markers: Vec<Marker>,
    pub in_point: Option<i64>,
    pub out_point: Option<i64>,
}
impl Default for Project {
    fn default() -> Self {
        Self::new("Untitled".into(), 1920, 1080, Rational { num: 30, den: 1 })
    }
}
impl Project {
    pub fn new(name: String, width: u32, height: u32, fps: Rational) -> Self {
        Self {
            version: 1,
            id: id(),
            name,
            width,
            height,
            fps,
            sample_rate: 48000,
            media: vec![],
            bins: vec![],
            tracks: vec![
                Track {
                    id: id(),
                    name: "Video 1".into(),
                    kind: "video".into(),
                    muted: false,
                    hidden: false,
                    locked: false,
                },
                Track {
                    id: id(),
                    name: "Audio 1".into(),
                    kind: "audio".into(),
                    muted: false,
                    hidden: false,
                    locked: false,
                },
            ],
            clips: vec![],
            markers: vec![],
            in_point: None,
            out_point: None,
        }
    }
    pub fn length(&self) -> i64 {
        self.clips.iter().map(Clip::end).max().unwrap_or(1).max(1)
    }
    pub fn validate(&self) -> Result<(), String> {
        crate::edit::validate_project(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditCommand {
    AddClip {
        media_id: String,
        track_id: String,
        start: i64,
        source_in: Option<Rational>,
        duration: Option<i64>,
    },
    AddTitle {
        track_id: String,
        start: i64,
        duration: i64,
        text: String,
    },
    Move {
        ids: Vec<String>,
        delta: i64,
        track_id: Option<String>,
    },
    Split {
        ids: Vec<String>,
        frame: i64,
    },
    Trim {
        id: String,
        edge: String,
        frame: i64,
    },
    Delete {
        ids: Vec<String>,
        ripple: bool,
    },
    Duplicate {
        ids: Vec<String>,
    },
    Slip {
        id: String,
        delta: i64,
    },
    RetimeClip {
        id: String,
        speed: Rational,
    },
    Link {
        ids: Vec<String>,
    },
    Unlink {
        ids: Vec<String>,
    },
    UpdateClip {
        id: String,
        patch: Value,
    },
    AddTrack {
        kind: String,
        name: String,
    },
    UpdateTrack {
        id: String,
        patch: Value,
    },
    CrossDissolve {
        id: String,
        frames: i64,
    },
    AddBin {
        name: String,
    },
    AssignBin {
        media_id: String,
        bin_id: Option<String>,
    },
    Relink {
        media_id: String,
        path: String,
    },
    Marker {
        frame: i64,
        name: String,
    },
    RemoveMarker {
        id: String,
    },
    SetRange {
        in_point: Option<i64>,
        out_point: Option<i64>,
    },
    Rename {
        name: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportSettings {
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
    pub codec: String,
    pub crf: u32,
    pub audio_bitrate: u32,
    pub sample_rate: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewRegion {
    pub start_frame: i64,
    pub end_frame: i64,
}
impl PreviewRegion {
    pub fn validate(&self, p: &Project) -> Result<(), String> {
        if self.start_frame < 0 || self.end_frame <= self.start_frame || self.end_frame > p.length()
        {
            return Err("Preview coverage must be a nonempty range within the sequence".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub progress: f64,
    pub path: Option<String>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_region: Option<PreviewRegion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub ffmpeg: String,
    pub ffprobe: String,
    pub version: String,
    pub encoders: Vec<String>,
    pub hardware: Vec<String>,
    pub gpu_devices: Vec<String>,
    pub cache_dir: String,
}
