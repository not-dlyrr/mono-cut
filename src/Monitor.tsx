import { useEffect, useRef, useState, type RefObject } from 'react';
import { ChevronLeft, ChevronRight, Play, Pause, Maximize, Brackets } from 'lucide-react';
import { IconButton } from './ui';
import { fpsValue, timecode, type Rational } from './types';

interface Props { title: string; name?: string; src: string | null; fps: Rational; image?: boolean; audio?: boolean; waveform?: number[]; frame: number; duration: number; setFrame: (n: number) => void; videoRef: RefObject<HTMLVideoElement | null>; speed?: number; status?: string; footer?: React.ReactNode; onMarkIn?: () => void; onMarkOut?: () => void; onPlay: () => void; onPause: () => void; onStep: (delta: number) => void }
export default function Monitor({ title, name, src, fps, image, audio, waveform, frame, duration, setFrame, videoRef, speed = 0, status, footer, onMarkIn, onMarkOut, onPlay, onPause, onStep }: Props) {
  const [playing, setPlaying] = useState(false); const surface = useRef<HTMLDivElement>(null);
  useEffect(() => setPlaying(false), [src]);
  useEffect(() => { const v = videoRef.current; if (!v || !src || image) return; if (speed > 0) { v.playbackRate = speed; void v.play().catch(onPause); } else v.pause(); }, [speed, src, image, videoRef, onPause]);
  useEffect(() => {
    const video = videoRef.current; if (!video || !playing) return;
    let cancelled = false, callback = 0, raf = 0;
    if (!audio && typeof video.requestVideoFrameCallback === 'function') { const next: VideoFrameRequestCallback = (_, meta) => { if (cancelled) return; setFrame(Math.round(meta.mediaTime * fpsValue(fps))); callback = video.requestVideoFrameCallback(next); }; callback = video.requestVideoFrameCallback(next); }
    else { const next = () => { if (cancelled) return; setFrame(Math.round(video.currentTime * fpsValue(fps))); raf = requestAnimationFrame(next); }; raf = requestAnimationFrame(next); }
    return () => { cancelled = true; if (callback) video.cancelVideoFrameCallback(callback); if (raf) cancelAnimationFrame(raf); };
  }, [playing, src, fps, setFrame, videoRef, audio]);
  const transportActive = playing || speed !== 0, unavailable = !src || image || !!status;
  return <section data-monitor={title === 'Source' ? 'source' : 'program'} className="monitor" aria-label={`${title} monitor`}><header className="monitor-header"><span>{title}</span><span className="monitor-name" title={name}>{name || 'No media selected'}</span><IconButton label={`Fullscreen ${title.toLowerCase()} monitor`} onClick={() => void surface.current?.requestFullscreen()}><Maximize size={13} /></IconButton></header>
    <div ref={surface} className={`monitor-surface ${audio ? 'audio-source' : ''}`}>
      {src ? image ? <img src={src} alt={name || 'Source image'} /> : <><video key={src} ref={videoRef} src={src} preload="auto" playsInline onLoadedMetadata={e => { e.currentTarget.currentTime = Math.min(frame / fpsValue(fps), e.currentTarget.duration || 0); }} onTimeUpdate={e => { if (!playing) setFrame(Math.round(e.currentTarget.currentTime * fpsValue(fps))); }} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => { setPlaying(false); onPause(); }} onError={() => { setPlaying(false); onPause(); }} />{audio && <div className="audio-source-label">{waveform && waveform.length > 0 && <svg className="source-waveform" viewBox="0 0 200 60" role="img" aria-label="Source audio waveform">{waveform.filter((_, i) => i % Math.max(1, Math.floor(waveform.length / 100)) === 0).slice(0, 100).map((v, i) => <line key={i} x1={i * 2} x2={i * 2} y1={30 - Math.max(1, v * 26)} y2={30 + Math.max(1, v * 26)} stroke="currentColor" />)}</svg>}<span>{name}</span></div>}</> : <div className="monitor-placeholder"><Brackets size={25} strokeWidth={1} /><span>{status || (title === 'Source' ? 'Select media to preview' : 'Your sequence appears here')}</span></div>}
      {src && status && <div className="monitor-status glass">{status}</div>}
    </div>
    <div className="monitor-scrub"><input type="range" aria-label={`Scrub ${title.toLowerCase()} monitor`} min="0" max={Math.max(1, duration)} step="1" value={Math.min(frame, Math.max(1, duration))} disabled={unavailable} onChange={e => { onPause(); const n = Number(e.target.value); setFrame(n); if (videoRef.current) videoRef.current.currentTime = n / fpsValue(fps); }} /></div>
    <div className="monitor-controls"><span className="timecode">{timecode(frame, fps)}</span><div className="transport"><IconButton label={`Previous ${title.toLowerCase()} frame`} disabled={unavailable} onClick={() => onStep(-1)}><ChevronLeft size={16} /></IconButton><IconButton label={`${transportActive ? 'Pause' : 'Play'} ${title.toLowerCase()}`} disabled={unavailable} onClick={transportActive ? onPause : onPlay}>{transportActive ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" />}</IconButton><IconButton label={`Next ${title.toLowerCase()} frame`} disabled={unavailable} onClick={() => onStep(1)}><ChevronRight size={16} /></IconButton></div><div className="monitor-range"><button className="text-button" aria-label={`Set ${title.toLowerCase()} in point`} disabled={!src} onClick={onMarkIn}>I</button><button className="text-button" aria-label={`Set ${title.toLowerCase()} out point`} disabled={!src} onClick={onMarkOut}>O</button></div></div>
    {footer && <div className="monitor-footer">{footer}</div>}
  </section>;
}
