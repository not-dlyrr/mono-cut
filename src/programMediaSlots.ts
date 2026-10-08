import { sameRegion, type PreviewRegion } from './previewRegion';
import { monitorEventAllowed, type MonitorMediaState } from './monitorLifecycle';
import type { Rational } from './types';

export interface ProgramMediaAsset { id: string; path: string; src: string; region: PreviewRegion; playable: boolean }
export interface ProgramMediaSlot { asset: ProgramMediaAsset; generation: number; startedAt: number; ready: boolean }
export type ProgramMediaOwnership = { current: ProgramMediaAsset | null; successor: ProgramMediaAsset | null };
export interface ProgramMediaNode { muted: boolean; pause(): void; load(): void; setAttribute(name: string, value: string): void; removeAttribute(name: string): void }
const matches = (slot: ProgramMediaSlot | null, asset: ProgramMediaAsset) => !!slot && slot.asset.id === asset.id && slot.asset.src === asset.src && sameRegion(slot.asset.region, asset.region);

/** Abort replaced resources before releasing their consuming pin; loaded promotion does not reload. */
export function commitProgramMedia(model: ProgramMediaSlots, nodes: (ProgramMediaNode | null)[], generations: (number | null)[]): void {
  for (let index = 0; index < 2; index++) {
    const node = nodes[index], slot = model.slots[index]; if (!node) continue;
    const generation = slot?.generation ?? null;
    if (generations[index] !== generation) {
      node.pause(); node.muted = true;
      if (slot) node.setAttribute('src', slot.asset.src); else node.removeAttribute('src');
      node.load(); generations[index] = generation;
    }
    if (index !== model.current) { node.pause(); node.muted = true; }
    else node.muted = false;
  }
}
export function detachProgramMedia(nodes: (ProgramMediaNode | null)[], generations: (number | null)[]): void {
  for (let index = 0; index < 2; index++) { const node = nodes[index]; if (node) { node.pause(); node.muted = true; node.removeAttribute('src'); node.load(); } generations[index] = null; }
}

/** Two persistent media nodes. Promotion preserves the candidate's loaded source and generation. */
export class ProgramMediaSlots {
  readonly slots: [ProgramMediaSlot | null, ProgramMediaSlot | null] = [null, null];
  current: 0 | 1 | null = null;
  successor: 0 | 1 | null = null;
  private serial = 0;
  ownership = 0;

  reconcile(current: ProgramMediaAsset | null, successor: ProgramMediaAsset | null, now: number): void {
    const oldCurrent = this.current === null ? null : this.slots[this.current];
    const find = (asset: ProgramMediaAsset) => this.slots.findIndex(slot => matches(slot, asset));
    let active = current ? find(current) : -1;
    if (current && active < 0) { active = successor ? 1 - Math.max(0, find(successor)) : this.current ?? 0; this.assign(active, current, now); }
    if (current) this.slots[active]!.asset = current;
    let next = successor && (!current || successor.id !== current.id) ? find(successor) : -1;
    if (successor && (!current || successor.id !== current.id) && next < 0) { next = active >= 0 ? 1 - active : this.successor ?? 0; this.assign(next, successor, now); }
    if (next >= 0 && successor) this.slots[next]!.asset = successor;
    this.current = active >= 0 ? active as 0 | 1 : null; this.successor = next >= 0 ? next as 0 | 1 : null;
    for (let i = 0; i < 2; i++) if (i !== active && i !== next) this.slots[i] = null;
    if ((oldCurrent?.generation ?? null) !== (this.current === null ? null : this.slots[this.current]?.generation)) this.ownership += 1;
  }
  private assign(index: number, asset: ProgramMediaAsset, now: number): void { this.slots[index] = { asset, generation: ++this.serial, startedAt: now, ready: false }; }
  owns(index: number, generation: number): boolean { return this.slots[index]?.generation === generation; }
  snapshot(): ProgramMediaOwnership { return { current: this.current === null ? null : this.slots[this.current]!.asset, successor: this.successor === null ? null : this.slots[this.successor]!.asset }; }

  admit(index: number, generation: number, media: MonitorMediaState, normalizedSrc: string, timestamp: number, timeOrigin: number, fps: Rational): boolean {
    const slot = this.slots[index];
    if (this.successor !== index || !this.owns(index, generation) || !slot || slot.ready || !media.paused || media.error != null) return false;
    if (!monitorEventAllowed('metadata', media, normalizedSrc, true, timestamp, slot.startedAt, timeOrigin, fps, slot.asset.region) || media.readyState < 3) return false;
    const duration = (slot.asset.region.end_frame - slot.asset.region.start_frame) * fps.den / fps.num, tolerance = fps.den / fps.num / 2;
    if (Math.abs(media.duration - duration) > tolerance || !Number.isFinite(media.currentTime) || media.currentTime < 0 || media.currentTime > tolerance || media.ended) return false;
    slot.ready = true; return true;
  }
}
