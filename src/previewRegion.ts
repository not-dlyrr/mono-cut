import type { Rational } from './types';

/** Exact half-open sequence-frame coverage. The asset's local frame zero is start_frame. */
export interface PreviewRegion { start_frame: number; end_frame: number }
export function sameRegion(a?: PreviewRegion | null, b?: PreviewRegion | null): boolean {
  return a == null || b == null ? a == null && b == null : a.start_frame === b.start_frame && a.end_frame === b.end_frame;
}
export function validRegion(region: PreviewRegion): boolean {
  return Number.isSafeInteger(region.start_frame) && Number.isSafeInteger(region.end_frame) && region.start_frame >= 0 && region.end_frame > region.start_frame;
}
export function regionContains(region: PreviewRegion, frame: number): boolean { return frame >= region.start_frame && frame < region.end_frame; }
export function lastSequenceFrame(frame: number, duration: number): number { return Math.max(0, Math.min(Math.max(0, duration - 1), Math.round(frame))); }
export function previewRegion(frame: number, duration: number, fps: Rational, seconds = 5, direction: 1 | -1 = 1): PreviewRegion {
  const count = Math.min(Math.max(1, duration), Math.max(1, Math.ceil(seconds * fps.num / fps.den)));
  const target = lastSequenceFrame(frame, duration);
  const start = Math.min(Math.max(0, direction < 0 ? target - count + 1 : target), Math.max(0, duration - count));
  return { start_frame: start, end_frame: start + count };
}
export function localPreviewTime(frame: number, fps: Rational, region?: PreviewRegion | null): number {
  const local = region ? Math.max(0, Math.min(region.end_frame - region.start_frame - 1, frame - region.start_frame)) : Math.max(0, frame);
  return local * fps.den / fps.num;
}
export function globalPreviewFrame(time: number, fps: Rational, region?: PreviewRegion | null): number {
  const local = Math.max(0, Math.round(time * fps.num / fps.den));
  return region ? Math.min(region.end_frame - 1, region.start_frame + local) : local;
}
