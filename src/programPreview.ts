import { PreviewBridge, previewErrorMessage, type PreviewSettings } from './previewBridge';
import { PreviewScheduler, type PreviewJob } from './previewScheduler';
import { lastSequenceFrame, previewRegion, regionContains, sameRegion, type PreviewRegion } from './previewRegion';
import type { Rational } from './types';

export const PREVIEW_DEBOUNCE_MS = 100;
export interface PreviewClock {
  now(): number;
  setTimeout(callback: () => void, milliseconds: number): unknown;
  clearTimeout(timer: unknown): void;
}
const clock: PreviewClock = {
  now: () => performance.now(),
  setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
  clearTimeout: timer => globalThis.clearTimeout(timer as ReturnType<typeof setTimeout>),
};
type PreviewStage = 'frame' | 'playback' | 'successor';
export interface PreviewTiming { kind: 'intent' | 'identity-ready' | 'dispatch' | 'asset' | 'successor-ready' | 'handoff' | 'pin'; at: number; stage: PreviewStage; region: PreviewRegion }
export interface ProgramPreviewCallbacks {
  asset(job: PreviewJob, region: PreviewRegion, playable: boolean): void;
  /** A validated file candidate; its media element must still load before admission. */
  successor?(job: PreviewJob | null, region: PreviewRegion | null): void;
  job(job: PreviewJob): void;
  unavailable(reason: 'seek' | 'invalidated'): void;
  error(error: unknown): void;
  timing?(event: PreviewTiming): void;
}

/** Production two-stage preparation; App and the silent benchmark use this same controller. */
export class ProgramPreviewController {
  private model: string | null = null;
  private hasClips = false;
  private fps: Rational = { num: 30, den: 1 };
  private duration = 0;
  private targetFrame = 0;
  private direction: 1 | -1 = 1;
  private stage: PreviewStage = 'frame';
  private region: PreviewRegion = { start_frame: 0, end_frame: 1 };
  private generation = 0;
  private programKey: string | null = null;
  private stageKey: string | null = null;
  private disposed = false;
  private pendingTimer: { handle: unknown; key: string; revision: number } | null = null;
  private playbackSpeed = 0;
  private shown: { job: PreviewJob; region: PreviewRegion; model: string; playable: boolean; pinned: boolean } | null = null;
  private next: { job: PreviewJob; region: PreviewRegion; model: string; ready: boolean } | null = null;

  constructor(
    private scheduler: PreviewScheduler,
    private bridge: PreviewBridge,
    private settings: () => Omit<PreviewSettings, 'region'>,
    private callbacks: ProgramPreviewCallbacks,
    private timer: PreviewClock = clock,
  ) {}

  get coverage(): PreviewRegion | null { return this.shown?.region ?? null; }
  get pendingRegion(): PreviewRegion { return { ...this.region }; }
  get playable(): boolean { return !!this.shown?.playable; }
  get successor(): { job: PreviewJob; region: PreviewRegion; ready: boolean } | null {
    return this.next ? { job: this.next.job, region: { ...this.next.region }, ready: this.next.ready } : null;
  }
  /** Owned monitor ticks update global recovery coordinates without scheduling work. */
  trackFrame(frame: number): void { if (!this.disposed) this.targetFrame = lastSequenceFrame(frame, this.duration); }

  private emit(kind: PreviewTiming['kind']) { this.callbacks.timing?.({ kind, at: this.timer.now(), stage: this.stage, region: { ...this.region } }); }
  private clearTimer() { if (this.pendingTimer) this.timer.clearTimeout(this.pendingTimer.handle); this.pendingTimer = null; }
  private setRegion(region: PreviewRegion, stage: PreviewStage) {
    this.clearTimer(); this.region = region; this.stage = stage; this.generation += 1; this.stageKey = null;
    this.scheduler.observe(this.model!, this.hasClips, region); this.emit('intent');
  }
  private resetAt(frame: number, reason: 'seek' | 'invalidated' = 'invalidated') {
    this.targetFrame = lastSequenceFrame(frame, this.duration);
    this.programKey = null;
    this.clearSuccessor(); this.shown = null; this.callbacks.unavailable(reason);
    this.scheduler.invalidate();
    this.setRegion({ start_frame: this.targetFrame, end_frame: this.targetFrame + 1 }, 'frame');
  }

  observe(model: string, fps: Rational, duration: number, frame: number, hasClips: boolean, resetSession = false): boolean {
    if (this.disposed) return false;
    if (resetSession) { this.scheduler.resetProject(); this.model = null; }
    const changed = model !== this.model || hasClips !== this.hasClips;
    this.model = model; this.hasClips = hasClips; this.fps = { ...fps }; this.duration = duration;
    if (changed) { this.playbackSpeed = 0; this.direction = 1; this.resetAt(frame); }
    else this.targetFrame = lastSequenceFrame(frame, duration);
    return changed;
  }

