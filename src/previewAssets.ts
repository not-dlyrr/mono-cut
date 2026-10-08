export interface PlaybackAssets { previewPath: string | null; sourcePath: string | null }

/** Serialize native pin updates and omit queued snapshots already superseded by the UI. */
export class PlaybackAssetQueue {
  private latest: PlaybackAssets = { previewPath: null, sourcePath: null };
  private revision = 0;
  private tail: Promise<void> = Promise.resolve();

  constructor(private write: (assets: PlaybackAssets) => Promise<void>, private report: (error: unknown) => void = () => {}) {}

  update(assets: PlaybackAssets, force = false): Promise<void> {
    if (!force && assets.previewPath === this.latest.previewPath && assets.sourcePath === this.latest.sourcePath) return this.tail;
    this.latest = { ...assets }; const revision = ++this.revision, snapshot = { ...assets };
    this.tail = this.tail.then(async () => { if (revision === this.revision) await this.write(snapshot); }).catch(error => { if (revision === this.revision) this.report(error); });
    return this.tail;
  }
}

/** Order source intent invalidation without making later selections wait for slow preparation. */
export class SourceIntentQueue {
  private tail: Promise<void> = Promise.resolve();

  constructor(private invalidateNative: () => Promise<void>) {}

  invalidate(): Promise<void> {
    const current = this.tail.then(() => this.invalidateNative());
    this.tail = current.catch(() => {});
    return current;
  }
}
