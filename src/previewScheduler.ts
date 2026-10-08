import { sameRegion, type PreviewRegion } from './previewRegion';

export interface PreviewJob {
  id: string;
  kind: string;
  status: 'running' | 'complete' | 'cancelled' | 'failed';
  progress: number;
  path: string | null;
  error: string | null;
  preview_key?: string | null;
  preview_region?: PreviewRegion | null;
}
export interface IdentityTicket { revision: number; serial: number; readySerial: number }
export interface RenderTicket { revision: number; serial: number; key: string; region?: PreviewRegion | null }

/** A late running response must never undo an earlier completion event. */
export function mergePreviewJob<T extends PreviewJob>(previous: T | undefined, next: T): T {
  if (!previous || previous.id !== next.id) return next;
  if (previous.status !== 'running') return previous;
  if (next.status !== 'running') return next;
  return next.progress >= previous.progress ? next : previous;
}

/** Key and job-ID arbitration, independent of React, timers, and the native bridge. */
export class PreviewScheduler {
  private descriptor: string | null = null;
  private revision = 0;
  private identitySerial = 0;
  private renderSerial = 0;
  private readySerial = 0;
  private key: string | null = null;
  private activeId: string | null = null;
  private pending: RenderTicket | null = null;
  private ready: PreviewJob | null = null;
  private hasClips = false;
  private events = new Map<string, PreviewJob>();
  private region: PreviewRegion | null = null;

  get expectedKey() { return this.key; }
  get activeJobId() { return this.activeId; }
  get contextRevision() { return this.revision; }

  /** A failed validation invalidates active replies while retaining a candidate for future cache checks. */
  invalidate(): void {
    this.revision += 1; this.identitySerial += 1; this.key = null; this.activeId = null; this.pending = null;
  }

  observe(descriptor: string, hasClips: boolean, region: PreviewRegion | null = null): boolean {
    if (this.descriptor === descriptor && this.hasClips === hasClips && sameRegion(this.region, region)) return false;
    this.region = region;
    this.descriptor = descriptor; this.hasClips = hasClips; this.revision += 1;
    this.identitySerial += 1; this.key = null; this.activeId = null; this.pending = null;
    if (!hasClips) this.ready = null;
    return true;
  }

  /** Opening/recovering another project is a new session, even if its ID is identical. */
  resetProject(): void {
    this.descriptor = null; this.revision += 1; this.identitySerial += 1;
    this.key = null; this.activeId = null; this.pending = null; this.ready = null; this.hasClips = false;
    this.region = null;
  }

  beginIdentity(): IdentityTicket { return { revision: this.revision, serial: ++this.identitySerial, readySerial: this.readySerial }; }
  /** Stop supersedes even a native render admission whose reply has not arrived. */
  cancelIntent(): number { this.invalidate(); return this.identitySerial; }
  forgetReady(): void { this.ready = null; this.readySerial += 1; }
  isCurrentIdentity(ticket: IdentityTicket): boolean { return ticket.revision === this.revision && ticket.serial === this.identitySerial; }
  isCurrentContext(revision: number): boolean { return revision === this.revision; }

  acceptIdentity(ticket: IdentityTicket, key: string, cachedPath: string | null = null): { changed: boolean; cacheInvalidated: boolean; recheck: boolean; ready: PreviewJob | null } | null {
    if (!this.isCurrentIdentity(ticket)) return null;
    const changed = key !== this.key;
    // The native cache sample may predate a completion delivered while this check was in flight.
    // Validate that newly completed output again before declaring it invalid.
    if (!changed && this.ready?.preview_key === key && this.ready.path !== cachedPath && ticket.readySerial !== this.readySerial) {
      return { changed: false, cacheInvalidated: false, recheck: true, ready: null };
    }
    if (changed) { this.key = key; this.activeId = null; this.pending = null; }
    const cacheInvalidated = this.ready?.preview_key === key && this.ready.path !== cachedPath;
    if (cacheInvalidated) { this.ready = null; this.activeId = null; }
    const ready = this.hasClips && this.ready?.preview_key === key && this.ready.path === cachedPath && sameRegion(this.ready.preview_region, this.region) ? this.ready : null;
    if (ready) this.activeId = ready.id;
    return { changed, cacheInvalidated, recheck: false, ready };
  }

  needsRender(): boolean {
    if (!this.hasClips || !this.key || (this.ready?.preview_key === this.key && sameRegion(this.ready.preview_region, this.region)) || this.pending) return false;
    const active = this.activeId ? this.events.get(this.activeId) : undefined;
    return !active || active.status !== 'running';
  }

  beginRender(): RenderTicket | null {
    if (!this.needsRender() || !this.key) return null;
    const ticket = { revision: this.revision, serial: ++this.renderSerial, key: this.key, region: this.region };
    this.pending = ticket; this.activeId = null; return ticket;
  }

  isCurrentRender(ticket: RenderTicket): boolean {
    return ticket.revision === this.revision && ticket.key === this.key && ticket.serial === this.renderSerial;
  }

  rejectRender(ticket: RenderTicket): boolean {
    if (!this.isCurrentRender(ticket)) return false;
    this.pending = null; return true;
  }

  private record(job: PreviewJob): PreviewJob {
    const merged = mergePreviewJob(this.events.get(job.id), job);
    this.events.delete(job.id); this.events.set(job.id, merged);
    while (this.events.size > 64) this.events.delete(this.events.keys().next().value!);
    return merged;
  }

  canApply(job: PreviewJob): boolean {
    return this.hasClips && !!this.key && job.kind === 'preview' && job.id === this.activeId && job.preview_key === this.key && sameRegion(job.preview_region, this.region);
  }

  receiveResponse(ticket: RenderTicket, job: PreviewJob): PreviewJob | null {
    if (!this.isCurrentRender(ticket) || job.kind !== 'preview' || job.preview_key !== ticket.key || !sameRegion(job.preview_region, ticket.region)) return null;
    this.pending = null; this.activeId = job.id;
    const merged = this.record(job); return this.canApply(merged) ? merged : null;
  }

  receiveEvent(job: PreviewJob): PreviewJob | null {
    const merged = this.record(job); return this.canApply(merged) ? merged : null;
  }

  markReady(job: PreviewJob): boolean {
    if (!this.canApply(job) || job.status !== 'complete' || !job.path) return false;
    if (this.ready?.id !== job.id || this.ready.path !== job.path || this.ready.preview_key !== job.preview_key) this.readySerial += 1;
    this.ready = job; return true;
  }
}