  /** Only explicit seeks outside validated coverage invalidate playback; playback ticks stay global. */
  seek(frame: number, direction: 1 | -1 = 1): boolean {
    this.direction = direction;
    const target = lastSequenceFrame(frame, this.duration); this.targetFrame = target;
    if (this.stage === 'successor') {
      this.clearSuccessor(); this.generation += 1; this.clearTimer();
      void this.bridge.cancel().catch(error => { if (!this.disposed) this.callbacks.error(error); });
      if (this.shown) this.setRegion({ ...this.shown.region }, 'playback');
      else { this.resetAt(target, 'seek'); return true; }
    }
    if (this.shown && this.shown.model === this.model && regionContains(this.shown.region, target)) return false;
    if (regionContains(this.region, target) && !this.shown) return false;
    this.resetAt(target, 'seek'); return true;
  }

  /** User transport intent, rather than temporary waiting speed, controls look-ahead. */
  setPlaybackSpeed(speed: number): void {
    if (this.disposed) return;
    this.playbackSpeed = speed;
    if (speed > 0) {
      if (!this.shown?.playable && this.stage !== 'successor' && !(this.shown && this.stage === 'frame')) void this.request(false);
      else this.prepareSuccessor();
      return;
    }
    if (this.stage !== 'successor') return;
    this.clearSuccessor(); this.clearTimer(); this.generation += 1;
    void this.bridge.cancel().catch(error => { if (!this.disposed) this.callbacks.error(error); });
    if (this.shown) this.setRegion({ ...this.shown.region }, 'playback');
    else this.setRegion({ start_frame: this.targetFrame, end_frame: this.targetFrame + 1 }, 'frame');
  }

  /** An explicit Stop also supersedes admission during the initial two-stage load. */
  stopPlayback(): void {
    if (this.disposed) return;
    this.playbackSpeed = 0; this.clearSuccessor(); this.clearTimer(); this.generation += 1;
    void this.bridge.cancel().catch(error => { if (!this.disposed) this.callbacks.error(error); });
    if (this.stage === 'successor') {
      if (this.shown) this.setRegion({ ...this.shown.region }, this.shown.playable ? 'playback' : 'frame');
      else this.setRegion({ start_frame: this.targetFrame, end_frame: this.targetFrame + 1 }, 'frame');
    }
  }

  /** Do not release the successor completion pin until its playback pin is acknowledged. */
  acknowledgeAsset(jobId: string): void {
    if (this.disposed || this.shown?.job.id !== jobId) return;
    this.shown.pinned = true; this.emit('pin'); this.prepareSuccessor();
  }

  private clearSuccessor(): void { this.next = null; this.callbacks.successor?.(null, null); }
  private prepareSuccessor(): void {
    const shown = this.shown;
    if (this.disposed || this.playbackSpeed <= 0 || !shown?.playable || !shown.pinned || shown.model !== this.model || this.stage === 'successor' || shown.region.end_frame >= this.duration) return;
    const start = shown.region.end_frame, count = Math.max(1, Math.ceil(5 * this.fps.num / this.fps.den));
    this.setRegion({ start_frame: start, end_frame: Math.min(this.duration, start + count) }, 'successor');
    void this.request(false);
  }

  successorReady(jobId: string): boolean {
    if (this.disposed || this.next?.job.id !== jobId || this.next.model !== this.model || this.stage !== 'successor' || this.playbackSpeed <= 0) return false;
    this.next.ready = true; this.emit('successor-ready');
    if (!this.shown && regionContains(this.next.region, this.targetFrame)) this.adoptSuccessor();
    return true;
  }

  successorFailed(jobId: string, error: unknown): void {
    if (this.disposed || this.next?.job.id !== jobId || this.stage !== 'successor') return;
    this.clearSuccessor(); this.scheduler.forgetReady(); this.callbacks.error(error);
  }

  private adoptSuccessor(): boolean {
    const next = this.next;
    if (!next?.ready || next.model !== this.model || this.playbackSpeed <= 0 || !regionContains(next.region, this.targetFrame)) return false;
    this.next = null; this.stage = 'playback';
    this.shown = { job: next.job, region: { ...next.region }, model: next.model, playable: true, pinned: false };
    this.emit('handoff'); this.callbacks.asset(next.job, { ...next.region }, true); this.callbacks.successor?.(null, null); return true;
  }

  /** Coverage advance keeps a ready adjoining asset and the signed playing intent. */
  advance(frame: number, direction: 1 | -1 = 1): boolean {
    if (direction < 0) return this.seek(frame, direction);
    this.direction = direction; this.targetFrame = lastSequenceFrame(frame, this.duration);
    if (this.shown && regionContains(this.shown.region, this.targetFrame)) return false;
    if (this.stage === 'successor' && regionContains(this.region, this.targetFrame)) {
      if (this.next?.ready && this.adoptSuccessor()) return false;
      if (this.shown) { this.shown = null; this.callbacks.unavailable('seek'); }
      return true;
    }
    return this.seek(frame, direction);
  }

