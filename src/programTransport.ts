import { regionContains, type PreviewRegion } from './previewRegion';

/** A signed shuttle intent may resume only its own validated coverage advance. */
export class ProgramTransport {
  private speed = 0;
  private revision = 0;
  private advancing = false;
  private pending: { speed: number; frame: number; revision: number } | null = null;

  constructor(private changed: (speed: number) => void) {}
  get currentSpeed(): number { return this.speed; }

  request(value: number | ((speed: number) => number)): void {
    this.revision += 1; this.pending = null;
    this.speed = typeof value === 'function' ? value(this.speed) : value;
    this.changed(this.speed);
  }
  cancelResume(): void { this.revision += 1; this.pending = null; }
  invalidate(coverageAdvance = false): void {
    if (!this.advancing || !coverageAdvance) { this.request(0); return; }
    this.speed = 0; this.pending = null; this.changed(0);
  }

  advance(frame: number, move: (frame: number, direction: 1 | -1) => boolean): boolean {
    const speed = this.speed, revision = this.revision;
    if (!speed) return false;
    const target = Math.max(0, Math.round(frame));
    this.advancing = true;
    try {
      if (move(target, speed < 0 ? -1 : 1) && this.revision === revision) {
        this.pending = { speed, frame: target, revision };
        this.speed = 0; this.changed(0);
      }
      return true;
    } finally { this.advancing = false; }
  }

  resume(region: PreviewRegion, playable: boolean): void {
    const pending = this.pending;
    if (!playable || !pending || pending.revision !== this.revision || !regionContains(region, pending.frame)) return;
    this.pending = null; this.speed = pending.speed; this.changed(this.speed);
  }
}
