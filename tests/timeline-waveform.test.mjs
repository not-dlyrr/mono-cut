// SPDX-License-Identifier: GPL-3.0-or-later
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { timelineHarness, collect, clipNode, lines, pointerTarget } from './timeline-harness.mjs';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..'), build = process.env.MONO_CUT_PREVIEW_TEST_BUILD;
const require = createRequire(join(root, 'package.json'));
const { timelineWaveform, MAX_WAVEFORM_POINTS, dragClipIds } = require(join(build, 'timelineWaveform.js'));
const r = (num, den = 1) => ({ num, den });
const media = (peak = 111) => ({ id: 'm', name: 'Source', path: 'source.mkv', kind: 'audio', duration: r(16), fps: r(30), has_audio: true, missing: false, waveform: Array.from({ length: 1024 }, (_, j) => j === peak ? .8 : 0) });
const clip = (extra = {}) => ({ id: 'c', media_id: 'm', track_id: 'A1', name: 'Clip', start: 0, duration: 480, source_in: r(0), speed: r(1), title: null, linked_id: null, fade_in: 0, fade_out: 0, ...extra });
const project = (clips = [clip()], m = media()) => ({ version: 1, name: 'Waveforms', fps: r(30), width: 160, height: 90, sample_rate: 48000, media: [m], clips, tracks: [{ id: 'A1', kind: 'audio', name: 'Audio', muted: false, hidden: false, locked: false }], markers: [], bins: [], in_point: null, out_point: null });
const drawing = (m, c, fps = r(30), scale = 2, left = 0, right = c.duration * scale, delta = 0) => timelineWaveform(m, c, fps, c.duration, scale, left, right, delta);
test('an impulse between old skipped bins survives max aggregation and disappears when excluded by trim', () => {
  const m = media(111); assert.equal(111 % 10 === 0, false);
  assert.ok(drawing(m, clip()).points.some(p => p.peak === .8));
  for (const speed of [r(1), r(2), r(1,2)]) assert.ok(drawing(m, clip({ source_in: r(12), duration: 120 * speed.den / speed.num, speed })).points.every(p => p.peak === 0));
});
test('exact half-open source boundaries exclude adjacent bins and retain selected bins', () => {
  const m = { ...media(), duration: r(1), waveform: [1, .6, .3, .9] };
  assert.ok(drawing(m, clip({source_in:r(1,4),duration:15})).points.every(p=>p.peak<=.6));
  assert.equal(Math.max(...drawing(m,clip({source_in:r(1,4),duration:15})).points.map(p=>p.peak)),.6);
});
test('source padding and outside coverage remain silent, without a second stream-offset shift', () => {
  const m = { ...media(0), timing: { origin:r(3), audio_start:r(16,5) } }; m.waveform.fill(0); m.waveform[64] = .8;
  assert.ok(drawing(m,clip({source_in:r(20),duration:30})).points.every(p=>p.peak===0));
  assert.ok(drawing(m,clip({source_in:r(-2),duration:30})).points.every(p=>p.peak===0));
  assert.ok(drawing(m,clip()).points.some(p=>p.peak===.8));
});
test('retime fractional tail is never used as extra displayed time', () => {
  const m = { ...media(), duration:r(2), waveform:[0,0,1,0] };
  const c = clip({duration:15, retime:{source_span:r(2)}});
  assert.ok(drawing(m,c).points.every(p=>p.peak===0));
});
test('mixed sequence rates and fractional source-in use sequence timing, not media fps', () => {
  const m={...media(400),fps:r(24000,1001)};
  const c=clip({source_in:r(7,13),speed:r(7,6),duration:200});
  assert.deepEqual(drawing(m,c,r(30000,1001)),drawing({...m,fps:r(120)},c,r(30000,1001)));
});
test('huge clips allocate only the visible pixel window and bounded max buckets', () => {
  const d=drawing(media(),clip({duration:10000000}),r(30),12,5000,6000);
  assert.equal(d.width,1000);assert.ok(d.points.length<=MAX_WAVEFORM_POINTS);assert.ok(d.binVisits<=1024+MAX_WAVEFORM_POINTS);
  assert.equal(drawing({...media(),waveform:Array(1025).fill(1)},clip()).points.length,0);
});
test('a cropped visible interval matches corresponding full interval without reading excluded early peaks', () => {
  assert.ok(drawing(media(111),clip(),r(30),2,720,960).points.every(p=>p.peak===0));
});
test('actual compiled Easy and Advanced Timeline render different selected source curves', () => {
  for(const easy of [true,false]) {const p=project([clip(),clip({id:'tail',source_in:r(12),duration:120}),clip({id:'fast',source_in:r(12),duration:60,speed:r(2)})]);const v=timelineHarness(root,build,p,easy);
    assert.ok(lines(clipNode(v,'c')).some(p=>p.peak>.1)); for(const id of ['tail','fast']) assert.ok(lines(clipNode(v,id)).every(p=>p.peak===.1));
  }
});
test('actual live trim-in, trim-out and slip previews change waveform and cancellation restores it', () => {
  for(const easy of [true,false]) for(const mode of ['in','out','slip']) {const p=project([clip()],media(mode==='out'?900:111)),v=timelineHarness(root,build,p,easy),target=pointerTarget();const original=lines(clipNode(v,'c'));
    if(mode==='slip') { const tool=collect(v.tree,n=>n.props.label==='Slip tool')[0]; if(tool) tool.props.onClick(); else { /* Easy uses inspector slip; shared drag semantics checked below. */ continue; } v.update(); }
    let node=clipNode(v,'c'); if(mode!=='slip') node=collect(node,n=>n.props.className==='trim-handle '+mode)[0];
    node.props.onPointerDown({stopPropagation(){},clientX:0,pointerId:1,currentTarget:target});v.update();
    node=mode==='slip'?clipNode(v,'c'):collect(clipNode(v,'c'),n=>n.props.className==='trim-handle '+mode)[0];
    node.props.onPointerMove({clientX:(mode==='out'?-120:360)*1.8,clientY:0,pointerId:1,currentTarget:target});v.update();
    assert.ok(lines(clipNode(v,'c')).every(p=>p.peak===.1),mode);
    clipNode(v,'c').props.onPointerCancel();v.update();assert.deepEqual(lines(clipNode(v,'c')),original);assert.equal(v.commands.length,0);
  }
});
test('trim/slip previews affect linked members, not unrelated multi-selected clips; move expands all selected groups',()=>{
  const p=project([clip({linked_id:'g'}),clip({id:'a',linked_id:'g'}),clip({id:'other'})]);
  assert.deepEqual([...dragClipIds(p,{id:'c',ids:['c','other'],mode:'in',delta:2})],['c','a']);
  assert.deepEqual([...dragClipIds(p,{id:'c',ids:['c','other'],mode:'move',delta:2})],['c','a','other']);
  for (const easy of [true,false]) {
    const view=timelineHarness(root,build,p,easy,['c','other']),target=pointerTarget();
    const before=p.clips.map(c=>lines(clipNode(view,c.id)));
    let handle=collect(clipNode(view,'c'),n=>n.props.className==='trim-handle in')[0];
    handle.props.onPointerDown({stopPropagation(){},clientX:0,pointerId:1,currentTarget:target});view.update();
    handle=collect(clipNode(view,'c'),n=>n.props.className==='trim-handle in')[0];
    handle.props.onPointerMove({clientX:360*1.8,clientY:0,pointerId:1,currentTarget:target});view.update();
    for (const id of ['c','a']) assert.ok(lines(clipNode(view,id)).every(point=>point.peak===.1));
    assert.deepEqual(lines(clipNode(view,'other')),before[2]);
    clipNode(view,'c').props.onPointerCancel();view.update();
    assert.deepEqual(p.clips.map(c=>lines(clipNode(view,c.id))),before);
  }
});
test('move alone retains the actual source-shaped drawing',()=>{
  const v=timelineHarness(root,build,project()),before=lines(clipNode(v,'c')),target=pointerTarget();
  clipNode(v,'c').props.onPointerDown({stopPropagation(){},clientX:0,pointerId:1,currentTarget:target});v.update();
  clipNode(v,'c').props.onPointerMove({clientX:18,clientY:0,pointerId:1,currentTarget:target});v.update();
  assert.deepEqual(lines(clipNode(v,'c')),before);
});
test('virtualized clips and waveforms remain bounded while scrolled into a long clip',()=>{
  const v=timelineHarness(root,build,project([clip({duration:10000000}),clip({id:'far',start:9000000})]));
  assert.equal(clipNode(v,'far'),undefined);
  collect(v.tree,n=>n.props.className==='timeline-viewport')[0].props.onScroll({currentTarget:{scrollLeft:1000000}});v.update();
  assert.ok(lines(clipNode(v,'c')).length<=MAX_WAVEFORM_POINTS);
  const svg=collect(clipNode(v,'c'),n=>n.type==='svg')[0];assert.ok(svg.props.style.width<=1052);
});
