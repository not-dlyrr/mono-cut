import type { Rational } from './types';
import type { PreviewRegion } from './previewRegion';

export type MonitorEvent = 'metadata' | 'play' | 'playing' | 'pause' | 'ended' | 'error';
export interface MonitorMediaState {
  currentSrc: string;
  readyState: number;
  currentTime: number;
  duration: number;
  paused: boolean;
  ended: boolean;
  error: unknown | null;
}

export function monitorClockActive(playing: boolean, speed: number, source: string | null, currentSource: string | null, currentNode: boolean): boolean {
  return playing && speed > 0 && source !== null && source === currentSource && currentNode;
}

/** A replaced element or transport intent cannot fail the newer play operation. */
export async function settleMonitorPlay(play: Promise<void>, current: () => boolean, pause: () => void, playing?: () => void): Promise<void> {
  try { await play; if (current()) playing?.(); } catch { if (current()) pause(); }
}

/** Guard queued events against replaced assets, seeks, and the current media element state. */
export function monitorEventAllowed(
  kind: MonitorEvent, video: MonitorMediaState, expectedSrc: string | null, currentNode: boolean,
  timestamp: number, generationStartedAt: number, timeOrigin: number, fps: Rational, region?: PreviewRegion | null,
): boolean {
  if (!currentNode || !expectedSrc || video.currentSrc !== expectedSrc) return false;
  // WebView events use a monotonic timestamp; older browsers may supply epoch milliseconds.
  const at = timestamp > 1e12 ? timestamp - timeOrigin : timestamp;
  if (!Number.isFinite(at) || at < generationStartedAt) return false;
  if (kind === 'metadata') return video.readyState >= 1 && Number.isFinite(video.duration);
  if (kind === 'play') return !video.paused && video.readyState >= 1;
  if (kind === 'playing') return !video.paused && video.readyState >= 2;
  if (kind === 'pause') return video.paused;
  if (kind === 'error') return video.error != null;
  if (!video.ended || !Number.isFinite(video.duration) || !Number.isFinite(video.currentTime)) return false;
  const expectedEnd = region ? (region.end_frame - region.start_frame) * fps.den / fps.num : video.duration;
  const tolerance = fps.den / fps.num / 2;
  return video.duration + tolerance >= expectedEnd && video.currentTime + tolerance >= expectedEnd;
}
