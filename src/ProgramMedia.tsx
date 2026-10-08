import { useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { ProgramMediaSlots, commitProgramMedia, detachProgramMedia, type ProgramMediaAsset, type ProgramMediaOwnership } from './programMediaSlots';
import { globalPreviewFrame, localPreviewTime } from './previewRegion';
import { monitorEventAllowed, settleMonitorPlay, type MonitorEvent } from './monitorLifecycle';
import type { Rational } from './types';

interface Props {
  current: ProgramMediaAsset | null; successor: ProgramMediaAsset | null; frame: number; fps: Rational; speed: number;
  videoRef: RefObject<HTMLVideoElement | null>; setFrame(frame: number): void; onPause(): void; onBoundary(): void;
  onReady(id: string): void; onFailed(id: string, error: unknown): void; onAssets(assets: ProgramMediaOwnership): void;
}
export default function ProgramMedia(props: Props) {
  const model = useRef(new ProgramMediaSlots()), nodes = useRef<[HTMLVideoElement | null, HTMLVideoElement | null]>([null, null]);
  const committedGenerations = useRef<[number | null, number | null]>([null, null]);
  const live = useRef(props), [, refresh] = useState(0), playRevision = useRef(0);
  live.current = props; model.current.reconcile(props.current, props.successor, performance.now());
  const owner = model.current.ownership;
  const allowed = (index: number, generation: number, kind: MonitorEvent, event: React.SyntheticEvent<HTMLVideoElement>) => {
    const slot = model.current.slots[index];
    return !!slot && model.current.current === index && model.current.owns(index, generation) && event.currentTarget === live.current.videoRef.current && monitorEventAllowed(kind, event.currentTarget, new URL(slot.asset.src, document.baseURI).href, true, event.timeStamp, slot.startedAt, performance.timeOrigin, live.current.fps, slot.asset.region);
  };
  // Detach and silence the previous node before publishing the new native consuming pins.
  useLayoutEffect(() => {
    commitProgramMedia(model.current, nodes.current, committedGenerations.current);
    const current = model.current.current;
    props.videoRef.current = current === null ? null : nodes.current[current];
    if (props.videoRef.current) props.videoRef.current.muted = false;
    props.onAssets(model.current.snapshot());
  });
  useLayoutEffect(() => {
    const index = model.current.current, node = index === null ? null : nodes.current[index], revision = ++playRevision.current;
    if (!node || !props.current) return;
    const current = () => revision === playRevision.current && owner === model.current.ownership && node === live.current.videoRef.current && live.current.speed > 0;
    if (props.speed > 0 && props.current.playable) {
      node.playbackRate = props.speed;
      void settleMonitorPlay(node.play(), current, props.onPause, () => refresh(value => value + 1));
    } else node.pause();
    return () => { if (playRevision.current === revision) playRevision.current += 1; };
  }, [owner, props.speed, props.current?.playable, props.onPause]);
  useLayoutEffect(() => {
    const index = model.current.current, slot = index === null ? null : model.current.slots[index], node = index === null ? null : nodes.current[index];
    if (!slot || !node || props.speed <= 0) return;
    let cancelled = false, callback = 0, raf = 0;
    const current = () => !cancelled && owner === model.current.ownership && node === live.current.videoRef.current && live.current.speed > 0;
    const tick = (seconds: number) => { if (current() && !node.paused) live.current.setFrame(globalPreviewFrame(seconds, live.current.fps, slot.asset.region)); };
    if (typeof node.requestVideoFrameCallback === 'function') { const next: VideoFrameRequestCallback = (_, meta) => { if (!current()) return; tick(meta.mediaTime); callback = node.requestVideoFrameCallback(next); }; callback = node.requestVideoFrameCallback(next); }
    else { const next = () => { if (!current()) return; tick(node.currentTime); raf = requestAnimationFrame(next); }; raf = requestAnimationFrame(next); }
    return () => { cancelled = true; if (callback) node.cancelVideoFrameCallback(callback); if (raf) cancelAnimationFrame(raf); };
  }, [owner, props.speed, props.fps]);
  useLayoutEffect(() => () => { playRevision.current += 1; detachProgramMedia(nodes.current, committedGenerations.current); live.current.videoRef.current = null; live.current.onAssets({ current: null, successor: null }); }, []);
  function ready(index: number, generation: number, event: React.SyntheticEvent<HTMLVideoElement>) {
    const slot = model.current.slots[index]; if (!slot) return;
    if (model.current.admit(index, generation, event.currentTarget, new URL(slot.asset.src, document.baseURI).href, event.timeStamp, performance.timeOrigin, live.current.fps)) live.current.onReady(slot.asset.id);
  }
  return <>{model.current.slots.map((slot, index) => <video key={index} ref={node => { nodes.current[index] = node; }} src={slot?.asset.src} preload="auto" playsInline aria-hidden={index !== model.current.current} tabIndex={-1} style={{ visibility: index === model.current.current ? 'visible' : 'hidden', pointerEvents: 'none' }}
    onLoadedMetadata={event => { if (!slot || !model.current.owns(index, slot.generation)) return; if (model.current.successor === index) { event.currentTarget.currentTime = 0; ready(index, slot.generation, event); } else if (allowed(index, slot.generation, 'metadata', event)) event.currentTarget.currentTime = Math.min(localPreviewTime(live.current.frame, live.current.fps, slot.asset.region), event.currentTarget.duration); }}
    onCanPlay={event => { if (slot) ready(index, slot.generation, event); }}
    onPlaying={event => { if (slot && allowed(index, slot.generation, 'playing', event)) refresh(value => value + 1); }}
    onEnded={event => { if (slot && allowed(index, slot.generation, 'ended', event)) live.current.onBoundary(); }}
    onError={event => { if (!slot || !model.current.owns(index, slot.generation)) return; if (model.current.successor === index && monitorEventAllowed('error', event.currentTarget, new URL(slot.asset.src, document.baseURI).href, true, event.timeStamp, slot.startedAt, performance.timeOrigin, live.current.fps, slot.asset.region)) live.current.onFailed(slot.asset.id, 'The next preview could not load. Render preview to retry.'); else if (allowed(index, slot.generation, 'error', event)) live.current.onPause(); }}
  />)}</>;
}
