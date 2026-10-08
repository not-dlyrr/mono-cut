// SPDX-License-Identifier: GPL-3.0-or-later
import type { Rational, RetimeClipCommand } from './types';

export type SpeedDraft = { speed: Rational; error: null } | { speed: null; error: string };
const rangeError = 'Enter a complete speed from 0.05 to 32×.';

/** Preserve the decimal request exactly within the native project format limits. */
export function parseSpeedDraft(draft: string): SpeedDraft {
  const text = draft.trim();
  const match = /^\+?(?:(\d+)(?:\.(\d+))?|\.(\d+))(?:[eE]([+-]?\d+))?$/.exec(text);
  const value = Number(text);
  if (!match || !Number.isFinite(value) || value < 0.05 || value > 32) return { speed: null, error: rangeError };
  if (text.length > 128) return { speed: null, error: 'Use fewer decimal places for speed.' };
  const fraction = match[2] ?? match[3] ?? '';
  const scale = fraction.length - Number(match[4] ?? 0);
  if (!Number.isSafeInteger(scale) || Math.abs(scale) > 128) return { speed: null, error: 'Use fewer decimal places for speed.' };
  let num = BigInt((match[1] ?? '0') + fraction), den = 1n;
  if (scale >= 0) den = 10n ** BigInt(scale); else num *= 10n ** BigInt(-scale);
  let a = num, b = den;
  while (b) [a, b] = [b, a % b];
  num /= a; den /= a;
  if (num * 20n < den || num > den * 32n) return { speed: null, error: rangeError };
  if (num > 1_000_000_000_000n || den > 1_000_000_000n) return { speed: null, error: 'Use fewer decimal places for speed.' };
  return { speed: { num: Number(num), den: Number(den) }, error: null };
}

export function retimeCommand(id: string, current: Rational, draft: string): { command: RetimeClipCommand | null; error: string | null } {
  // A persisted rational such as 1/3 can have a longer decimal display than the
  // project accepts as a new exact request. Focusing that unchanged value is inert.
  if (draft.trim() === String(current.num / current.den)) return { command: null, error: null };
  const parsed = parseSpeedDraft(draft);
  if (parsed.error !== null) return { command: null, error: parsed.error };
  if (BigInt(parsed.speed.num) * BigInt(current.den) === BigInt(current.num) * BigInt(parsed.speed.den)) return { command: null, error: null };
  return { command: { type: 'retime_clip', id, speed: parsed.speed }, error: null };
}
