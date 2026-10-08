export interface Rational { num: number; den: number }
export interface SourceTiming { origin: Rational; video_start: Rational | null; audio_start: Rational | null; video_end: Rational | null; audio_end: Rational | null; video_stream: number | null; audio_stream: number | null; container_duration?: Rational }
export interface Media { id: string; name: string; path: string; kind: 'video' | 'audio' | 'image'; duration: Rational; fps: Rational; width: number; height: number; has_audio: boolean; bin_id: string | null; thumbnail: string | null; waveform: number[]; proxy: string | null; timing?: SourceTiming; proxy_timing_version?: number; legacy_source?: { duration: Rational; sha256: string | null }; missing: boolean }
export interface Bin { id: string; name: string }
export interface Track { id: string; name: string; kind: 'video' | 'audio'; muted: boolean; hidden: boolean; locked: boolean }
export interface Keyframe { property: 'opacity' | 'volume' | 'x' | 'y' | 'scale'; frame: number; value: number }
export interface Transform { x: number; y: number; scale: number; rotation: number; crop_left: number; crop_right: number; crop_top: number; crop_bottom: number }
export interface Clip { id: string; media_id: string | null; track_id: string; name: string; start: number; duration: number; source_in: Rational; speed: Rational; linked_id: string | null; title: string | null; transform: Transform; opacity: number; volume: number; fade_in: number; fade_out: number; fade_in_start?: number | null; fade_out_end?: number | null; composition?: { group_id: string; offset: number } | null; render_offset?: number | null; brightness: number; contrast: number; saturation: number; keyframes: Keyframe[] }
export interface Marker { id: string; frame: number; name: string }
export interface Project { version: number; id: string; name: string; width: number; height: number; fps: Rational; sample_rate: number; media: Media[]; bins: Bin[]; tracks: Track[]; clips: Clip[]; markers: Marker[]; in_point: number | null; out_point: number | null }
export interface Job { id: string; kind: 'preview' | 'export' | 'proxy'; status: 'running' | 'complete' | 'cancelled' | 'failed'; progress: number; path: string | null; error: string | null; preview_key?: string | null }
export interface Capabilities { ffmpeg: string; ffprobe: string; version: string; encoders: string[]; hardware: string[]; gpu_devices: string[]; cache_dir: string }
export type EditCommand = { type: string; [key: string]: unknown };
export interface ExportSettings { width: number; height: number; fps: Rational; codec: 'h264' | 'ffv1'; crf: number; audio_bitrate: number; sample_rate: number }
export type Edit = (command: EditCommand) => Promise<void>;
export const fpsValue = (r: Rational) => r.num / r.den;
export const seconds = (r: Rational) => r.num / r.den;
export const endFrame = (p: Project) => Math.max(0, ...p.clips.map(c => c.start + c.duration));
export function timecode(frame: number, fps: Rational) {
  const rate = Math.round(fpsValue(fps)); const total = Math.max(0, Math.floor(frame));
  const f = total % rate, s = Math.floor(total / rate) % 60, m = Math.floor(total / rate / 60) % 60, h = Math.floor(total / rate / 3600);
  return [h, m, s, f].map(n => String(n).padStart(2, '0')).join(':');
}
export const durationLabel = (r: Rational) => `${Math.floor(seconds(r) / 60)}:${String(Math.floor(seconds(r) % 60)).padStart(2, '0')}`;
