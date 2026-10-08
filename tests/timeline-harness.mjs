// SPDX-License-Identifier: GPL-3.0-or-later
// Actual compiled component with controlled hooks/events. No DOM or audio device.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import vm from 'node:vm';
export function timelineHarness(root, build, project, easy = false, selected = []) {
  const require = createRequire(join(root, 'package.json'));
  const slots = []; let cursor = 0, dirty = false;
  const slot = init => slots[cursor] ?? (slots[cursor] = init());
  const react = {
    useRef(initial) { const v = slot(() => ({ current: initial })); cursor++; return v; },
    useState(initial) { const v = slot(() => ({ value: typeof initial === 'function' ? initial() : initial })); cursor++; return [v.value, next => { v.value = typeof next === 'function' ? next(v.value) : next; dirty = true; }]; },
    useEffect() {},
    useMemo(fn, deps) { const v = slot(() => ({ deps: undefined, value: undefined })); cursor++; if (!v.deps || deps.some((d,i)=>!Object.is(d,v.deps[i]))) {v.deps=[...deps];v.value=fn();} return v.value; },
  };
  const jsx = (type, props, key) => ({ type, props: props ?? {}, key });
  const sandbox = { exports: {}, require(name) {
    if (name === 'react') return react;
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'Fragment' };
    if (name === 'lucide-react') return new Proxy({}, { get: (_, key) => String(key) });
    if (name === './ui') return { IconButton: 'IconButton', Menu: 'Menu' };
    if (name === './types' || name === './timelineWaveform') return require(join(build, name.slice(2) + '.js'));
    throw new Error(`Unexpected Timeline import ${name}`);
  } };
  vm.runInNewContext(readFileSync(join(build, 'Timeline.js'), 'utf8'), sandbox, { filename: 'actual-compiled-Timeline.js' });
  const commands = [];
  let props = { project, easy, selected, frame: 0, activeTrack: null, snap: false, setFrame() {},
    setSelected(ids) { props.selected = ids; }, edit: async command => { commands.push(command); },
    addTitle() {}, addMarker() {}, setActiveTrack() {}, setSnap() {} }, tree;
  function update(next = {}) { props = { ...props, ...next }; for (let n = 0; n < 10; n++) { cursor = 0; dirty = false; tree = sandbox.exports.default(props); if (!dirty) return tree; } assert.fail('Timeline hooks did not settle'); }
  update(); return { update, commands, get tree() { return tree; } };
}
export function collect(node, predicate, result = []) {
  if (Array.isArray(node)) node.forEach(child => collect(child, predicate, result));
  else if (node && typeof node === 'object') { if (predicate(node)) result.push(node); collect(node.props?.children, predicate, result); }
  return result;
}
export const clipNode = (view, id) => collect(view.tree, node => node.key === id && node.props.className?.startsWith('timeline-clip '))[0];
export const lines = clip => collect(clip, node => node.type === 'line').map(node => ({ x: node.props.x1, peak: (12 - node.props.y1) / 10 }));
export function pointerTarget() { const captured = new Set(); return { setPointerCapture: id => captured.add(id), hasPointerCapture: id => captured.has(id), releasePointerCapture: id => captured.delete(id) }; }
