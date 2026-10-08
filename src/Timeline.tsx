import { useEffect, useMemo, useRef, useState, type PointerEvent } from 'react';
import { Magnet, Minus, Plus, Scissors, Trash2, Copy, Link2, Unlink, Volume2, VolumeX, Eye, EyeOff, LockKeyhole, UnlockKeyhole, Flag, Type, MousePointer2, ArrowLeftRight, X } from 'lucide-react';
import { IconButton, Menu } from './ui';
import { endFrame, fpsValue, timecode, type Clip, type Edit, type Project } from './types';
import { displayedClip, dragClipIds, timelineWaveform } from './timelineWaveform';

interface Props { project: Project; frame: number; setFrame: (frame: number) => void; selected: string[]; setSelected: (ids: string[]) => void; edit: Edit; addTitle: () => void; addMarker: () => void; activeTrack: string | null; setActiveTrack: (id: string) => void; snap: boolean; setSnap: (v: boolean) => void; easy?: boolean }
interface Drag { id: string; mode: 'move' | 'in' | 'out' | 'slip'; origin: number; delta: number; track: string; ids: string[] }
const HEADER = 148, ROW = 62;
export default function Timeline({ project, frame, setFrame, selected, setSelected, edit, addTitle, addMarker, activeTrack, setActiveTrack, snap, setSnap, easy = false }: Props) {
  const viewport = useRef<HTMLDivElement>(null), [zoom, setZoom] = useState(1.8), [scroll, setScroll] = useState({ left: 0, width: 1000 }), [drag, setDrag] = useState<Drag | null>(null), [slipMode, setSlipMode] = useState(false);
  const dragRef = useRef<Drag | null>(null); const scale = zoom * 30 / fpsValue(project.fps), total = Math.max(endFrame(project) + Math.round(fpsValue(project.fps) * 10), Math.round((scroll.width - HEADER) / scale));
  const width = Math.max(0, total * scale); const tracks = [...project.tracks.filter(t => t.kind === 'video').reverse(), ...project.tracks.filter(t => t.kind === 'audio')];
  useEffect(() => { const v = viewport.current; if (!v) return; const ro = new ResizeObserver(() => setScroll(s => ({ ...s, width: v.clientWidth }))); ro.observe(v); return () => ro.disconnect(); }, []);
  const step = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800].find(s => s * scale > 92) || 1800;
  const first = Math.floor(scroll.left / scale / step) * step, last = Math.ceil((scroll.left + scroll.width) / scale);
  const ticks = []; for (let n = first; n < last; n += step) ticks.push(n);
  const draggingIds = drag ? dragClipIds(project, drag) : new Set<string>();
  const visible = project.clips.filter(c => { const p = displayedClip(c, draggingIds.has(c.id) ? drag : null); return draggingIds.has(c.id) || ((p.start + p.duration) * scale >= scroll.left - 100 && p.start * scale <= scroll.left + scroll.width + 100); });
  const waveforms = useMemo(() => new Map(visible.map(c => {
    const media = project.media.find(m => m.id === c.media_id), p = displayedClip(c, draggingIds.has(c.id) ? drag : null);
    return [c.id, media ? timelineWaveform(media, c, project.fps, p.duration, scale, scroll.left - p.start * scale - 100, scroll.left + scroll.width - HEADER - p.start * scale + 100, p.sourceFrameOffset) : null];
  })), [project, drag, scale, scroll.left, scroll.width]);
  function seek(e: PointerEvent) { const rect = e.currentTarget.getBoundingClientRect(); setFrame(Math.max(0, Math.round((e.clientX - rect.left) / scale))); }
  function snapped(value: number, ignoreIds: string[], duration = 0) {
    if (!snap) return value; const anchors = [0, frame, ...project.markers.map(m => m.frame), ...project.clips.filter(c => !ignoreIds.includes(c.id)).flatMap(c => [c.start, c.start + c.duration])];
    let best = value, threshold = 8 / scale;
    for (const anchor of anchors) for (const edge of [value, value + duration]) { const diff = anchor - edge; if (Math.abs(diff) < threshold) { best = value + diff; threshold = Math.abs(diff); } }
    return Math.round(best);
  }
  function begin(e: PointerEvent, clip: Clip, mode: Drag['mode']) {
    e.stopPropagation(); if (project.tracks.find(t => t.id === clip.track_id)?.locked) return;
    const ids = selected.includes(clip.id) ? selected : e.shiftKey ? [...selected, clip.id] : [clip.id]; setSelected(ids); setActiveTrack(clip.track_id);
    const d = { id: clip.id, mode, origin: e.clientX, delta: 0, track: clip.track_id, ids }; dragRef.current = d; setDrag(d); e.currentTarget.setPointerCapture(e.pointerId);
  }
  function update(e: PointerEvent, clip: Clip) {
    const d = dragRef.current; if (!d || d.id !== clip.id || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    let delta = Math.round((e.clientX - d.origin) / scale);
    if (d.mode === 'move') delta = snapped(Math.max(0, clip.start + delta), d.ids, clip.duration) - clip.start;
    if (d.mode === 'in') delta = snapped(clip.start + delta, d.ids) - clip.start;
    if (d.mode === 'out') delta = snapped(clip.start + clip.duration + delta, d.ids) - clip.start - clip.duration;
    const viewportRect = viewport.current?.getBoundingClientRect();
    const rowIndex = viewportRect ? Math.floor((e.clientY - viewportRect.top + (viewport.current?.scrollTop || 0) - 36) / ROW) : -1;
    const target = tracks[rowIndex];
    const track = target && !target.locked && target.kind === project.tracks.find(t => t.id === clip.track_id)?.kind ? target.id : d.track;
    const next = { ...d, delta, track }; dragRef.current = next; setDrag(next);
  }
  function finish(e: PointerEvent, clip: Clip) {
    const d = dragRef.current; if (!d || d.id !== clip.id) return; e.currentTarget.releasePointerCapture(e.pointerId); dragRef.current = null; setDrag(null);
    if (d.mode === 'move' && (d.delta || d.track !== clip.track_id)) void edit({ type: 'move', ids: [d.id, ...d.ids.filter(id => id !== d.id)], delta: d.delta, ...(d.track !== clip.track_id ? { track_id: d.track } : {}) });
    if (d.mode === 'in' && d.delta) void edit({ type: 'trim', id: d.id, edge: 'in', frame: clip.start + d.delta });
    if (d.mode === 'out' && d.delta) void edit({ type: 'trim', id: d.id, edge: 'out', frame: clip.start + clip.duration + d.delta });
    if (d.mode === 'slip' && d.delta) void edit({ type: 'slip', id: d.id, delta: d.delta });
  }
  function drawClip(c: Clip, trackId: string) {
    const media = project.media.find(m => m.id === c.media_id), d = drag && draggingIds.has(c.id) ? drag : null;
    const { start, duration } = displayedClip(c, d);
    if (c.track_id !== trackId) return null;
    const anchorTrack = d && project.clips.find(clip => clip.id === d.id)?.track_id;
    const translate = d?.mode === 'move' && c.track_id === anchorTrack ? (tracks.findIndex(t => t.id === d.track) - tracks.findIndex(t => t.id === c.track_id)) * ROW : 0;
    const wave = waveforms.get(c.id);
    return <div key={c.id} className={`timeline-clip ${selected.includes(c.id) ? 'selected' : ''} ${media?.kind === 'audio' ? 'audio' : ''} ${c.title !== null ? 'title-clip' : ''} ${media?.missing ? 'missing' : ''}`} style={{ left: start * scale, width: Math.max(5, duration * scale), transform: `translateY(${translate}px)`, zIndex: d ? 10 : undefined }} tabIndex={0} role="button" aria-label={`${c.name}, starts ${timecode(c.start, project.fps)}, ${c.duration} frames`}
      onFocus={() => setActiveTrack(c.track_id)} onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setSelected(e.shiftKey ? [...selected.filter(id => id !== c.id), c.id] : [c.id]); } }}
      onPointerDown={e => begin(e, c, slipMode ? 'slip' : 'move')} onPointerMove={e => update(e, c)} onPointerUp={e => finish(e, c)} onPointerCancel={() => { dragRef.current = null; setDrag(null); }}>
      <div className="trim-handle in" aria-hidden="true" onPointerDown={e => begin(e, c, 'in')} onPointerMove={e => update(e, c)} onPointerUp={e => finish(e, c)} />
      <span className="clip-name">{c.title !== null && <Type size={12} />}{c.linked_id && <Link2 size={11} />}{c.name}</span>
      {wave && wave.width > 0 && <svg className="waveform" style={{ left: wave.left, width: wave.width }} viewBox={`0 0 ${wave.width} 24`} preserveAspectRatio="none" aria-hidden="true">{wave.points.map((p, i) => <line key={i} x1={p.x} x2={p.x} y1={12 - Math.max(1, p.peak * 10)} y2={12 + Math.max(1, p.peak * 10)} />)}</svg>}
      {c.fade_in > 0 && <div className="fade-shape in" style={{ width: Math.min(duration * scale / 2, c.fade_in * scale) }} />}{c.fade_out > 0 && <div className="fade-shape out" style={{ width: Math.min(duration * scale / 2, c.fade_out * scale) }} />}
      <div className="trim-handle out" aria-hidden="true" onPointerDown={e => begin(e, c, 'out')} onPointerMove={e => update(e, c)} onPointerUp={e => finish(e, c)} />
    </div>;
  }
  return <section data-tour="timeline" className="timeline-panel" aria-label="Multitrack timeline">
    <div className="timeline-toolbar"><div className="timeline-title"><span>Timeline</span><span className="secondary">{project.name}</span></div><div className="tool-group">
      {!easy && <><IconButton label="Selection tool" active={!slipMode} onClick={() => setSlipMode(false)}><MousePointer2 size={16} /></IconButton><IconButton label="Slip tool" active={slipMode} onClick={() => setSlipMode(true)}><ArrowLeftRight size={16} /></IconButton><span className="tool-separator" /></>}
      <IconButton label="Split selected clips at playhead" disabled={!selected.length} onClick={() => void edit({ type: 'split', ids: selected, frame })}><Scissors size={16} />{easy && <span>Split</span>}</IconButton>{easy && <IconButton label="Delete selected clips" disabled={!selected.length} onClick={() => void edit({ type: 'delete', ids: selected, ripple: false })}><Trash2 size={15} /><span>Delete</span></IconButton>}{!easy && <IconButton label="Duplicate selected clips" disabled={!selected.length} onClick={() => void edit({ type: 'duplicate', ids: selected })}><Copy size={16} /></IconButton>}
      <Menu label={easy ? 'More' : 'Edit'}><button data-close-menu disabled={!selected.length} onClick={() => void edit({ type: 'delete', ids: selected, ripple: false })}><Trash2 size={15} />Delete</button><button data-close-menu disabled={!selected.length} onClick={() => void edit({ type: 'delete', ids: selected, ripple: true })}>Ripple delete</button><button data-close-menu disabled={selected.length < 2} onClick={() => void edit({ type: 'link', ids: selected })}><Link2 size={15} />Link clips</button><button data-close-menu disabled={!selected.length} onClick={() => void edit({ type: 'unlink', ids: selected })}><Unlink size={15} />Unlink clips</button><div className="menu-divider" /><button data-close-menu onClick={addTitle}><Type size={15} />Add title</button><button data-close-menu onClick={addMarker}><Flag size={15} />Add marker</button><button data-close-menu onClick={() => void edit({ type: 'add_track', kind: 'video', name: `Video ${project.tracks.filter(t => t.kind === 'video').length + 1}` })}>Add video track</button><button data-close-menu onClick={() => void edit({ type: 'add_track', kind: 'audio', name: `Audio ${project.tracks.filter(t => t.kind === 'audio').length + 1}` })}>Add audio track</button></Menu>
      {!easy && <Menu label="Markers"><button data-close-menu onClick={addMarker}><Flag size={15} />Add marker</button>{project.markers.map(m => <div className="marker-menu-row" key={m.id}><button data-close-menu onClick={() => setFrame(m.frame)}>{m.name}<span>{timecode(m.frame, project.fps)}</span></button><IconButton label={`Delete marker ${m.name}`} onClick={() => void edit({ type: 'remove_marker', id: m.id })}><X size={13} /></IconButton></div>)}</Menu>}<IconButton label="Snap to clip edges, markers and playhead" active={snap} onClick={() => setSnap(!snap)}><Magnet size={16} />{easy && <span>Snap</span>}</IconButton></div>
      <div className="timeline-zoom"><IconButton label="Zoom out timeline" onClick={() => setZoom(v => Math.max(.2, v / 1.4))}><Minus size={14} /></IconButton><input aria-label="Timeline zoom" type="range" min="0.2" max="12" step="0.1" value={zoom} onChange={e => setZoom(Number(e.target.value))} /><IconButton label="Zoom in timeline" onClick={() => setZoom(v => Math.min(12, v * 1.4))}><Plus size={14} /></IconButton><button className="text-button" onClick={() => setZoom(Math.max(.2, (scroll.width - HEADER - 30) / Math.max(1, endFrame(project)) * fpsValue(project.fps) / 30))}>Fit</button></div>
    </div>
    <div ref={viewport} className="timeline-viewport" onScroll={e => setScroll(s => ({ ...s, left: e.currentTarget.scrollLeft }))}>
      <div className="timeline-grid" style={{ width: width + HEADER, minHeight: tracks.length * ROW + 36 }}>
        <div className="ruler-header">{timecode(frame, project.fps)}</div><div className="timeline-ruler" style={{ left: HEADER, width }} onPointerDown={e => { seek(e); e.currentTarget.setPointerCapture(e.pointerId); }} onPointerMove={e => { if (e.currentTarget.hasPointerCapture(e.pointerId)) seek(e); }} onPointerUp={e => e.currentTarget.releasePointerCapture(e.pointerId)}>
          {ticks.map(n => <div className="ruler-tick" key={n} style={{ left: n * scale }}><span>{timecode(n, project.fps)}</span></div>)}
          {project.markers.map(m => <button className="timeline-marker" key={m.id} style={{ left: m.frame * scale }} title={m.name} aria-label={`Go to marker: ${m.name}`} onPointerDown={e => e.stopPropagation()} onClick={() => setFrame(m.frame)}><Flag size={12} fill="currentColor" /></button>)}
        </div>
        {tracks.map((track, index) => <div className={`timeline-track ${activeTrack === track.id ? 'active-track' : ''}`} key={track.id} data-track-id={track.id} style={{ top: 36 + index * ROW, height: ROW }}>
          <div className="track-header" onClick={() => setActiveTrack(track.id)}><button className="track-name" onClick={() => setActiveTrack(track.id)}><span>{track.kind === 'video' ? 'V' : 'A'}{project.tracks.filter(t => t.kind === track.kind).findIndex(t => t.id === track.id) + 1}</span>{track.name}</button><div className="track-controls">
            {track.kind === 'video' && <IconButton label={`${track.hidden ? 'Show' : 'Hide'} ${track.name}`} active={track.hidden} onClick={() => void edit({ type: 'update_track', id: track.id, patch: { hidden: !track.hidden } })}>{track.hidden ? <EyeOff size={13} /> : <Eye size={13} />}</IconButton>}
            <IconButton label={`${track.muted ? 'Unmute' : 'Mute'} ${track.name}`} active={track.muted} onClick={() => void edit({ type: 'update_track', id: track.id, patch: { muted: !track.muted } })}>{track.muted ? <VolumeX size={13} /> : <Volume2 size={13} />}</IconButton><IconButton label={`${track.locked ? 'Unlock' : 'Lock'} ${track.name}`} active={track.locked} onClick={() => void edit({ type: 'update_track', id: track.id, patch: { locked: !track.locked } })}>{track.locked ? <LockKeyhole size={13} /> : <UnlockKeyhole size={13} />}</IconButton></div></div>
          <div className="track-lane" style={{ left: HEADER, width }} onClick={e => { if (e.target === e.currentTarget) { setActiveTrack(track.id); setSelected([]); const r = e.currentTarget.getBoundingClientRect(); setFrame(Math.max(0, Math.round((e.clientX - r.left) / scale))); } }} onDragOver={e => { if (!track.locked) e.preventDefault(); }} onDrop={e => { e.preventDefault(); const media_id = e.dataTransfer.getData('application/mono-media'); if (media_id && !track.locked) { const rect = e.currentTarget.getBoundingClientRect(); void edit({ type: 'add_clip', media_id, track_id: track.id, start: snapped(Math.max(0, Math.round((e.clientX - rect.left) / scale)), []) }); } }}>
            {visible.map(c => drawClip(c, track.id))}
          </div>
        </div>)}
        {(project.in_point !== null || project.out_point !== null) && <div className="range-band" style={{ left: HEADER + (project.in_point || 0) * scale, width: Math.max(1, ((project.out_point ?? endFrame(project)) - (project.in_point || 0)) * scale) }} />}
        <div className="playhead" style={{ left: HEADER + frame * scale }}><div className="playhead-cap" /><div className="playhead-line" /></div>
        {!project.clips.length && <div className="timeline-empty" style={{ left: HEADER + 20 }}>Drag media here, or select a file and press Add to Timeline.</div>}
      </div>
    </div>
    <footer className="timeline-footer"><span>{selected.length ? `${selected.length} clip${selected.length > 1 ? 's' : ''} selected` : `${project.clips.length} clips`}</span><span>{Math.round(fpsValue(project.fps) * 1000) / 1000} fps <span className="footer-divider">/</span> {project.width} × {project.height} <span className="footer-divider">/</span> {timecode(endFrame(project), project.fps)}</span></footer>
  </section>;
}
