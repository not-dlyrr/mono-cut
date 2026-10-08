import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open, save } from '@tauri-apps/plugin-dialog';
import { Upload, FolderOpen, Save, Undo2, Redo2, ChevronDown, Search, Folder, Plus, Film, Music2, Image, X, ArrowUpRight, Settings2, Sun, Moon, Keyboard, RefreshCw, FileVideo, AlertTriangle, LoaderCircle, Check, Minus, Square, PanelLeftClose, PanelRightClose, Download, Flag, Type, SlidersHorizontal, Link2, FolderPlus } from 'lucide-react';
import { version as appVersion } from '../package.json';
import Timeline from './Timeline';
import GuidedTour from './GuidedTour';
import Inspector from './Inspector';
import Monitor from './Monitor';
import { previewModelDescriptor } from './previewModel';
import { mergePreviewJob, PreviewScheduler } from './previewScheduler';
import { PlaybackAssetQueue, SourceIntentQueue } from './previewAssets';
import { PreviewBridge, previewErrorMessage, type PreviewIdentity, type PreviewIntent } from './previewBridge';
import { ProgramPreviewController } from './programPreview';
import { localPreviewTime, type PreviewRegion } from './previewRegion';
import { ProgramTransport } from './programTransport';
import type { ProgramMediaAsset, ProgramMediaOwnership } from './programMediaSlots';
import { editorShortcutAllowed } from './editorShortcuts';
import { Dialog, Field, IconButton, Menu, NumberField, Splitter } from './ui';
import { durationLabel, endFrame, fpsValue, seconds, timecode, type Capabilities, type EditCommand, type ExportSettings, type Job, type Media, type Project } from './types';

type Modal = 'new' | 'export' | 'shortcuts' | 'title' | 'marker' | 'bin' | 'capabilities' | 'recovery' | 'replace' | null;
type ReplaceAction = 'new' | 'open' | 'close';
const defaults: Record<string, string> = { play: 'Space', reverse: 'j', stop: 'k', forward: 'l', previous: 'ArrowLeft', next: 'ArrowRight', split: 'Ctrl+b', delete: 'Delete', rippleDelete: 'Shift+Delete', undo: 'Ctrl+z', redo: 'Ctrl+Shift+z', duplicate: 'Ctrl+d', save: 'Ctrl+s', open: 'Ctrl+o', new: 'Ctrl+n', import: 'Ctrl+i', markIn: 'i', markOut: 'o', marker: 'm', nudgeLeft: 'Alt+ArrowLeft', nudgeRight: 'Alt+ArrowRight' };
const shortcutLabels: Record<string, string> = { play: 'Play / pause', reverse: 'Reverse shuttle', stop: 'Stop', forward: 'Forward shuttle', previous: 'Previous frame', next: 'Next frame', split: 'Split at playhead', delete: 'Delete selection', rippleDelete: 'Ripple delete', undo: 'Undo', redo: 'Redo', duplicate: 'Duplicate', save: 'Save project', open: 'Open project', new: 'New project', import: 'Import media', markIn: 'Set in point', markOut: 'Set out point', marker: 'Add marker', nudgeLeft: 'Nudge clips left', nudgeRight: 'Nudge clips right' };
const local = <T,>(key: string, fallback: T): T => { try { return JSON.parse(localStorage.getItem(`mono-cut-${key}`) || 'null') ?? fallback; } catch { return fallback; } };
function normalizeKey(e: KeyboardEvent) { return `${e.ctrlKey || e.metaKey ? 'Ctrl+' : ''}${e.altKey ? 'Alt+' : ''}${e.shiftKey ? 'Shift+' : ''}${e.code === 'Space' ? 'Space' : e.key.length === 1 ? e.key.toLowerCase() : e.key}`; }
const mediaFilters = [{ name: 'Media', extensions: ['mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'mts', 'm2ts', 'mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'png', 'jpg', 'jpeg', 'webp', 'bmp', 'tif', 'tiff'] }];
const framesFromMedia = (m: Media, p: Project) => m.kind === 'image' ? Math.round(fpsValue(p.fps) * 5) : Math.max(1, Math.floor(seconds(m.duration) * fpsValue(p.fps)));

