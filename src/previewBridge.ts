import { PreviewScheduler, type IdentityTicket, type PreviewJob } from './previewScheduler';
import { sameRegion, type PreviewRegion } from './previewRegion';

export interface PreviewSettings { height: number; useProxies: boolean; region?: PreviewRegion | null }
export interface PreviewIdentity { key: string; cached_path: string | null; region?: PreviewRegion | null; program_key?: string }
export interface PreviewIntent { session: number; revision: number; key: string }
export interface PreviewBridgePort {
  identity(settings: PreviewSettings): Promise<PreviewIdentity>;
  intent(settings: PreviewSettings, key: string, revision: number): Promise<PreviewIntent>;
  render(settings: PreviewSettings, key: string, intent: PreviewIntent): Promise<PreviewJob>;
  cancel?(revision: number): Promise<void>;
}
type IdentityDecision = NonNullable<ReturnType<PreviewScheduler['acceptIdentity']>>;
export function previewErrorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error); }
interface PublishedIntent { context: number; token: PreviewIntent }
export interface PreviewCheck { identity: PreviewIdentity; decision: IdentityDecision; intent: PreviewIntent | null }
interface IdentityRequest {
  settings: PreviewSettings;
  ticket: IdentityTicket;
  signature: string;
  promise: Promise<PreviewCheck | null>;
  resolve(value: PreviewCheck | null): void;
  reject(error: unknown): void;
}

/** The same native intent publication precedes both ready-cache reuse and rendering. */
export class PreviewBridge {
  private published: PublishedIntent | null = null;
  private identityActive: IdentityRequest | null = null;
  private identityPending: IdentityRequest | null = null;
  private operation = 0;

  constructor(private scheduler: PreviewScheduler, private port: PreviewBridgePort, private alive: () => boolean = () => true) {}

  check(settings: PreviewSettings): Promise<PreviewCheck | null> {
    if (!this.alive()) return Promise.resolve(null);
    const region = settings.region;
    const signature = `${this.scheduler.contextRevision}:${settings.height}:${settings.useProxies}:${region ? `${region.start_frame}:${region.end_frame}` : 'full'}`;
    // Metadata checks arriving during the same native sample share its validated result.
    for (const request of [this.identityActive, this.identityPending]) {
      if (request?.signature === signature && this.scheduler.isCurrentIdentity(request.ticket)) return request.promise;
    }
    const ticket = this.scheduler.beginIdentity();
    this.operation += 1;
    let resolve!: IdentityRequest['resolve'], reject!: IdentityRequest['reject'];
    const promise = new Promise<PreviewCheck | null>((yes, no) => { resolve = yes; reject = no; });
    const request: IdentityRequest = { settings: { ...settings, region: region ? { ...region } : region }, ticket, signature, promise, resolve, reject };
    if (this.identityActive) {
      this.identityPending?.resolve(null);
      this.identityPending = request;
    } else void this.sampleIdentity(request);
    return promise;
  }

  discardPendingChecks(): void { this.identityPending?.resolve(null); this.identityPending = null; }

  cancel(): Promise<void> {
    this.discardPendingChecks(); this.published = null;
    const revision = this.scheduler.cancelIntent();
    const operation = ++this.operation;
    return (this.port.cancel?.(revision) ?? Promise.resolve()).catch(error => {
      // A later check or cancellation owns the intent even before its native
      // reply arrives. An older Stop cannot fail that newer operation.
      if (this.alive() && operation === this.operation && (!this.published || this.published.token.revision <= revision)) throw error;
    });
  }

  private releaseIdentity(request: IdentityRequest): void {
    if (this.identityActive !== request) return;
    this.identityActive = null;
    const next = this.identityPending; this.identityPending = null;
    if (next) {
      if (this.alive() && this.scheduler.isCurrentIdentity(next.ticket)) void this.sampleIdentity(next);
      else next.resolve(null);
    }
  }

  private async sampleIdentity(request: IdentityRequest): Promise<void> {
    this.identityActive = request;
    try {
      const identity = await this.port.identity(request.settings);
      // Bound the native identity RPC, not its reply-to-intent continuation: a held old
      // intent reply must still allow a newer cached Undo intent to supersede it.
      this.releaseIdentity(request);
      request.resolve(await this.publishIdentity(request, identity));
    } catch (error) {
      this.releaseIdentity(request);
      if (this.alive() && this.scheduler.isCurrentIdentity(request.ticket)) request.reject(error);
      else request.resolve(null);
    }
  }

  private async publishIdentity(request: IdentityRequest, identity: PreviewIdentity): Promise<PreviewCheck | null> {
    const { settings, ticket } = request;
    try {
      if (!this.alive()) return null;
      if (!this.scheduler.isCurrentIdentity(ticket)) return null;
      if (!sameRegion(identity.region, settings.region)) throw new Error('The media engine returned preview coverage for a different sequence region. Try rendering again.');
      if (settings.region && !identity.program_key) throw new Error('The media engine did not validate the program identity for this preview region. Try rendering again.');
      const decision = this.scheduler.acceptIdentity(ticket, identity.key, identity.cached_path);
      if (!decision) return null;
      if (decision.recheck) return { identity, decision, intent: null };
      const token = await this.port.intent(settings, identity.key, ticket.serial);
      if (!this.alive() || !this.scheduler.isCurrentIdentity(ticket)) return null;
      if (token.key !== identity.key) throw new Error('The media engine accepted a different preview intent. Try rendering again.');
      this.published = { context: ticket.revision, token };
      return { identity, decision, intent: token };
    } catch (error) { if (this.alive() && this.scheduler.isCurrentIdentity(ticket)) throw error; return null; }
  }

  async render(settings: PreviewSettings): Promise<PreviewJob | null> {
    const intent = this.published;
    if (!this.alive() || !intent || !this.scheduler.isCurrentContext(intent.context) || this.scheduler.expectedKey !== intent.token.key) return null;
    const request = this.scheduler.beginRender(); if (!request) return null;
    try {
      const job = await this.port.render(settings, intent.token.key, intent.token);
      if (!this.alive()) return null;
      const result = this.scheduler.receiveResponse(request, job);
      if (!result && this.scheduler.isCurrentRender(request)) {
        throw new Error('The media engine returned a preview for a different render identity. Try rendering again.');
      }
      return result;
    } catch (error) { if (this.alive() && this.scheduler.rejectRender(request)) throw error; return null; }
  }
}