  async request(manual = false): Promise<void> {
    if (this.disposed || !this.hasClips) return;
    const generation = this.generation, settings = { ...this.settings(), region: { ...this.region } };
    try {
      const checked = await this.bridge.check(settings);
      if (!checked || this.disposed || generation !== this.generation) return;
      const { decision, identity } = checked;
      if (decision.recheck) { await this.request(manual); return; }
      this.emit('identity-ready');
      if ((this.programKey && this.programKey !== identity.program_key) || (this.stageKey && this.stageKey !== identity.key)) {
        this.resetAt(this.targetFrame); await this.request(manual); return;
      }
      if (decision.cacheInvalidated) {
        if (this.stage === 'successor') this.clearSuccessor();
        else { this.resetAt(this.targetFrame); await this.request(manual); return; }
      }
      this.programKey = identity.program_key!; this.stageKey = identity.key;
      if (decision.ready) { this.apply(decision.ready); return; }
      if (!this.scheduler.needsRender()) return;
      const revision = this.scheduler.contextRevision;
      const begin = async () => {
        if (this.disposed || generation !== this.generation || !this.scheduler.isCurrentContext(revision) || this.scheduler.expectedKey !== identity.key) return;
        try {
          this.emit('dispatch'); const job = await this.bridge.render(settings);
          if (!job || this.disposed || generation !== this.generation) return;
          this.callbacks.job(job); this.apply(job);
        } catch (error) {
          if (!this.disposed && generation === this.generation && this.scheduler.isCurrentContext(revision)) {
            if (previewErrorMessage(error).startsWith('PREVIEW_IDENTITY_CHANGED:')) this.resetAt(this.targetFrame);
            this.callbacks.error(error);
          }
        }
      };
      if (manual || this.stage !== 'frame') { this.clearTimer(); await begin(); }
      else if (!this.pendingTimer || this.pendingTimer.key !== identity.key || this.pendingTimer.revision !== revision) {
        this.clearTimer(); const handle = this.timer.setTimeout(() => { this.pendingTimer = null; void begin(); }, PREVIEW_DEBOUNCE_MS);
        this.pendingTimer = { handle, key: identity.key, revision };
      }
    } catch (error) {
      if (!this.disposed && generation === this.generation) {
        if (this.stage === 'successor' && !previewErrorMessage(error).startsWith('PREVIEW_IDENTITY_CHANGED:')) {
          // A failed look-ahead inspection does not invalidate a file already
          // being consumed. Cancel its work and validate that slot again on retry.
          this.clearSuccessor(); this.clearTimer(); this.generation += 1; this.stageKey = null;
          void this.bridge.cancel().catch(cancelError => { if (!this.disposed) this.callbacks.error(cancelError); });
        } else this.resetAt(this.targetFrame);
        this.callbacks.error(error);
      }
    }
  }

  receive(job: PreviewJob): void {
    if (this.disposed) return;
    const accepted = this.scheduler.receiveEvent(job); if (accepted) this.apply(accepted);
  }
  private apply(job: PreviewJob): void {
    if (!this.scheduler.canApply(job)) return;
    if (job.status === 'failed' || job.status === 'cancelled') {
      if (job.status === 'failed') {
        if (job.error?.startsWith('PREVIEW_IDENTITY_CHANGED:')) this.resetAt(this.targetFrame);
        this.callbacks.error(job.error || 'Preview preparation failed.');
      }
      return;
    }
    if (job.status !== 'complete' || !job.path || !this.scheduler.markReady(job)) return;
    if (this.stage === 'successor') {
      if (this.next?.job.id === job.id && this.next.job.path === job.path) return;
      this.next = { job, region: { ...this.region }, model: this.model!, ready: false };
      this.callbacks.successor?.(job, { ...this.region }); return;
    }
    if (!regionContains(this.region, this.targetFrame)) return;
    if (this.shown?.job.id === job.id && this.shown.job.path === job.path && sameRegion(this.shown.region, this.region)) return;
    const continuous = previewRegion(this.targetFrame, this.duration, this.fps, 5, this.direction);
    const playable = this.stage === 'playback' || sameRegion(this.region, continuous);
    this.shown = { job, region: { ...this.region }, model: this.model!, playable, pinned: false };
    this.emit('asset'); this.callbacks.asset(job, { ...this.region }, playable);
    if (!playable) {
      this.setRegion(continuous, 'playback'); void this.request(false);
    }
  }
  /** An ended asset advances to its exact half-open boundary; monitor time never becomes local. */
  boundary(): number | null {
    const next = this.shown?.region.end_frame;
    return next !== undefined && next < this.duration ? next : null;
  }
  cancelPreparation(): void { this.clearSuccessor(); this.playbackSpeed = 0; this.clearTimer(); this.generation += 1; void this.bridge.cancel().catch(error => { if (!this.disposed) this.callbacks.error(error); }); this.scheduler.resetProject(); this.model = null; }
  activate(): void { this.disposed = false; }
  dispose(): void { this.disposed = true; this.clearSuccessor(); this.playbackSpeed = 0; this.clearTimer(); this.shown = null; void this.bridge.cancel().catch(() => {}); this.scheduler.resetProject(); }
}