export default function App() {
  const [monitorTarget, setMonitorTarget] = useState<'source' | 'program'>('program');
  const monitorTargetRef = useRef<'source' | 'program'>('program'), sourceSelection = useRef<{ projectId: string; id: string; path: string; kind: string; identity: string } | null>(null);
  const activateMonitor = useCallback((target: 'source' | 'program') => { monitorTargetRef.current = target; setMonitorTarget(target); setMonitorTab(target); }, []);
  function activateWorkspace(e: React.SyntheticEvent) { if (e.target instanceof Element && e.target.closest('.timeline-panel')) activateMonitor('program'); }
  function changeMode(next: 'easy' | 'advanced') { activateMonitor('program'); setMode(next); }
  function sourceIdentity(m: Media) { return JSON.stringify([m.path, m.kind, m.fps.num, m.fps.den, m.duration.num, m.duration.den, m.width, m.height, m.has_audio, m.timing ?? null]); }
  function navigateMonitorTabs(e: React.KeyboardEvent) {
    if (e.defaultPrevented || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    const tab = e.target instanceof Element ? e.target.closest('[data-monitor-tab]')?.getAttribute('data-monitor-tab') : null; if (!tab) return;
    const next = e.key === 'Home' ? 'source' : e.key === 'End' ? 'program' : tab === 'source' ? 'program' : 'source';
    e.preventDefault(); activateMonitor(next); e.currentTarget.querySelector<HTMLButtonElement>(`[data-monitor-tab="${next}"]`)?.focus();
  }
  const [mode, setMode] = useState<'easy' | 'advanced'>(local('mode', 'easy')), [monitorTab, setMonitorTab] = useState<'source' | 'program'>('program'), [tour, setTour] = useState(!local('tour-complete', false)), [tourStep, setTourStep] = useState(0), [mediaTool, setMediaTool] = useState<'media' | 'text'>('media');
  const [project, setProject] = useState<Project | null>(null), [error, setError] = useState<string | null>(null), [busy, setBusy] = useState('Opening editor'), [path, setPath] = useState<string | null>(null), [dirty, setDirty] = useState(false);
  const [selected, setSelected] = useState<string[]>([]), [activeTrack, setActiveTrack] = useState<string | null>(null), [frame, setFrameState] = useState(0), [sourceFrame, setSourceFrame] = useState(0), [sourceShuttle, setSourceShuttle] = useState(0), [sourceId, setSourceId] = useState<string | null>(null), [sourcePath, setSourcePath] = useState<string | null>(null), [sourceBusy, setSourceBusy] = useState(false), [sourceIn, setSourceIn] = useState(0), [sourceOut, setSourceOut] = useState<number | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]), [previewPath, setPreviewPath] = useState<string | null>(null), [previewHeight, setPreviewHeight] = useState(local('preview-height', 540)), [useProxies, setUseProxies] = useState(local('proxies', true)), [previewDirty, setPreviewDirty] = useState(false), [shuttle, setShuttleState] = useState(0), [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [previewRegion, setPreviewRegion] = useState<PreviewRegion | null>(null), [previewPlayable, setPreviewPlayable] = useState(false);
  const [programAsset, setProgramAsset] = useState<ProgramMediaAsset | null>(null), [successor, setSuccessor] = useState<ProgramMediaAsset | null>(null);
  const mediaOwnership = useRef<ProgramMediaOwnership>({ current: null, successor: null }), candidate = useRef<ProgramMediaAsset | null>(null), pendingProgram = useRef<ProgramMediaAsset | null>(null);
  const [modal, setModal] = useState<Modal>(null), [pendingAction, setPendingAction] = useState<ReplaceAction | null>(null), [guardBusy, setGuardBusy] = useState(false), [guardMessage, setGuardMessage] = useState<string | null>(null), [search, setSearch] = useState(''), [binId, setBinId] = useState<string | null>(null), [snap, setSnap] = useState(true), [theme, setTheme] = useState(local('theme', 'dark')), [glass, setGlass] = useState(local('glass', !matchMedia('(prefers-reduced-transparency: reduce)').matches)), [reducedMotion, setReducedMotion] = useState(local('reduced-motion', matchMedia('(prefers-reduced-motion: reduce)').matches)), [shortcuts, setShortcuts] = useState<Record<string, string>>({ ...defaults, ...local('shortcuts', defaults) });
  const [binWidth, setBinWidth] = useState(254), [inspectorWidth, setInspectorWidth] = useState(272), [timelineHeight, setTimelineHeight] = useState(mode === 'easy' ? 240 : 308), [showBins, setShowBins] = useState(true), [showInspector, setShowInspector] = useState(true), [formName, setFormName] = useState('Untitled'), [text, setText] = useState(''), [newWidth, setNewWidth] = useState(1920), [newHeight, setNewHeight] = useState(1080), [newFps, setNewFps] = useState('30000/1001'), [exportSettings, setExportSettings] = useState<ExportSettings>({ width: 1920, height: 1080, fps: { num: 30000, den: 1001 }, codec: 'h264', crf: 18, audio_bitrate: 192, sample_rate: 48000 }), [exportPath, setExportPath] = useState(''), [exportJobId, setExportJobId] = useState<string | null>(null), [recordShortcut, setRecordShortcut] = useState<string | null>(null), [notice, setNotice] = useState<string | null>(null);
  const programVideo = useRef<HTMLVideoElement>(null), sourceVideo = useRef<HTMLVideoElement>(null), frameRef = useRef(0), projectRef = useRef<Project | null>(null), currentPreview = useRef<string | null>(null), editQueue = useRef<Promise<void>>(Promise.resolve()), sourceGeneration = useRef(0), sourceFrameRef = useRef(0);
  const schedulerRef = useRef<PreviewScheduler | null>(null); if (!schedulerRef.current) schedulerRef.current = new PreviewScheduler();
  const previewSession = useRef<number | null>(null), previewBridge = useRef<PreviewBridge | null>(null), programPreview = useRef<ProgramPreviewController | null>(null), programTransport = useRef<ProgramTransport | null>(null);
  if (!programTransport.current) programTransport.current = new ProgramTransport(setShuttleState);
  const setShuttle = useCallback((value: number | ((speed: number) => number)) => { programTransport.current!.request(value); if (programTransport.current!.currentSpeed === 0) programPreview.current?.stopPlayback(); else programPreview.current?.setPlaybackSpeed(programTransport.current!.currentSpeed); }, []);
  const previewSettingsRef = useRef({ height: previewHeight, useProxies }), previewMounted = useRef(true), previewAsset = useRef<string | null>(null), sourceAsset = useRef<string | null>(null), assetQueue = useRef<PlaybackAssetQueue | null>(null), sourceIntentQueue = useRef<SourceIntentQueue | null>(null), handlePreviewJobRef = useRef<(job: Job) => void>(() => {}), renderPreviewRef = useRef<(manual?: boolean) => Promise<void>>(async () => {});
  const dirtyRef = useRef(false), pendingMutations = useRef(0), jobsRef = useRef<Job[]>([]), pendingActionRef = useRef<ReplaceAction | null>(null), guardBusyRef = useRef(false), closingRef = useRef(false), requestReplacementRef = useRef<(action: ReplaceAction) => Promise<void>>(async () => {});
  projectRef.current = project; frameRef.current = frame; sourceFrameRef.current = sourceFrame;
  jobsRef.current = jobs;
  previewSettingsRef.current = { height: previewHeight, useProxies };
  const native = isTauri();
  const report = useCallback((e: unknown) => { setError(typeof e === 'string' ? e : e instanceof Error ? e.message : String(e)); }, []);
  if (!assetQueue.current) assetQueue.current = new PlaybackAssetQueue(assets => invoke<void>('set_playback_assets', { ...assets }), report);
  if (!sourceIntentQueue.current) sourceIntentQueue.current = new SourceIntentQueue(() => invoke<void>('invalidate_source'));
  if (!previewBridge.current) previewBridge.current = new PreviewBridge(schedulerRef.current!, {
    identity: settings => invoke<PreviewIdentity>('preview_identity', { ...settings }),
    intent: (settings, key, revision) => invoke<PreviewIntent>('set_preview_intent', { ...settings, expectedKey: key, session: previewSession.current, revision }),
    render: (settings, key, intent) => invoke<Job>('render_preview', { ...settings, expectedKey: key, intent }),
    cancel: revision => previewSession.current === null ? Promise.resolve() : invoke<void>('cancel_preview_intent', { session: previewSession.current, revision }),
  }, () => previewMounted.current && !closingRef.current);
  function publishProgramAsset(asset: ProgramMediaAsset) {
    if (mediaOwnership.current.successor?.id === asset.id) setSuccessor(null);
    setProgramAsset(asset); setPreviewRegion(asset.region); setPreviewPlayable(asset.playable); setPreviewPath(asset.src); setPreviewDirty(!asset.playable);
    programTransport.current!.resume(asset.region, asset.playable); programPreview.current!.setPlaybackSpeed(programTransport.current!.currentSpeed);
  }
  function publishPlaybackAssets(force = false): Promise<boolean> {
    if (!native) return Promise.resolve(false);
    const current = mediaOwnership.current.current, next = candidate.current, pending = pendingProgram.current;
    const snapshot = { previewPath: current?.path ?? pending?.path ?? null, sourcePath: sourceAsset.current, successorPreviewPath: mediaOwnership.current.successor?.path ?? (pending && current && pending.id !== current.id ? pending.path : next?.path ?? null) };
    return assetQueue.current!.update(snapshot, force).then(pinned => {
      if (pinned && previewMounted.current && !closingRef.current && current && mediaOwnership.current.current?.id === current.id) programPreview.current!.acknowledgeAsset(current.id);
      if (pinned && pending && pendingProgram.current === pending && previewMounted.current && !closingRef.current && (snapshot.previewPath === pending.path || snapshot.successorPreviewPath === pending.path)) {
        publishProgramAsset(pending);
      }
      if (pinned && next && snapshot.successorPreviewPath === next.path && candidate.current === next && previewMounted.current && !closingRef.current && programPreview.current!.successor?.job.id === next.id) setSuccessor(next);
      return pinned;
    });
  }
  if (!programPreview.current) programPreview.current = new ProgramPreviewController(schedulerRef.current!, previewBridge.current!, () => previewSettingsRef.current, {
    unavailable: reason => { programTransport.current!.invalidate(reason === 'seek'); currentPreview.current = null; pendingProgram.current = null; if (!candidate.current) setSuccessor(null); programVideo.current?.pause(); setProgramAsset(null); setPreviewPath(null); setPreviewRegion(null); setPreviewPlayable(false); setPreviewDirty(true); },
    asset: (job, region, playable) => {
      const src = `${convertFileSrc(job.path!)}?preview=${encodeURIComponent(job.id)}`; currentPreview.current = job.id; pendingProgram.current = { id: job.id, path: job.path!, src, region, playable };
      if (mediaOwnership.current.successor?.id === job.id) { publishProgramAsset(pendingProgram.current); return; }
      if (mediaOwnership.current.successor && mediaOwnership.current.successor.id !== job.id) setSuccessor(null);
      void publishPlaybackAssets(true);
    },
    successor: (job, region) => {
      const next = job?.path && region ? { id: job.id, path: job.path, src: `${convertFileSrc(job.path)}?preview=${encodeURIComponent(job.id)}`, region, playable: true } : null;
      candidate.current = next;
      if (!next) { if (!pendingProgram.current || pendingProgram.current.id !== mediaOwnership.current.successor?.id) setSuccessor(null); void publishPlaybackAssets(); return; }
      // The native completion pin protects this file until the consuming pin is confirmed.
      if (mediaOwnership.current.successor && mediaOwnership.current.successor.id !== next.id) setSuccessor(null);
      void publishPlaybackAssets(true);
    },
    job: job => { currentPreview.current = job.id; setJobs(js => [...js.filter(j => j.id !== job.id), mergePreviewJob(js.find(j => j.id === job.id), job as Job)].slice(-50)); void invoke<Job[]>('get_jobs').then(latest => { if (!previewMounted.current || closingRef.current) return; setJobs(js => { const merged = new Map(js.map(j => [j.id, j])); for (const j of latest) merged.set(j.id, mergePreviewJob(merged.get(j.id), j)); return [...merged.values()].slice(-50); }); for (const j of latest) if (j.kind === 'preview') handlePreviewJobRef.current(j); }).catch(report); },
    error: e => { if (previewErrorMessage(e).startsWith('PREVIEW_IDENTITY_CHANGED:')) { void flushEdits().then(() => renderPreviewRef.current(false)); } else report(e); },
  });
  const onProgramAssets = useCallback((assets: ProgramMediaOwnership) => { mediaOwnership.current = assets; if (assets.current?.id === pendingProgram.current?.id) pendingProgram.current = null; previewAsset.current = assets.current?.path ?? null; void publishPlaybackAssets(); }, [native]);
  const onSuccessorReady = useCallback((id: string) => { programPreview.current!.successorReady(id); }, []);
  const onSuccessorFailed = useCallback((id: string, error: unknown) => { programPreview.current!.successorFailed(id, error); }, []);
  const trackProgramFrame = useCallback((n: number) => { frameRef.current = n; programPreview.current!.trackFrame(n); setFrameState(n); }, []);
  const observeProgram = useCallback((p: Project, resetSession = false) => {
    const settings = previewSettingsRef.current;
    const changed = programPreview.current!.observe(previewModelDescriptor(p, settings.height, settings.useProxies), p.fps, endFrame(p), frameRef.current, p.clips.length > 0, resetSession);
    if (!p.clips.length) setPreviewDirty(false);
    return changed;
  }, []);
  const replaceProject = useCallback((p: Project, edited = true, resetSession = false) => {
    const selection = sourceSelection.current, media = p.media.find(m => m.id === selection?.id);
    if (selection && (p.id !== selection.projectId || !media || media.missing || sourceIdentity(media) !== selection.identity)) resetSourceForProject();
    observeProgram(p, resetSession); projectRef.current = p; setProject(p); setSelected(ids => ids.filter(id => p.clips.some(c => c.id === id))); if (edited) { dirtyRef.current = true; setDirty(true); }
  }, [observeProgram]);
  const edit = useCallback((command: EditCommand): Promise<void> => {
    if (closingRef.current) return Promise.resolve();
    pendingMutations.current += 1;
    const run = editQueue.current.then(async () => { try { const p = await invoke<Project>('edit', { command }); replaceProject(p); } catch (e) { report(e); } finally { pendingMutations.current -= 1; } }); editQueue.current = run; return run;
  }, [replaceProject, report]);
  const seekProgram = useCallback((n: number, direction: 1 | -1 = 1) => { const p = projectRef.current; const f = Math.max(0, Math.min(p ? Math.max(endFrame(p), 1) : 0, Math.round(n))); frameRef.current = f; setFrameState(f); const preparing = programPreview.current!.seek(f, direction); if (preparing) void renderPreviewRef.current(false); else if (programVideo.current && p) programVideo.current.currentTime = localPreviewTime(f, p.fps, programPreview.current!.coverage); return preparing; }, []);
  const seek = useCallback((n: number) => { setShuttle(0); programVideo.current?.pause(); seekProgram(n); }, [seekProgram, setShuttle]);
  const advanceProgram = useCallback((n: number, direction: 1 | -1) => { const p = projectRef.current; const f = Math.max(0, Math.min(p ? Math.max(endFrame(p), 1) : 0, Math.round(n))); frameRef.current = f; setFrameState(f); const preparing = programPreview.current!.advance(f, direction); if (preparing) void renderPreviewRef.current(false); return preparing; }, []);
  const pause = useCallback(() => { setShuttle(0); programVideo.current?.pause(); }, [setShuttle]);
  const pauseSource = useCallback(() => { setSourceShuttle(0); sourceVideo.current?.pause(); }, []);
  const stepSource = useCallback((delta: number) => {
    pauseSource(); const p = projectRef.current, m = p?.media.find(m => m.id === sourceId); if (!p || !m || m.missing || m.kind === 'image' || !sourcePath || sourceBusy || sourceSelection.current?.id !== sourceId) return;
    const rate = fpsValue(m.fps) || fpsValue(p.fps), n = Math.max(0, Math.min(Math.round(seconds(m.duration) * rate), sourceFrameRef.current + delta)); setSourceFrame(n); if (sourceVideo.current) sourceVideo.current.currentTime = n / rate;
  }, [pauseSource, sourceId, sourcePath, sourceBusy]);
  useEffect(() => { document.documentElement.dataset.theme = theme; document.documentElement.dataset.glass = glass ? 'on' : 'off'; document.documentElement.dataset.motion = reducedMotion ? 'reduce' : 'full'; for (const [k, v] of Object.entries({ theme, glass, mode, 'reduced-motion': reducedMotion, shortcuts, 'preview-height': previewHeight, proxies: useProxies })) localStorage.setItem(`mono-cut-${k}`, JSON.stringify(v)); }, [theme, glass, mode, reducedMotion, shortcuts, previewHeight, useProxies]);
  useEffect(() => {
    if (!native) { setBusy(''); setError('Mono Cut needs its desktop media engine. Launch the Windows application to edit and export.'); return; }
    let disposed = false; const unlisteners: (() => void)[] = [];
    void (async () => {
      try {
        const offJob = await listen<Job>('job-progress', event => { if (disposed) return; const j = event.payload; setJobs(js => [...js.filter(other => other.id !== j.id), mergePreviewJob(js.find(other => other.id === j.id), j)].slice(-50)); if (j.kind === 'preview') handlePreviewJobRef.current(j); }); unlisteners.push(offJob);
        const offProject = await listen<Project>('project-changed', e => { if (!disposed) replaceProject(e.payload); }); unlisteners.push(offProject);
        const offProjectError = await listen<string>('project-error', e => { if (!disposed) report(e.payload); }); unlisteners.push(offProjectError);
        const offDrop = await getCurrentWindow().onDragDropEvent(e => { if (e.payload.type === 'drop') void importPaths(e.payload.paths); }); unlisteners.push(offDrop);
        previewSession.current = await invoke<number>('begin_preview_session'); if (disposed) return;
        const p = await invoke<Project>('get_project'); if (disposed) return; replaceProject(p, false, true); setActiveTrack(p.tracks.find(t => t.kind === 'video')?.id || null); setBusy('');
        invoke<Capabilities>('capabilities').then(c => { if (!disposed) setCapabilities(c); }).catch(report);
        invoke<boolean>('recovery_available').then(available => { if (!disposed && available) setModal('recovery'); }).catch(report);
        invoke<Job[]>('get_jobs').then(latest => { if (disposed) return; setJobs(js => { const merged = new Map(js.map(j => [j.id, j])); for (const j of latest) merged.set(j.id, mergePreviewJob(merged.get(j.id), j)); return [...merged.values()].slice(-50); }); for (const j of latest) if (j.kind === 'preview') handlePreviewJobRef.current(j); }).catch(report);
      } catch (e) { report(e); setBusy(''); }
    })(); return () => { disposed = true; unlisteners.forEach(fn => fn()); };
  }, []);
  useEffect(() => {
    if (!native) return;
    let disposed = false, unlisten: (() => void) | undefined;
    void getCurrentWindow().onCloseRequested(event => {
      // Commit inspector drafts before deciding whether closing would lose work.
      if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
      if (dirtyRef.current || pendingMutations.current || jobsRef.current.some(j => j.status === 'running') || pendingActionRef.current) {
        event.preventDefault(); void requestReplacementRef.current('close');
      }
    }).then(off => { if (disposed) off(); else unlisten = off; }).catch(report);
    return () => { disposed = true; unlisten?.(); };
  }, [native, report]);
  handlePreviewJobRef.current = job => { if (previewMounted.current && !closingRef.current) programPreview.current!.receive(job); };
  const renderPreview = useCallback(async (manual = true) => {
    if (closingRef.current || !previewMounted.current || !native || previewSession.current === null) return;
    const p = projectRef.current; if (!p) return; observeProgram(p);
    if (manual) await publishPlaybackAssets(true);
    await programPreview.current!.request(manual);
  }, [native, observeProgram]);
  renderPreviewRef.current = renderPreview;
  useEffect(() => { if (project) void renderPreview(false); }, [project, previewHeight, useProxies, renderPreview]);
  useEffect(() => {
    if (!native) return;
    void publishPlaybackAssets();
  }, [native, previewPath, sourcePath, report]);
  useEffect(() => {
    previewMounted.current = true; programPreview.current!.activate();
    return () => { previewMounted.current = false; sourceGeneration.current += 1; programPreview.current!.dispose(); if (native) { void sourceIntentQueue.current!.invalidate().catch(() => {}); void assetQueue.current!.update({ previewPath: null, sourcePath: null, successorPreviewPath: null }, true); } };
  }, [native]);
  function changePreviewSettings(height: number, proxies: boolean) { previewSettingsRef.current = { height, useProxies: proxies }; if (projectRef.current) observeProgram(projectRef.current); setPreviewHeight(height); setUseProxies(proxies); }
  useEffect(() => { if (shuttle >= 0 || !project) return; programVideo.current?.pause(); const interval = window.setInterval(() => { const speed = programTransport.current!.currentSpeed; if (speed >= 0) return; programTransport.current!.advance(frameRef.current - Math.max(1, Math.abs(speed)), seekProgram); if (frameRef.current <= 0) pause(); }, 1000 / fpsValue(project.fps)); return () => clearInterval(interval); }, [shuttle, project?.fps, seekProgram, pause]);
  useEffect(() => { if (sourceShuttle >= 0) return; const media = projectRef.current?.media.find(m => m.id === sourceId); if (!media) return; const rate = fpsValue(media.fps) || fpsValue(projectRef.current!.fps); sourceVideo.current?.pause(); const interval = window.setInterval(() => { const n = Math.max(0, sourceFrameRef.current - Math.abs(sourceShuttle)); setSourceFrame(n); if (sourceVideo.current) sourceVideo.current.currentTime = n / rate; if (n <= 0) setSourceShuttle(0); }, 1000 / rate); return () => clearInterval(interval); }, [sourceShuttle, sourceId]);
  useEffect(() => { if (!notice) return; const t = setTimeout(() => setNotice(null), 4500); return () => clearTimeout(t); }, [notice]);

  async function importPaths(paths: string[]) {
    if (!paths.length || pendingActionRef.current || closingRef.current) return;
    pendingMutations.current += 1;
    const destinationBin = binId;
    const run = editQueue.current.then(async () => {
      setBusy(`Importing ${paths.length} file${paths.length > 1 ? 's' : ''}`);
      try { const p = await invoke<Project>('import_media', { paths }); replaceProject(p); if (destinationBin) for (const m of p.media.filter(m => paths.includes(m.path))) replaceProject(await invoke<Project>('edit', { command: { type: 'assign_bin', media_id: m.id, bin_id: destinationBin } })); setNotice(`${paths.length} file${paths.length > 1 ? 's' : ''} imported`); }
      catch (e) { report(e); } finally { setBusy(''); pendingMutations.current -= 1; }
    }); editQueue.current = run; await run;
  }
  async function importMedia() { try { const paths = await open({ title: 'Import media', multiple: true, filters: mediaFilters }); if (paths) await importPaths(Array.isArray(paths) ? paths : [paths]); } catch (e) { report(e); } }
  async function flushEdits() { let queued: Promise<void>; do { queued = editQueue.current; await queued; } while (queued !== editQueue.current); }
  async function saveProject(as = false): Promise<boolean> {
    if (!projectRef.current) return false;
    try { const target = !as && path ? path : await save({ title: 'Save Mono Cut project', defaultPath: `${projectRef.current.name}.monocut`, filters: [{ name: 'Mono Cut project', extensions: ['monocut'] }] }); if (!target) return false; await flushEdits(); setBusy('Saving project'); const p = await invoke<Project>('save_project', { path: target }); replaceProject(p, false); setPath(target); dirtyRef.current = false; setDirty(false); setNotice('Project saved'); return true; }
    catch (e) { report(e); return false; } finally { setBusy(''); }
  }
  async function openProject() {
    try { const target = await open({ title: 'Open Mono Cut project', multiple: false, filters: [{ name: 'Mono Cut project', extensions: ['monocut', 'json'] }] }); if (!target || Array.isArray(target)) return; setBusy('Opening project'); pause(); pauseSource(); const p = await invoke<Project>('open_project', { path: target }); replaceProject(p, false, true); setPath(target); dirtyRef.current = false; setDirty(false); seek(0); resetSourceForProject(); setBinId(null); setActiveTrack(p.tracks.find(t => t.kind === 'video')?.id || null); activateMonitor('program'); }
    catch (e) { report(e); } finally { setBusy(''); }
  }
  async function history(command: 'undo' | 'redo') {
    pendingMutations.current += 1;
    const run = editQueue.current.then(async () => { try { replaceProject(await invoke<Project>(command)); } catch (e) { report(e); } finally { pendingMutations.current -= 1; } }); editQueue.current = run; await run;
  }
  async function performReplacement(action: ReplaceAction) {
    if (action === 'new') { setFormName('Untitled'); setModal('new'); }
    else if (action === 'open') { setModal(null); await openProject(); }
    else if (native) {
      closingRef.current = true; pause(); pauseSource(); programPreview.current!.cancelPreparation();
      try {
        const active = await invoke<Job[]>('get_jobs'), cancelIds = active.filter(j => j.status === 'running').map(j => j.id);
        for (const id of cancelIds) await invoke('cancel_job', { id });
        const deadline = Date.now() + 5000;
        while (cancelIds.length) {
          const latest = await invoke<Job[]>('get_jobs'); jobsRef.current = latest; setJobs(latest);
          if (!latest.some(j => cancelIds.includes(j.id) && j.status === 'running')) break;
          if (Date.now() >= deadline) throw new Error('Background media jobs are still stopping. Keep the editor open and close it again when they finish.');
          await new Promise(resolve => window.setTimeout(resolve, 100));
        }
        await getCurrentWindow().destroy();
      }
      catch (e) { closingRef.current = false; report(e); }
    }
  }
  async function requestReplacement(action: ReplaceAction) {
    if (pendingActionRef.current || closingRef.current) return;
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    await flushEdits();
    if (pendingActionRef.current || closingRef.current) return;
    pause(); pauseSource();
    if (dirtyRef.current || (action === 'close' && jobsRef.current.some(j => j.status === 'running'))) {
      pendingActionRef.current = action; setPendingAction(action); setGuardMessage(null); setModal('replace');
    } else await performReplacement(action);
  }
  requestReplacementRef.current = requestReplacement;
  function cancelReplacement() { if (guardBusyRef.current) return; pendingActionRef.current = null; setPendingAction(null); setGuardMessage(null); setModal(null); }
  async function resolveReplacement(saveFirst: boolean) {
    const action = pendingActionRef.current; if (!action || guardBusyRef.current) return;
    guardBusyRef.current = true; setGuardBusy(true); setGuardMessage(null);
    try {
      if (saveFirst && !await saveProject()) { setGuardMessage('Your project was not saved. Save again, discard changes, or cancel to keep editing.'); return; }
      if (action === 'close') {
        await performReplacement(action);
        if (!closingRef.current) setGuardMessage('Closing did not finish. Your project is still open.');
      } else { pendingActionRef.current = null; setPendingAction(null); setModal(null); await performReplacement(action); }
    } finally { guardBusyRef.current = false; setGuardBusy(false); }
  }
  function resetSourceForProject() { activateMonitor('program'); sourceSelection.current = null; pauseSource(); sourceGeneration.current += 1; sourceAsset.current = null; setSourceId(null); setSourcePath(null); setSourceBusy(false); setSourceFrame(0); setSourceIn(0); setSourceOut(null); if (native) void sourceIntentQueue.current!.invalidate().catch(report); publishPlaybackAssets(); }
  async function selectSource(m: Media) {
    pause(); pauseSource(); const generation = ++sourceGeneration.current;
    sourceSelection.current = projectRef.current ? { projectId: projectRef.current.id, id: m.id, path: m.path, kind: m.kind, identity: sourceIdentity(m) } : null;
    activateMonitor('source'); sourceAsset.current = null; publishPlaybackAssets(); setSourceId(m.id); setSourcePath(null); setSourceFrame(0); setSourceIn(0); setSourceOut(null); setSourceBusy(!m.missing);
    try {
      await sourceIntentQueue.current!.invalidate();
      if (!previewMounted.current || closingRef.current || generation !== sourceGeneration.current || m.missing) return;
      const path = m.kind === 'image' ? m.path : await invoke<string>('prepare_source', { mediaId: m.id });
      if (!previewMounted.current || closingRef.current) return;
      if (generation === sourceGeneration.current) { sourceAsset.current = path; publishPlaybackAssets(); setSourcePath(convertFileSrc(path)); }
      else publishPlaybackAssets(true);
    } catch (e) { if (previewMounted.current && !closingRef.current && generation === sourceGeneration.current) report(e); }
    finally { if (previewMounted.current && !closingRef.current && generation === sourceGeneration.current) setSourceBusy(false); }
  }
  async function addToTimeline(m?: Media) { const p = projectRef.current; if (!p) return; const media = m || p.media.find(m => m.id === sourceId); if (!media || media.missing) return; const active = p.tracks.find(t => t.id === activeTrack), track = active && !active.locked && (media.kind !== 'audio' || active.kind === 'audio') ? active : p.tracks.find(t => t.kind === (media.kind === 'audio' ? 'audio' : 'video') && !t.locked); if (!track) { report('Add an unlocked track for this media first.'); return; } const inFrame = m ? 0 : sourceIn, outFrame = m ? null : sourceOut; const sourceRate = media.fps.num > 0 ? media.fps : p.fps; const sourceFps = fpsValue(sourceRate); await edit({ type: 'add_clip', media_id: media.id, track_id: track.id, start: frame, source_in: { num: inFrame * sourceRate.den, den: sourceRate.num }, duration: outFrame !== null ? Math.max(1, Math.round((outFrame - inFrame) / sourceFps * fpsValue(p.fps))) : Math.max(1, framesFromMedia(media, p) - Math.round(inFrame / sourceFps * fpsValue(p.fps))) }); activateMonitor('program'); }
  async function relink(m: Media) { try { const replacement = await open({ title: `Relink ${m.name}`, multiple: false, filters: mediaFilters }); if (replacement && !Array.isArray(replacement)) await edit({ type: 'relink', media_id: m.id, path: replacement }); } catch (e) { report(e); } }
  const openTitle = () => { setText('Title'); setModal('title'); }, openMarker = () => { setText(`Marker ${(project?.markers.length || 0) + 1}`); setModal('marker'); };
  function showExport() { if (!project?.clips.length) return; setExportSettings(s => ({ ...s, width: project.width, height: project.height, fps: project.fps })); setExportPath(''); setExportJobId(null); setModal('export'); }
  async function beginExport() { try { let target = exportPath; if (!target) { const value = await save({ title: 'Export video', defaultPath: `${project?.name || 'Sequence'}.${exportSettings.codec === 'h264' ? 'mp4' : 'mkv'}`, filters: [{ name: exportSettings.codec === 'h264' ? 'MP4 video' : 'Matroska lossless', extensions: [exportSettings.codec === 'h264' ? 'mp4' : 'mkv'] }] }); if (!value) return; target = value; setExportPath(value); } const job = await invoke<Job>('export_project', { path: target, settings: exportSettings }); setJobs(js => [...js.filter(j => j.id !== job.id), job]); setExportJobId(job.id); } catch (e) { report(e); } }
  function sourceCommandsReady() {
    const m = project?.media.find(m => m.id === sourceId), selection = sourceSelection.current;
    return !!(m && !m.missing && m.kind !== 'image' && sourcePath && !sourceBusy && selection && selection.id === sourceId && selection.identity === sourceIdentity(m) && selection.projectId === project?.id && projectRef.current?.id === project?.id);
  }
  useEffect(() => { const key = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.isComposing) return;
    if (recordShortcut) { e.preventDefault(); if (e.key === 'Escape') setRecordShortcut(null); else if (!['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) { const key = normalizeKey(e); setShortcuts(s => ({ ...s, [recordShortcut]: key })); setRecordShortcut(null); } return; }
    if (e.defaultPrevented || !editorShortcutAllowed(e.code === 'Space' ? 'Space' : e.key, e.target instanceof Element ? e.target : null, modal !== null)) return;
    const action = Object.entries(shortcuts).find(([, value]) => value === normalizeKey(e))?.[0]; if (!action) return; e.preventDefault();
    const sourceActive = monitorTargetRef.current === 'source', sourceReady = sourceCommandsReady(), programReady = !!previewPath && !previewDirty && previewPlayable;
    if (action === 'play') { if (sourceActive) { if (sourceReady && sourceVideo.current) sourceShuttle !== 0 || !sourceVideo.current.paused ? pauseSource() : setSourceShuttle(1); } else { if (!programReady) return; const v = programVideo.current; if (v) { if (shuttle !== 0 || !v.paused) pause(); else setShuttle(1); } } }
    if (action === 'stop') { pause(); pauseSource(); } if (action === 'reverse') { if (sourceActive && sourceReady && sourceVideo.current) setSourceShuttle(s => s < 0 ? Math.max(-4, s * 2) : -1); else if (!sourceActive && programReady) setShuttle(s => s < 0 ? Math.max(-4, s * 2) : -1); } if (action === 'forward') { if (sourceActive && sourceReady && sourceVideo.current) setSourceShuttle(s => s > 0 ? Math.min(4, s * 2) : 1); else if (!sourceActive && programReady) setShuttle(s => s > 0 ? Math.min(4, s * 2) : 1); }
    if (action === 'previous' || action === 'next') { const delta = action === 'previous' ? -1 : 1; if (sourceActive) { if (sourceReady) stepSource(delta); } else { pause(); seek(frameRef.current + delta); } }
    if (action === 'save') void saveProject(); if (action === 'open' || action === 'new') void requestReplacement(action); if (action === 'import') void importMedia(); if (action === 'undo' || action === 'redo') void history(action);
    if (action === 'split' && selected.length) void edit({ type: 'split', ids: selected, frame }); if ((action === 'delete' || action === 'rippleDelete') && selected.length) void edit({ type: 'delete', ids: selected, ripple: action === 'rippleDelete' }); if (action === 'duplicate' && selected.length) void edit({ type: 'duplicate', ids: selected });
    if (action === 'nudgeLeft' || action === 'nudgeRight') { if (selected.length) void edit({ type: 'move', ids: selected, delta: action === 'nudgeLeft' ? -1 : 1 }); }
    if (action === 'markIn' && project) { if (sourceActive) { if (sourceReady) setSourceIn(sourceFrameRef.current); } else void edit({ type: 'set_range', in_point: frameRef.current, out_point: project.out_point }); } if (action === 'markOut' && project) { if (sourceActive) { if (sourceReady) setSourceOut(sourceFrameRef.current); } else void edit({ type: 'set_range', in_point: project.in_point, out_point: frameRef.current }); } if (action === 'marker') openMarker();
  }; document.addEventListener('keydown', key); return () => document.removeEventListener('keydown', key); }, [shortcuts, recordShortcut, modal, previewPath, previewDirty, previewPlayable, frame, selected, project, pause, pauseSource, stepSource, seek, sourcePath, sourceBusy, sourceFrame, sourceId, shuttle, sourceShuttle]);
  const source = project?.media.find(m => m.id === sourceId), previewJob = jobs.find(j => j.id === currentPreview.current), exportJob = jobs.find(j => j.id === exportJobId), runningJobs = jobs.filter(j => j.status === 'running' && j.kind !== 'preview');
  const mediaList = project?.media.filter(m => (!binId || m.bin_id === binId) && m.name.toLowerCase().includes(search.toLowerCase())) || [];
  const markRange = (edge: 'in' | 'out') => { if (project) void edit({ type: 'set_range', in_point: edge === 'in' ? frameRef.current : project.in_point, out_point: edge === 'out' ? frameRef.current : project.out_point }); };
  const closeTour = useCallback(() => { setTour(false); localStorage.setItem('mono-cut-tour-complete', 'true'); }, []);
  useEffect(() => { if (mode === 'easy') { if (monitorTab === 'source') pause(); else pauseSource(); } }, [monitorTab, mode]);
  return <div className={`app-shell ${mode === 'easy' ? 'easy' : 'advanced'}`} data-monitor={monitorTab} onFocusCapture={activateWorkspace} onPointerDownCapture={activateWorkspace}>
    <svg width="0" height="0" className="filter-defs" aria-hidden="true"><defs><filter id="mono-glass-rim" x="-8%" y="-12%" width="116%" height="124%"><feTurbulence type="fractalNoise" baseFrequency=".009 .016" numOctaves="1" seed="9" result="curve" /><feDisplacementMap in="SourceGraphic" in2="curve" scale="4" xChannelSelector="R" yChannelSelector="G" /><feGaussianBlur stdDeviation=".35" /></filter></defs></svg>
    <header className="app-header"><div className="brand" data-tauri-drag-region><svg width="27" height="27" viewBox="0 0 28 28" aria-hidden="true"><path d="M4 23V5h5l5 9 5-9h5v18h-5V13l-5 8-5-8v10Z" fill="currentColor" /><path d="M5 25h18" stroke="currentColor" strokeWidth="1" /></svg><span>Mono Cut</span></div>
      <div className="main-menus"><Menu label="Project"><button data-close-menu onClick={() => void requestReplacement('new')}>New project<span>Ctrl+N</span></button><button data-close-menu onClick={() => void requestReplacement('open')}><FolderOpen size={15} />Open project<span>Ctrl+O</span></button><button data-close-menu onClick={() => void saveProject()} disabled={!project}><Save size={15} />Save<span>Ctrl+S</span></button><button data-close-menu onClick={() => void saveProject(true)} disabled={!project}>Save as…</button><div className="menu-divider" /><button data-close-menu onClick={() => void importMedia()}><Upload size={15} />Import media<span>Ctrl+I</span></button><button data-close-menu onClick={showExport} disabled={!project?.clips.length}>Export…</button></Menu>
        <Menu label="Workspace"><button onClick={() => setShowBins(v => !v)}><PanelLeftClose size={15} />Media bin{showBins && <Check size={14} />}</button><button onClick={() => setShowInspector(v => !v)}><PanelRightClose size={15} />Inspector{showInspector && <Check size={14} />}</button><button data-close-menu onClick={() => { setBinWidth(254); setInspectorWidth(272); setTimelineHeight(308); setShowBins(true); setShowInspector(true); }}>Reset panel layout</button><div className="menu-divider" /><button data-close-menu onClick={() => setModal('shortcuts')}><Keyboard size={15} />Keyboard shortcuts</button><button data-close-menu onClick={() => setModal('capabilities')}><Settings2 size={15} />Media engine</button></Menu>
        <Menu label="Appearance"><div className="menu-caption">Theme</div><button onClick={() => setTheme('dark')}><Moon size={15} />Dark{theme === 'dark' && <Check size={14} />}</button><button onClick={() => setTheme('light')}><Sun size={15} />Light{theme === 'light' && <Check size={14} />}</button><div className="menu-divider" /><label className="menu-check"><span>Liquid glass</span><input type="checkbox" checked={glass} onChange={e => setGlass(e.target.checked)} /></label><label className="menu-check"><span>Reduced motion</span><input type="checkbox" checked={reducedMotion} onChange={e => setReducedMotion(e.target.checked)} /></label><p className="menu-note">Turn glass off for reduced transparency.</p></Menu>
      <Menu label="Help"><button data-close-menu onClick={() => { setShowBins(true); setShowInspector(true); setTourStep(0); setTour(true); }}>Restart tutorial</button><button data-close-menu onClick={() => setModal('shortcuts')}><Keyboard size={15} />Keyboard shortcuts</button><button data-close-menu onClick={() => setModal('capabilities')}>About this release</button></Menu></div><label className="mode-control"><select aria-label="Editor mode" value={mode} onChange={e => changeMode(e.target.value as 'easy' | 'advanced')}><option value="easy">Easy mode</option><option value="advanced">Advanced</option></select></label><div className="header-project" data-tauri-drag-region><span>{project?.name || 'Mono Cut'}</span>{dirty && <span className="unsaved" aria-label="Unsaved changes">•</span>}<span className="header-project-meta">{project ? `${project.width} × ${project.height}` : ''}</span></div>
      <div className="header-actions"><IconButton label="Undo" disabled={!project || !!busy} onClick={() => void history('undo')}><Undo2 size={16} /></IconButton><IconButton label="Redo" disabled={!project || !!busy} onClick={() => void history('redo')}><Redo2 size={16} /></IconButton><IconButton data-tour="save" label="Save project" disabled={!project || !!busy} onClick={() => void saveProject()}><Save size={16} /></IconButton><button data-tour="export" className="export-button glass" disabled={!project?.clips.length || !!busy} onClick={showExport}>Export<ArrowUpRight size={15} /></button></div>
      <div className="window-controls"><IconButton label="Minimize window" onClick={() => native && void getCurrentWindow().minimize()}><Minus size={14} /></IconButton><IconButton label="Maximize or restore window" onClick={() => native && void getCurrentWindow().toggleMaximize()}><Square size={11} /></IconButton><IconButton label="Close window" className="window-close" onClick={() => native && void requestReplacement('close')}><X size={15} /></IconButton></div>
    </header>
    {error && <div className="error-banner" role="alert"><AlertTriangle size={16} /><span>{error}</span><IconButton label="Dismiss error" onClick={() => setError(null)}><X size={15} /></IconButton></div>}
    {project ? <main className="editor-main" style={{ gridTemplateRows: `minmax(240px, 1fr) 6px ${timelineHeight}px` }}>
      <div className="workspace" style={{ gridTemplateColumns: `${showBins ? `${binWidth}px 6px` : ''} minmax(390px,1fr) ${showInspector ? `6px ${inspectorWidth}px` : ''}` }}>
        {showBins && <><aside className="media-panel panel" aria-label="Project media">{mode === 'easy' && <nav className="easy-media-tools" aria-label="Content tools"><button className={mediaTool === 'media' ? 'active' : ''} onClick={() => setMediaTool('media')}><Film size={17} />Media</button><button className={mediaTool === 'text' ? 'active' : ''} onClick={() => setMediaTool('text')}><Type size={17} />Text</button></nav>}<header className="panel-heading"><span><FolderOpen size={15} />Media</span><span className="badge">{project.media.length}</span><IconButton data-tour="import" label="Import media" disabled={!!busy} onClick={() => void importMedia()}><Plus size={16} /></IconButton></header>
          {mode === 'easy' && mediaTool === 'text' && <div className="easy-text-panel"><Type size={28} strokeWidth={1.2} /><strong>Add your words</strong><p>A title sits on a video track. Change its text, position and fades in the inspector.</p><button className="primary-button" onClick={openTitle}><Plus size={14} />Add text</button></div>}<div className={`media-search ${mode === 'easy' && mediaTool === 'text' ? 'tool-hidden' : ''}`}><Search size={14} /><input aria-label="Search media" placeholder="Search media" value={search} onChange={e => setSearch(e.target.value)} /></div>
          <div className={`bin-navigation ${mode === 'easy' && mediaTool === 'text' ? 'tool-hidden' : ''}`}><button className={!binId ? 'selected' : ''} onClick={() => setBinId(null)}><Folder size={14} /><span>All media</span><span>{project.media.length}</span></button>{project.bins.map(b => <button key={b.id} className={binId === b.id ? 'selected' : ''} onClick={() => setBinId(b.id)}><Folder size={14} /><span>{b.name}</span><span>{project.media.filter(m => m.bin_id === b.id).length}</span></button>)}<button className="add-bin" onClick={() => { setText('New bin'); setModal('bin'); }}><FolderPlus size={14} />New bin</button></div>
          <div className={`media-list ${mode === 'easy' && mediaTool === 'text' ? 'tool-hidden' : ''}`}>{mediaList.length ? mediaList.map(m => <div key={m.id} className={`media-item ${sourceId === m.id ? 'selected' : ''} ${m.missing ? 'missing' : ''}`}>
            <button className="media-item-content" onClick={() => void selectSource(m)} onDoubleClick={() => void addToTimeline(m)} draggable={!m.missing} onDragStart={e => { e.dataTransfer.setData('application/mono-media', m.id); e.dataTransfer.effectAllowed = 'copy'; }}>
              <div className="media-thumbnail">{m.thumbnail ? <img src={convertFileSrc(m.thumbnail)} alt="" loading="lazy" /> : m.kind === 'audio' ? <Music2 size={22} /> : m.kind === 'image' ? <Image size={22} /> : <Film size={22} />}{m.missing && <AlertTriangle size={17} className="media-missing-icon" />}<span>{m.kind === 'image' ? 'STILL' : durationLabel(m.duration)}</span></div><div className="media-item-text"><strong>{m.name}</strong><span>{m.missing ? 'Missing media' : m.kind === 'audio' ? 'Audio' : `${m.width} × ${m.height}`}{m.proxy && ' · Proxy'}</span></div>
            </button><Menu label="•••"><button data-close-menu disabled={m.missing} onClick={() => void addToTimeline(m)}>Add to timeline</button><button data-close-menu onClick={() => void relink(m)}><Link2 size={15} />Relink file…</button><button data-close-menu disabled={m.kind !== 'video' || m.missing || !!m.proxy} onClick={async () => { try { const j = await invoke<Job>('generate_proxy', { mediaId: m.id }); setJobs(js => [...js, j]); } catch (e) { report(e); } }}>Generate proxy</button><div className="menu-divider" /><div className="menu-caption">Move to bin</div><button data-close-menu onClick={() => void edit({ type: 'assign_bin', media_id: m.id, bin_id: null })}>All media</button>{project.bins.map(b => <button data-close-menu key={b.id} onClick={() => void edit({ type: 'assign_bin', media_id: m.id, bin_id: b.id })}>{b.name}</button>)}</Menu>
          </div>) : <div className="media-empty"><div className="empty-film"><Film size={29} strokeWidth={1.1} /></div><strong>{project.media.length ? 'No matching media' : 'Start with your media'}</strong><p>{project.media.length ? 'Try another search or bin.' : 'Video, audio and images, ready for your timeline.'}</p>{!project.media.length && <button className="primary-button" disabled={!!busy} onClick={() => void importMedia()}><Upload size={15} />Import media</button>}</div>}</div>
          <footer className="media-footer"><button data-tour="add" className="subtle-button" disabled={!source || source.missing} onClick={() => void addToTimeline()}><Plus size={14} />Add to Timeline</button></footer>
        </aside><Splitter direction="horizontal" label="Resize media bin" onMove={d => setBinWidth(v => Math.min(360, Math.max(200, v + d)))} /></>}
        <div data-tour="preview" className="monitors-area">{mode === 'easy' && <div className="easy-monitor-tabs" role="tablist" aria-label="Preview content" onKeyDown={navigateMonitorTabs}><button role="tab" data-monitor-tab="source" tabIndex={monitorTab === 'source' ? 0 : -1} aria-selected={monitorTab === 'source'} onClick={() => activateMonitor('source')}>Clip</button><button role="tab" data-monitor-tab="program" tabIndex={monitorTab === 'program' ? 0 : -1} aria-selected={monitorTab === 'program'} onClick={() => activateMonitor('program')}>Timeline</button><span>{monitorTab === 'source' ? 'Original media' : 'Your edit'}</span></div>}<div className="monitors"><Monitor active={monitorTarget === 'source'} onActivate={() => activateMonitor('source')} markable={sourceCommandsReady()} playable={sourceCommandsReady()} title="Source" name={source?.name} src={sourcePath} image={source?.kind === 'image'} audio={source?.kind === 'audio'} waveform={source?.waveform} fps={source?.fps && source.fps.num > 0 ? source.fps : project.fps} frame={sourceFrame} duration={source ? Math.round(seconds(source.duration) * (fpsValue(source.fps) || fpsValue(project.fps))) : 0} setFrame={setSourceFrame} videoRef={sourceVideo} speed={sourceShuttle} onPlay={() => { if (sourceCommandsReady()) setSourceShuttle(1); }} onPause={pauseSource} onStep={stepSource} status={sourceBusy ? 'Preparing source…' : source?.missing ? 'Media missing · relink this file' : undefined} onMarkIn={() => { if (sourceCommandsReady()) setSourceIn(sourceFrameRef.current); }} onMarkOut={() => { if (sourceCommandsReady()) setSourceOut(sourceFrameRef.current); }} footer={<span>{source ? `${source.kind === 'image' ? 'Still image' : `${timecode(sourceIn, source.fps.num ? source.fps : project.fps)} → ${sourceOut !== null ? timecode(sourceOut, source.fps.num ? source.fps : project.fps) : 'End'}`}` : 'Source range'}{source && <button className="text-button" onClick={() => { setSourceIn(0); setSourceOut(null); }}>Clear</button>}</span>} />
          <Monitor active={monitorTarget === 'program'} onActivate={() => activateMonitor('program')} markable title="Program" name={project.name} src={previewPath} fps={project.fps} region={previewRegion} playable={previewPlayable} onSeek={seek} onBoundary={() => { const next = programPreview.current!.boundary(); if (next === null || programTransport.current!.currentSpeed <= 0) pause(); else programTransport.current!.advance(next, advanceProgram); }} frame={frame} duration={endFrame(project)} setFrame={trackProgramFrame} programAsset={programAsset} successor={successor} onSuccessorReady={onSuccessorReady} onSuccessorFailed={onSuccessorFailed} onProgramAssets={onProgramAssets} videoRef={programVideo} speed={shuttle} onPlay={() => { if (previewPath && !previewDirty && previewPlayable) setShuttle(1); }} onPause={pause} onStep={delta => { pause(); seek(frameRef.current + delta); }} status={previewDirty ? previewJob?.status === 'running' ? `Rendering preview · ${Math.round(previewJob.progress * 100)}%` : 'Preview needs rendering…' : undefined} onMarkIn={() => markRange('in')} onMarkOut={() => markRange('out')} footer={<span>{project.in_point !== null ? `In ${timecode(project.in_point, project.fps)}` : 'Sequence range'}{project.out_point !== null && ` · Out ${timecode(project.out_point, project.fps)}`}{(project.in_point !== null || project.out_point !== null) && <button className="text-button" onClick={() => void edit({ type: 'set_range', in_point: null, out_point: null })}>Clear</button>}</span>} /></div>
          <div className="preview-toolbar"><div className="preview-options"><label>Preview<select aria-label="Preview resolution" value={previewHeight} onChange={e => changePreviewSettings(Number(e.target.value), useProxies)}><option value={360}>360p</option><option value={540}>540p</option><option value={720}>720p</option><option value={1080}>1080p</option></select></label><label className="inline-check"><input type="checkbox" checked={useProxies} onChange={e => changePreviewSettings(previewHeight, e.target.checked)} />Use proxies</label><IconButton label="Render program preview" disabled={!project.clips.length} onClick={() => void renderPreview()}><RefreshCw size={14} /></IconButton></div><span className="preview-state">{previewDirty && project.clips.length ? <><LoaderCircle size={13} className="spin" />Updating</> : project.clips.length ? 'Preview ready' : 'No sequence clips'}{shuttle !== 0 && ` · ${shuttle}×`}</span></div>
        </div>
        {showInspector && <><Splitter direction="horizontal" label="Resize inspector" onMove={d => setInspectorWidth(v => Math.min(380, Math.max(238, v - d)))} /><Inspector easy={mode === 'easy'} project={project} selected={selected} frame={frame} edit={edit} /></>}
      </div><Splitter direction="vertical" label="Resize timeline" onMove={d => setTimelineHeight(v => Math.min(window.innerHeight - 320, Math.max(180, v - d)))} />
      <Timeline easy={mode === 'easy'} project={project} frame={frame} setFrame={seek} selected={selected} setSelected={setSelected} edit={edit} addTitle={openTitle} addMarker={openMarker} activeTrack={activeTrack} setActiveTrack={setActiveTrack} snap={snap} setSnap={setSnap} />
    </main> : <div className="startup"><svg viewBox="0 0 28 28" width="44" height="44" aria-hidden="true"><path d="M4 23V5h5l5 9 5-9h5v18h-5V13l-5 8-5-8v10Z" fill="currentColor" /></svg><h1>Mono Cut</h1><p>{busy || 'Desktop media engine unavailable'}</p></div>}
    <footer className="app-status"><span>{busy ? <><LoaderCircle size={12} className="spin" />{busy}</> : notice || (project ? (path ? 'Saved project · autosave active' : 'Autosave active') : 'Media engine unavailable')}{project?.media.some(m => m.missing) && <button className="text-button warning-text" onClick={() => { const m = project.media.find(m => m.missing); if (m) void relink(m); }}>Missing media · Relink</button>}</span><div>{runningJobs.map(j => <span key={j.id}>{j.kind} {Math.round(j.progress * 100)}%<button className="text-button" aria-label={`Cancel ${j.kind}`} onClick={() => void invoke('cancel_job', { id: j.id }).catch(report)}>Cancel</button></span>)}<span className="status-engine">Software media engine</span><span>v{appVersion}</span></div></footer>
    {tour && project && !modal && <GuidedTour step={tourStep} setStep={setTourStep} onClose={closeTour} onStep={step => { if (step === 0 || step === 2) { setShowBins(true); setMediaTool('media'); } if (step === 4) setShowInspector(true); }} />}
    {modal === 'replace' && <Dialog title={dirty ? 'Save your changes?' : 'Stop background work?'} onClose={cancelReplacement}><div className="modal-body"><p>{dirty ? <>Your changes to <strong>{project?.name || 'Untitled'}</strong> have not been saved to a project file. Save before {pendingAction === 'close' ? 'closing Mono Cut' : pendingAction === 'new' ? 'creating a new project' : 'opening another project'}.</> : 'Media jobs are still running. Closing Mono Cut will cancel them.'}</p>{pendingAction === 'close' && dirty && jobs.some(j => j.status === 'running' && j.kind !== 'preview') && <p>Closing also cancels active exports and proxy jobs.</p>}{guardMessage && <p className="warning-text" role="alert">{guardMessage}</p>}</div><div className="modal-actions"><button className="subtle-button" disabled={guardBusy} onClick={cancelReplacement} autoFocus>Cancel</button><button className="subtle-button" disabled={guardBusy} onClick={() => void resolveReplacement(false)}>{dirty ? 'Discard changes' : 'Stop jobs & close'}</button>{dirty && <button className="primary-button" disabled={guardBusy} onClick={() => void resolveReplacement(true)}>{guardBusy ? <><LoaderCircle size={14} className="spin" />Saving…</> : <><Save size={14} />Save project</>}</button>}</div></Dialog>}
    {modal === 'new' && <Dialog title="New project" onClose={() => setModal(null)}><div className="modal-body"><Field label="Project name"><input value={formName} onChange={e => setFormName(e.target.value)} autoFocus /></Field><div className="field-pair"><NumberField label="Width" value={newWidth} min={16} max={7680} onChange={setNewWidth} suffix="px" /><NumberField label="Height" value={newHeight} min={16} max={4320} onChange={setNewHeight} suffix="px" /></div><Field label="Frame rate"><select value={newFps} onChange={e => setNewFps(e.target.value)}>{[['24000/1001', '23.976'], ['24/1', '24'], ['25/1', '25'], ['30000/1001', '29.97'], ['30/1', '30'], ['50/1', '50'], ['60000/1001', '59.94'], ['60/1', '60']].map(([v, name]) => <option value={v} key={v}>{name} fps</option>)}</select></Field></div><div className="modal-actions"><button className="subtle-button" onClick={() => setModal(null)}>Cancel</button><button className="primary-button" onClick={async () => { try { await editQueue.current; pause(); pauseSource(); const [num, den] = newFps.split('/').map(Number); const p = await invoke<Project>('new_project', { name: formName || 'Untitled', width: Math.round(newWidth), height: Math.round(newHeight), fps: { num, den } }); replaceProject(p, true, true); setPath(null); resetSourceForProject(); setBinId(null); setActiveTrack(p.tracks.find(t => t.kind === 'video')?.id || null); activateMonitor('program'); seek(0); setModal(null); } catch (e) { report(e); } }}>Create project</button></div></Dialog>}
    {(modal === 'title' || modal === 'marker' || modal === 'bin') && <Dialog title={modal === 'title' ? 'Add title' : modal === 'marker' ? 'Add marker' : 'New bin'} onClose={() => setModal(null)}><div className="modal-body"><Field label={modal === 'title' ? 'Title text' : 'Name'}><input value={text} onChange={e => setText(e.target.value)} autoFocus /></Field>{modal !== 'bin' && project && <p className="field-help">At playhead {timecode(frame, project.fps)}{modal === 'title' && ' · 5 seconds'}</p>}</div><div className="modal-actions"><button className="subtle-button" onClick={() => setModal(null)}>Cancel</button><button className="primary-button" disabled={!text.trim()} onClick={async () => { if (!project) return; if (modal === 'title') { const track = project.tracks.find(t => t.id === activeTrack && t.kind === 'video' && !t.locked) || project.tracks.find(t => t.kind === 'video' && !t.locked); if (!track) { report('Add an unlocked video track for this title.'); return; } await edit({ type: 'add_title', track_id: track.id, start: frame, duration: Math.round(fpsValue(project.fps) * 5), text }); } else if (modal === 'marker') await edit({ type: 'marker', frame, name: text }); else await edit({ type: 'add_bin', name: text }); setModal(null); }}>Add {modal}</button></div></Dialog>}
    {modal === 'recovery' && <Dialog title="Recover your project" onClose={() => setModal(null)}><div className="modal-body"><p>A recoverable autosave is available. Restore it to resume editing.</p></div><div className="modal-actions"><button className="subtle-button" onClick={() => setModal(null)}>Keep current project</button><button className="primary-button" onClick={async () => { try { const recovered = await invoke<Project>('recover_project'); replaceProject(recovered, true, true); resetSourceForProject(); setPath(null); setBinId(null); seek(0); setActiveTrack(recovered.tracks.find(t => t.kind === 'video')?.id || null); activateMonitor('program'); setModal(null); setNotice('Autosave recovered'); } catch (e) { report(e); } }}>Recover autosave</button></div></Dialog>}
    {modal === 'shortcuts' && <Dialog title="Keyboard shortcuts" wide onClose={() => { setModal(null); setRecordShortcut(null); }}><div className="modal-body shortcut-body"><p className="field-help">Click a shortcut, then press a new key combination. Escape cancels. Reverse shuttle is silent.</p><div className="shortcut-grid">{Object.entries(shortcutLabels).map(([action, label]) => <div className="shortcut-row" key={action}><span>{label}</span><button className={recordShortcut === action ? 'recording' : ''} onClick={() => setRecordShortcut(action)}>{recordShortcut === action ? 'Press keys…' : shortcuts[action]}</button></div>)}</div></div><div className="modal-actions"><button className="subtle-button" onClick={() => setShortcuts(defaults)}>Reset defaults</button><button className="primary-button" onClick={() => setModal(null)}>Done</button></div></Dialog>}
    {modal === 'capabilities' && <Dialog title="Media engine" wide onClose={() => setModal(null)}><div className="modal-body"><div className="engine-info"><FileVideo size={28} /><div><strong>FFmpeg · software fallback</strong><p>{capabilities?.version || 'Detecting media components'}</p></div></div><dl className="engine-details"><dt>Graphics devices</dt><dd>{capabilities?.gpu_devices?.join(', ') || 'No graphics devices reported'}</dd><dt>Available decode interfaces</dt><dd>{capabilities?.hardware.join(', ') || 'No compiled hardware interfaces'}</dd><dt>Export path</dt><dd>Software H.264/AAC or lossless FFV1/FLAC</dd><dt>Rendering</dt><dd>Shared sequence filter compiler for preview and export</dd><dt>Preview architecture</dt><dd>Bounded frame and five-second playback regions</dd></dl><p className="field-help">0.1 is a foundation release. Nested sequences, multicam, scopes, LUTs, masks, tracking, captions and plugins are not implemented yet.</p></div><div className="modal-actions"><button className="primary-button" onClick={() => setModal(null)}>Done</button></div></Dialog>}
    {modal === 'export' && <Dialog title="Export sequence" wide onClose={() => setModal(null)}><div className="modal-body export-body"><div className="export-preview">{previewPath ? <video src={previewPath} preload="metadata" muted onLoadedMetadata={e => { e.currentTarget.currentTime = localPreviewTime(frame, project!.fps, previewRegion); }} /> : <Film size={32} />}<span>{project?.name}</span><span className="secondary">{project && timecode(endFrame(project), project.fps)}</span></div><div className="export-fields"><Field label="Format"><select value={exportSettings.codec} disabled={exportJob?.status === 'running'} onChange={e => setExportSettings(s => ({ ...s, codec: e.target.value as 'h264' | 'ffv1' }))}><option value="h264">H.264 · MP4</option><option value="ffv1">FFV1 · MKV lossless</option></select></Field><div className="field-pair"><NumberField label="Width" value={exportSettings.width} min={16} max={7680} step={2} onChange={v => setExportSettings(s => ({ ...s, width: Math.round(v) }))} /><NumberField label="Height" value={exportSettings.height} min={16} max={4320} step={2} onChange={v => setExportSettings(s => ({ ...s, height: Math.round(v) }))} /></div><Field label="Frame rate"><select value={`${exportSettings.fps.num}/${exportSettings.fps.den}`} onChange={e => { const [num, den] = e.target.value.split('/').map(Number); setExportSettings(s => ({ ...s, fps: { num, den } })); }}>{[['24000/1001', '23.976'], ['24/1', '24'], ['25/1', '25'], ['30000/1001', '29.97'], ['30/1', '30'], ['50/1', '50'], ['60000/1001', '59.94'], ['60/1', '60']].map(([v, name]) => <option value={v} key={v}>{name} fps</option>)}</select></Field>{exportSettings.codec === 'h264' && <NumberField label="Quality (CRF)" value={exportSettings.crf} min={0} max={51} suffix="lower is better" onChange={v => setExportSettings(s => ({ ...s, crf: Math.round(v) }))} />}<div className="field-pair"><Field label="Audio bitrate"><select value={exportSettings.audio_bitrate} onChange={e => setExportSettings(s => ({ ...s, audio_bitrate: Number(e.target.value) }))}><option value={128}>128 kbps</option><option value={192}>192 kbps</option><option value={256}>256 kbps</option><option value={320}>320 kbps</option></select></Field><Field label="Sample rate"><select value={exportSettings.sample_rate} onChange={e => setExportSettings(s => ({ ...s, sample_rate: Number(e.target.value) }))}><option value={48000}>48 kHz</option><option value={44100}>44.1 kHz</option></select></Field></div></div></div>{exportJob && <div className="export-progress"><div><span>{exportJob.status === 'running' ? 'Encoding sequence' : exportJob.status === 'complete' ? 'Export complete' : exportJob.status === 'cancelled' ? 'Export cancelled' : 'Export failed'}</span><span>{Math.round(exportJob.progress * 100)}%</span></div><progress value={exportJob.progress} max="1" aria-label="Export progress" />{exportJob.error && <p role="alert" className="warning-text">{exportJob.error}</p>}<p className="path-text" title={exportPath}>{exportPath}</p></div>}<div className="modal-actions"><button className="subtle-button" onClick={() => setModal(null)}>Close</button>{exportJob?.status === 'running' ? <button className="primary-button" onClick={() => void invoke('cancel_job', { id: exportJob.id }).catch(report)}>Cancel export</button> : <button className="primary-button" onClick={() => void beginExport()}><Download size={15} />{exportJob ? 'Export again' : 'Choose file & export'}</button>}</div></Dialog>}
  </div>;
}
