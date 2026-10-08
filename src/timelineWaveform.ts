// SPDX-License-Identifier: GPL-3.0-or-later
import type { Clip, Media, Project, Rational } from './types';

export const WAVEFORM_SAMPLE_RATE = 4000;
export const MAX_WAVEFORM_BINS = 1024;
export const MAX_WAVEFORM_POINTS = 512;
export const WAVEFORM_BUCKET_PIXELS = 2;
export interface TimelineDrag { id: string; mode: 'move' | 'in' | 'out' | 'slip'; delta: number; ids: string[] }
export function dragClipIds(project: Project, drag: TimelineDrag): Set<string> {
  const requested = new Set(drag.mode === 'move' ? drag.ids : [drag.id]);
  const groups = new Set(project.clips.filter(c => requested.has(c.id) && c.linked_id).map(c => c.linked_id));
  return new Set(project.clips.filter(c => requested.has(c.id) || (c.linked_id && groups.has(c.linked_id))).map(c => c.id));
}
export function displayedClip(clip: Clip, drag: TimelineDrag | null) {
  return {
    start: clip.start + (drag?.mode === 'move' || drag?.mode === 'in' ? drag.delta : 0),
    duration: Math.max(0, clip.duration + (drag?.mode === 'out' ? drag.delta : drag?.mode === 'in' ? -drag.delta : 0)),
    sourceFrameOffset: drag?.mode === 'in' || drag?.mode === 'slip' ? drag.delta : 0,
  };
}
type Fraction = { n: bigint; d: bigint };
const rational = (v: Rational): Fraction => ({ n: BigInt(v.num), d: BigInt(v.den) });
const add = (a: Fraction, b: Fraction): Fraction => ({ n: a.n * b.d + b.n * a.d, d: a.d * b.d });
const mul = (a: Fraction, b: Fraction): Fraction => ({ n: a.n * b.n, d: a.d * b.d });
const compare = (a: Fraction, b: Fraction) => a.n * b.d - b.n * a.d;
const minimum = (a: Fraction, b: Fraction) => compare(a, b) <= 0 ? a : b;
const maximum = (a: Fraction, b: Fraction) => compare(a, b) >= 0 ? a : b;
const zero: Fraction = { n: 0n, d: 1n };
// CSS geometry is decimal-valued; preserve that geometry exactly before combining
// it with rational project timing. No intermediate rounded sequence frames.
function decimal(v: number): Fraction {
  const [mantissa, exponent = '0'] = String(v).toLowerCase().split('e');
  const [whole, tail = ''] = mantissa.split('.');
  const scale = tail.length - Number(exponent);
  const n = BigInt(whole + tail);
  return scale >= 0 ? { n, d: 10n ** BigInt(scale) } : { n: n * 10n ** BigInt(-scale), d: 1n };
}
const valid = (r: Rational) => Number.isSafeInteger(r.num) && Number.isSafeInteger(r.den) && r.den > 0;
export interface WaveformPoint { x: number; peak: number }
export interface WaveformDrawing { left: number; width: number; points: WaveformPoint[]; binVisits: number }

/** Source peaks, before clip effects. Window and source ranges are half-open.
 * Native bin j owns samples [ceil(j*E/N), ceil((j+1)*E/N)), E=ceil(duration*4000).
 * Read every intersecting bin; bins cannot reveal the location of a peak inside
 * their own coarse interval. Work <= O(N + P log N), storage <= O(N + P).
 */
export function timelineWaveform(media: Pick<Media, 'waveform' | 'duration'>, clip: Pick<Clip, 'source_in' | 'speed'>,
  fps: Rational, duration: number, scale: number, left: number, right: number, sourceFrameOffset = 0): WaveformDrawing {
  const empty = { left: 0, width: 0, points: [], binVisits: 0 };
  const count = media.waveform.length;
  if (!count || count > MAX_WAVEFORM_BINS || ![media.duration, clip.source_in, clip.speed, fps].every(valid)
    || media.duration.num <= 0 || clip.speed.num <= 0 || fps.num <= 0 || !Number.isSafeInteger(duration) || duration <= 0
    || !Number.isSafeInteger(sourceFrameOffset) || ![scale, left, right].every(Number.isFinite) || scale <= 0) return empty;
  const start = Math.max(0, left), end = Math.min(duration * scale, right);
  if (end <= start) return empty;
  const width = end - start, points: WaveformPoint[] = [], buckets = Math.min(MAX_WAVEFORM_POINTS, Math.max(1, Math.ceil(width / WAVEFORM_BUCKET_PIXELS)));
  const extent = rational(media.duration), samples = BigInt(Math.max(1, Math.ceil(media.duration.num / media.duration.den * WAVEFORM_SAMPLE_RATE))), n = BigInt(count);
  // Explicit native sample boundaries also handle E < N (empty source bins).
  const boundaries = Array.from({ length: count + 1 }, (_, j) => ({ n: (BigInt(j) * samples + n - 1n) / n, d: 4000n }));
  const frameSeconds = { n: BigInt(fps.den), d: BigInt(fps.num) }, speed = rational(clip.speed), pixelScale = decimal(scale);
  const base = add(rational(clip.source_in), mul(mul({ n: BigInt(sourceFrameOffset), d: 1n }, frameSeconds), speed));
  const sourceAt = (px: number) => {
    const frames = px >= duration * scale ? { n: BigInt(duration), d: 1n } : mul(decimal(px), { n: pixelScale.d, d: pixelScale.n });
    return add(base, mul(mul(frames, frameSeconds), speed));
  };
  function firstEndingAfter(time: Fraction) {
    let lo = 0, hi = count;
    while (lo < hi) { const mid = (lo + hi) >>> 1; if (compare(boundaries[mid + 1], time) <= 0) lo = mid + 1; else hi = mid; }
    return lo;
  }
  let binVisits = 0;
  for (let i = 0; i < buckets; i++) {
    const lo = maximum(zero, sourceAt(start + width * i / buckets));
    const hi = minimum(extent, sourceAt(i + 1 === buckets ? end : start + width * (i + 1) / buckets));
    let peak = 0;
    if (compare(lo, hi) < 0) for (let j = firstEndingAfter(lo); j < count && compare(boundaries[j], hi) < 0; j++) {
      binVisits++;
      if (compare(boundaries[j], boundaries[j + 1]) < 0) {
        const v = media.waveform[j];
        if (Number.isFinite(v)) peak = Math.max(peak, Math.min(1, Math.max(0, v)));
      }
    }
    points.push({ x: width * (i + .5) / buckets, peak });
  }
  return { left: start, width, points, binVisits };
}
