import { PreviewScheduler, type PreviewJob } from './previewScheduler';

export interface PreviewSettings { height: number; useProxies: boolean }
export interface PreviewIdentity { key: string; cached_path: string | null }
export interface PreviewIntent { session: number; revision: number; key: string }
export interface PreviewBridgePort {
  identity(settings: PreviewSettings): Promise<PreviewIdentity>;
  intent(settings: PreviewSettings, key: string, revision: number): Promise<PreviewIntent>;
  render(settings: PreviewSettings, key: string, intent: PreviewIntent): Promise<PreviewJob>;
}
type IdentityDecision = NonNullable<ReturnType<PreviewScheduler['acceptIdentity']>>;
interface PublishedIntent { context: number; token: PreviewIntent }
export interface PreviewCheck { identity: PreviewIdentity; decision: IdentityDecision; intent: PreviewIntent | null }

/** The same native intent publication precedes both ready-cache reuse and rendering. */
export class PreviewBridge {
  private published: PublishedIntent | null = null;

  constructor(private scheduler: PreviewScheduler, private port: PreviewBridgePort, private alive: () => boolean = () => true) {}

  async check(settings: PreviewSettings): Promise<PreviewCheck | null> {
    const ticket = this.scheduler.beginIdentity();
    try {
      const identity = await this.port.identity(settings);
      if (!this.alive()) return null;
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
