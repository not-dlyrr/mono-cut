import type { Project } from './types';

/** Sort object keys while retaining the renderer's significant array order. */
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, v]) => [key, canonical(v)]));
  return value;
}

/** Pure render inputs; the native identity additionally includes actual file stats. */
export function previewModelDescriptor(project: Project, height: number, useProxies: boolean): string {
  const { id: _id, name: _name, bins: _bins, markers: _markers, in_point: _in, out_point: _out, ...renderProject } = project;
  const mediaIds = new Set(project.clips.flatMap(clip => clip.media_id ? [clip.media_id] : []));
  const trackIds = new Set(project.clips.map(clip => clip.track_id));
  const tracks = project.tracks.filter(track => trackIds.has(track.id)).map(({ name: _name, locked: _locked, ...track }) => track);
  const media = project.media.filter(item => mediaIds.has(item.id)).map(({ name: _name, bin_id: _bin, thumbnail: _thumbnail, waveform: _waveform, ...item }) => {
    if (useProxies) return item;
    const { proxy: _proxy, proxy_timing_version: _proxyTiming, ...original } = item;
    return original;
  }).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const clips = project.clips.map(({ name: _name, linked_id: _link, ...clip }) => clip).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  return JSON.stringify(canonical({ height, useProxies, project: { ...renderProject, tracks, media, clips } }));
}
