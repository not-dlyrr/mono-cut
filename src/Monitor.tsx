import { useEffect, useRef, useState, type RefObject } from 'react';
import { ChevronLeft, ChevronRight, Play, Pause, Maximize, Brackets } from 'lucide-react';
import { IconButton } from './ui';
import { timecode, type Rational } from './types';
import { globalPreviewFrame, localPreviewTime, type PreviewRegion } from './previewRegion';
import { monitorClockActive, monitorEventAllowed, settleMonitorPlay, type MonitorEvent } from './monitorLifecycle';
import ProgramMedia from './ProgramMedia';
import type { ProgramMediaAsset, ProgramMediaOwnership } from './programMediaSlots';

interface Props { title: string; active?: boolean; onActivate?: () => void; markable?: boolean; name?: string; src: string | null; fps: Rational; image?: boolean; audio?: boolean; waveform?: number[]; frame: number; duration: number; setFrame: (n: number) => void; videoRef: RefObject<HTMLVideoElement | null>; speed?: number; status?: string; region?: PreviewRegion | null; playable?: boolean; onSeek?: (frame: number) => void; onBoundary?: () => void; programAsset?: ProgramMediaAsset | null; successor?: ProgramMediaAsset | null; onSuccessorReady?: (id: string) => void; onSuccessorFailed?: (id: string, error: unknown) => void; onProgramAssets?: (assets: ProgramMediaOwnership) => void; footer?: React.ReactNode; onMarkIn?: () => void; onMarkOut?: () => void; onPlay: () => void; onPause: () => void; onStep: (delta: number) => void }
export default function Monitor({ title, active = false, onActivate, markable, name, src, fps, image, audio, waveform, frame, duration, setFrame, videoRef, speed = 0, status, region, playable = true, onSeek, onBoundary, programAsset, successor, onSuccessorReady, onSuccessorFailed, onProgramAssets, footer, onMarkIn, onMarkOut, onPlay, onPause, onStep }: Props) {
  const program = programAsset !== undefined;
  const [playing, setPlaying] = useState(false); const surface = useRef<HTMLDivElement>(null), activeSrc = useRef(src), activeSpeed = useRef(speed), generationStartedAt = useRef(performance.now());
  activeSpeed.current = speed;
  if (activeSrc.current !== src) { activeSrc.current = src; generationStartedAt.current = performance.now(); }
  const normalizedSrc = src ? new URL(src, document.baseURI).href : null;
  function allowed(kind: MonitorEvent, event: React.SyntheticEvent<HTMLVideoElement>) {
    return monitorEventAllowed(kind, event.currentTarget, normalizedSrc, event.currentTarget === videoRef.current && activeSrc.current === src, event.timeStamp, generationStartedAt.current, performance.timeOrigin, fps, region);
  }
  useEffect(() => setPlaying(false), [src]);
  useEffect(() => {
    const v = videoRef.current; if (program || !v || !src || image) { setPlaying(false); return; }
    const generation = generationStartedAt.current; let active = true;
    if (speed > 0) {
      v.playbackRate = speed;
      void settleMonitorPlay(v.play(), () => active && videoRef.current === v && activeSrc.current === src && generationStartedAt.current === generation, onPause, () => { if (activeSpeed.current > 0 && !v.paused) setPlaying(true); });
    } else { setPlaying(false); v.pause(); }
    return () => { active = false; };
  }, [speed, src, image, videoRef, onPause, program]);
  useEffect(() => {
    const video = videoRef.current; if (program || !video || !monitorClockActive(playing, speed, src, activeSrc.current, video === videoRef.current)) return;
    let cancelled = false, callback = 0, raf = 0;
    if (!audio && typeof video.requestVideoFrameCallback === 'function') { const next: VideoFrameRequestCallback = (_, meta) => { if (cancelled || video.paused || !monitorClockActive(true, activeSpeed.current, src, activeSrc.current, videoRef.current === video)) return; setFrame(globalPreviewFrame(meta.mediaTime, fps, region)); callback = video.requestVideoFrameCallback(next); }; callback = video.requestVideoFrameCallback(next); }
    else { const next = () => { if (cancelled || video.paused || !monitorClockActive(true, activeSpeed.current, src, activeSrc.current, videoRef.current === video)) return; setFrame(globalPreviewFrame(video.currentTime, fps, region)); raf = requestAnimationFrame(next); }; raf = requestAnimationFrame(next); }
    return () => { cancelled = true; if (callback) video.cancelVideoFrameCallback(callback); if (raf) cancelAnimationFrame(raf); };
  }, [playing, speed, src, fps, setFrame, videoRef, audio, region, program]);
  const transportActive = speed !== 0, unavailable = !src || image || !playable || (!!status && !region);
  function act(callback?: () => void) { onActivate?.(); callback?.(); }
  return <section data-monitor={title === 'Source' ? 'source' : 'program'} className={`monitor ${active ? 'monitor-active' : ''}`} role="region" tabIndex={0} aria-label={`${title} monitor${active ? ', keyboard target' : ''}`} onFocusCapture={() => onActivate?.()} onPointerDownCapture={e => { onActivate?.(); if (e.target instanceof Element && !e.target.closest('button,input,select,textarea,a[href],[contenteditable],[role="button"],[role="tab"]')) e.currentTarget.focus({ preventScroll: true }); }}><header className="monitor-header"><span>{title}</span><span className="monitor-target" aria-hidden={!active}>Keyboard</span><span className="monitor-name" title={name}>{name || 'No media selected'}</span><IconButton label={`Fullscreen ${title.toLowerCase()} monitor`} onClick={() => act(() => { void surface.current?.requestFullscreen(); })}><Maximize size={13} /></IconButton></header>
    <div ref={surface} className={`monitor-surface ${audio ? 'audio-source' : ''}`}>
      {program && <ProgramMedia current={programAsset ?? null} successor={successor ?? null} frame={frame} fps={fps} speed={speed} videoRef={videoRef} setFrame={setFrame} onPause={onPause} onBoundary={onBoundary!} onReady={onSuccessorReady!} onFailed={onSuccessorFailed!} onAssets={onProgramAssets!} />}
      {src ? program ? null : image ? <img src={src} alt={name || 'Source image'} /> : <><video key={src} ref={videoRef} src={src} preload="auto" playsInline onLoadedMetadata={e => { if (!allowed('metadata', e)) return; e.currentTarget.currentTime = Math.min(localPreviewTime(frame, fps, region), e.currentTarget.duration || 0); }} onTimeUpdate={e => { if (e.currentTarget === videoRef.current && activeSrc.current === src && !onSeek && !playing && (!region || playable)) setFrame(globalPreviewFrame(e.currentTarget.currentTime, fps, region)); }} onPlay={e => { if (speed > 0 && allowed('play', e)) setPlaying(true); }} onPlaying={e => { if (speed > 0 && allowed('playing', e)) setPlaying(true); }} onPause={e => { if (allowed('pause', e)) setPlaying(false); }} onEnded={e => { if (!allowed('ended', e)) return; setPlaying(false); if (onBoundary) onBoundary(); else onPause(); }} onError={e => { if (!allowed('error', e)) return; setPlaying(false); onPause(); }} />{audio && <div className="audio-source-label">{waveform && waveform.length > 0 && <svg className="source-waveform" viewBox="0 0 200 60" role="img" aria-label="Source audio waveform">{waveform.filter((_, i) => i % Math.max(1, Math.floor(waveform.length / 100)) === 0).slice(0, 100).map((v, i) => <line key={i} x1={i * 2} x2={i * 2} y1={30 - Math.max(1, v * 26)} y2={30 + Math.max(1, v * 26)} stroke="currentColor" />)}</svg>}<span>{name}</span></div>}</> : <div className="monitor-placeholder"><Brackets size={25} strokeWidth={1} /><span>{status || (title === 'Source' ? 'Select media to preview' : 'Your sequence appears here')}</span></div>}
      {src && status && <div className="monitor-status glass">{status}</div>}
    </div>
    <div className="monitor-scrub"><input type="range" aria-label={`Scrub ${title.toLowerCase()} monitor`} min="0" max={Math.max(1, duration)} step="1" value={Math.min(frame, Math.max(1, duration))} disabled={image || (!src && !onSeek) || (!onSeek && !playable)} onChange={e => { act(onPause); const n = Number(e.target.value); if (onSeek) onSeek(n); else { setFrame(n); if (videoRef.current) videoRef.current.currentTime = localPreviewTime(n, fps, region); } }} /></div>
    <div className="monitor-controls"><span className="timecode">{timecode(frame, fps)}</span><div className="transport"><IconButton label={`Previous ${title.toLowerCase()} frame`} disabled={image || (!src && !onSeek) || (!onSeek && !playable)} onClick={() => act(() => onStep(-1))}><ChevronLeft size={16} /></IconButton><IconButton label={`${transportActive ? 'Pause' : 'Play'} ${title.toLowerCase()}`} disabled={unavailable} onClick={() => act(transportActive ? onPause : onPlay)}>{transportActive ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" />}</IconButton><IconButton label={`Next ${title.toLowerCase()} frame`} disabled={image || (!src && !onSeek) || (!onSeek && !playable)} onClick={() => act(() => onStep(1))}><ChevronRight size={16} /></IconButton></div><div className="monitor-range"><button className="text-button" aria-label={`Set ${title.toLowerCase()} in point`} disabled={!(markable ?? !!src)} onClick={() => act(onMarkIn)}>I</button><button className="text-button" aria-label={`Set ${title.toLowerCase()} out point`} disabled={!(markable ?? !!src)} onClick={() => act(onMarkOut)}>O</button></div></div>
    {footer && <div className="monitor-footer">{footer}</div>}
  </section>;
}
