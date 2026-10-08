// SPDX-License-Identifier: GPL-3.0-or-later
// Silent file-only gate: actual native hydration/edit/render plus compiled Timeline.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdirSync,existsSync} from 'node:fs';
import {resolve,join,relative,sep} from 'node:path';
import {spawn,spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {createInterface} from 'node:readline';
import {timelineHarness,clipNode,lines} from '../tests/timeline-harness.mjs';
const [repoArg,driverArg,templateArg,outArg,freezeArg]=process.argv.slice(2);
if(!freezeArg) throw Error('Usage: node verify-timeline-waveforms.mjs REPO DRIVER TEMPLATE OUTPUT NATIVE_FREEZE');
const repo=resolve(repoArg),driverPath=resolve(driverArg),out=resolve(outArg),freeze=JSON.parse(readFileSync(freezeArg));
assert.ok(out!==repo&&!out.startsWith(repo+sep),'Raw evidence must remain outside checkout');mkdirSync(out,{recursive:true});
const sha=p=>createHash('sha256').update(readFileSync(p)).digest('hex'),r=(num,den=1)=>({num,den}),v=r=>r.num/r.den;
const resources=join(repo,'src-tauri/resources/media'),ffmpeg=join(resources,'ffmpeg.exe'),ffprobe=join(resources,'ffprobe.exe'),font=join(resources,'Inter.ttf');
const nativeHashes=()=>Object.fromEntries(Object.keys(freeze).map(n=>[n,sha(join(repo,n))]));
const report={version:1,stage:6,silent:true,native_ui_or_audio_device_opened:false,helper_sha256:sha(new URL(import.meta.url)),native_before:nativeHashes(),tools:Object.fromEntries([driverPath,ffmpeg,ffprobe,font].map(p=>[p.split(/[\\/]/).at(-1),sha(p)])),cases:[],commands:[],failures:[],verified:false};
assert.deepEqual(report.native_before,freeze);assert.equal(sha(driverPath),'a9a7193ea0943822b651d79ae93f03113037af582b9e7eb229676f89d0f4b0d2');
const run=(exe,args)=>{const x=spawnSync(exe,args.map(String),{cwd:repo,windowsHide:true,encoding:null,maxBuffer:128*1024*1024,timeout:180000});report.commands.push({tool:exe.split(/[\\/]/).at(-1),args,exit:x.status});assert.equal(x.status,0,x.stderr?.toString());return x.stdout;};
const build=join(out,'compiled');mkdirSync(build,{recursive:true});
run(process.execPath,[join(repo,'node_modules/typescript/bin/tsc'),'--target','ES2022','--module','commonjs','--moduleResolution','node','--jsx','react-jsx','--skipLibCheck','--strict','--outDir',build,'--rootDir',join(repo,'src'),join(repo,'src/Timeline.tsx')]);
const require=createRequire(join(repo,'package.json')), {timelineWaveform}=require(join(build,'timelineWaveform.js'));
let driver,serial=0,pending=new Map(),readyResolve;
async function open(path){const ready=new Promise(res=>readyResolve=res);driver=spawn(driverPath,[path,join(out,'cache'),font],{windowsHide:true,env:{...process.env,MONO_CUT_FFMPEG:ffmpeg,MONO_CUT_FFPROBE:ffprobe}});driver.stderr.on('data',b=>writeFileSync(join(out,'driver-stderr.log'),b,{flag:'a'}));createInterface({input:driver.stdout}).on('line',line=>{writeFileSync(join(out,'rpc.jsonl'),line+'\n',{flag:'a'});const x=JSON.parse(line);if(x.event==='ready')readyResolve(x.project);if(x.id&&pending.has(x.id)){const p=pending.get(x.id);clearTimeout(p.timer);pending.delete(x.id);x.error?p.reject(Error(x.error)):p.resolve(x.result);}});return ready;}
function rpc(op,fields={}){return new Promise((resolve,reject)=>{const id=++serial;const timer=setTimeout(()=>{pending.delete(id);reject(Error('RPC timeout '+op));},180000);pending.set(id,{resolve,reject,timer});driver.stdin.write(JSON.stringify({id,op,...fields})+'\n');});}
async function close(){if(!driver)return;const d=driver;driver=undefined;await new Promise((resolve,reject)=>{const t=setTimeout(()=>{d.kill();reject(Error('Driver shutdown timeout'));},30000);d.once('exit',code=>{clearTimeout(t);code===0?resolve():reject(Error('Driver exit '+code));});d.stdin.end();});}
function curves(p){return [true,false].map(easy=>{const view=timelineHarness(repo,build,p,easy);return p.clips.map(c=>({id:c.id,lines:lines(clipNode(view,c.id))}));});}
function clusters(values,rate,threshold=.05,gap=.025){const groups=[];let first=-1,last=-1;for(let i=0;i<values.length;i++){if(Math.abs(values[i])<=threshold)continue;if(first<0)first=i;else if(i-last>gap*rate){groups.push([first/rate,(last+1)/rate]);first=i;}last=i;}if(first>=0)groups.push([first/rate,(last+1)/rate]);return groups;}
function drawingGroups(d,secondsPerPixel){const a=[];let start=null,last=0;const bucket=d.width/d.points.length;for(const p of d.points){if(p.peak>.05){if(start===null)start=(d.left+p.x-bucket/2)*secondsPerPixel;last=(d.left+p.x+bucket/2)*secondsPerPixel;}else if(start!==null){a.push([start,last]);start=null;}}if(start!==null)a.push([start,last]);return a;}
function decode(path,af,rate=4000,copyts=false){const b=run(ffmpeg,['-v','error','-nostdin',...(copyts?['-copyts']:[]),'-i',path,'-map','0:a:0','-vn',...(af?['-af',af]:[]),'-ac','1','-ar',rate,'-f','f32le','pipe:1']);return new Float32Array(b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength));}
function compareGroups(predicted,actual,tolerance){assert.equal(predicted.length,actual.length,JSON.stringify({predicted,actual,tolerance}));let maxError=0;predicted.forEach((p,i)=>p.forEach((x,j)=>{const error=Math.abs(x-actual[i][j]);maxError=Math.max(error,maxError);assert.ok(error<=tolerance,JSON.stringify({predicted,actual,error,tolerance}));}));return maxError;}
async function check(label,p,proxies=false,clipIndex=0){const c=p.clips[clipIndex],m=p.media[0],fps=v(p.fps),speed=v(c.speed),scale=1.8*30/fps;
  const d=timelineWaveform(m,c,p.fps,c.duration,scale,0,c.duration*scale),predicted=drawingGroups(d,1/scale/fps);
  const viewCurves=curves(p);assert.deepEqual(viewCurves[0],viewCurves[1]);
  // Actual component uses the same complete visible window for these <=480-frame clips.
  const component=viewCurves[0][clipIndex].lines;assert.deepEqual(component,d.points.map(x=>({x:x.x,peak:Math.max(1,x.peak*10)/10})));
  const plan=await rpc('plan',{height:120,useProxies:proxies});
  const graph=join(out,label+'-filter.txt');writeFileSync(graph,plan.filter_graph);let args=[...plan.args];args[args.indexOf('-filter_complex_script')+1]=graph;
  for(const flag of ['-preset','-crf','-b:a','-movflags','-g','-keyint_min','-sc_threshold'])while(args.includes(flag))args.splice(args.indexOf(flag),2);
  args[args.indexOf('-c:v')+1]='ffv1';args[args.indexOf('-c:a')+1]='pcm_f32le';const rendered=join(out,label+'.mkv');args[args.length-1]=rendered;run(ffmpeg,args);
  const pcm=decode(rendered,null,48000),first=Math.ceil(c.start/fps*48000),last=Math.ceil((c.start+c.duration)/fps*48000);
  const actual=clusters(pcm.slice(first,last),48000);
  const sourceBinSeconds=Math.ceil(Math.ceil(v(m.duration)*4000)/m.waveform.length)/4000;
  const tolerance=sourceBinSeconds/speed+d.width/d.points.length/scale/fps+1/4000/speed;
  const error=compareGroups(predicted,actual,tolerance);
  if(proxies)assert.ok(plan.args.some(x=>String(x).includes(m.proxy)), 'Plan must use the real proxy');
  report.cases.push({label,proxy:proxies,sequence_fps:p.fps,source_in:c.source_in,speed:c.speed,duration_frames:c.duration,source_span:c.retime?.source_span??null,predicted_seconds:predicted,decoded_seconds:actual,maximum_boundary_error_seconds:error,tolerance_seconds:tolerance,display_points:d.points.length,bin_visits:d.binVisits,easy_advanced_component_exact:true,render_sha256:sha(rendered),graph_sha256:sha(graph)});
  writeFileSync(join(out,label+'.monocut'),JSON.stringify(p,null,2));return viewCurves;
}
try{
  const base=join(out,'pulse-base.mkv'),offset=join(out,'pulse-offset.mkv');assert.ok(!existsSync(base),'Use a fresh output directory; evidence is immutable');
  // Short early impulse falls between bins sampled by the former every-tenth-bin shortcut.
  run(ffmpeg,['-v','error','-nostdin','-f','lavfi','-i','color=c=gray:s=160x90:r=30000/1001:d=16','-f','lavfi','-i','aevalsrc=0.7*sin(2*PI*440*t)*between(t\\,1.56\\,1.566)+0.5*sin(2*PI*660*t)*between(t\\,13.1\\,13.135):s=48000:d=16','-map','0:v','-map','1:a','-c:v','ffv1','-c:a','pcm_s16le',base]);
  run(ffmpeg,['-v','error','-nostdin','-copyts','-itsoffset','3','-i',base,'-itsoffset','3.2','-i',base,'-map','0:v','-map','1:a','-c','copy','-fps_mode','passthrough','-avoid_negative_ts','disabled',offset]);
  report.fixtures=[base,offset].map(p=>({file:p.split(/[\\/]/).at(-1),sha256:sha(p)}));
  const template=JSON.parse(readFileSync(templateArg));
  for(const mixed of [false,true]){
    const p=structuredClone(template),m=structuredClone(p.media[0]);m.id='pulse';m.name='Synthetic pulses';m.path=offset;m.duration=r(81,5);m.timing=null;m.proxy=null;m.thumbnail=null;m.waveform=[];delete m.legacy_source;delete m.proxy_timing_version;
    Object.assign(p,{id:'stage6-'+mixed,name:'Waveform file validation',fps:mixed?r(30000,1001):r(30),width:160,height:90,media:[m],clips:[],bins:[],markers:[],in_point:null,out_point:null});p.tracks=p.tracks.filter(t=>t.id==='A1'||t.id==='V1');const projectPath=join(out,'seed-'+mixed+'.monocut');writeFileSync(projectPath,JSON.stringify(p));let state=await open(projectPath);const imported=state.media[0];assert.equal(imported.waveform.length,1024);assert.equal(v(imported.timing.origin),3);assert.equal(v(imported.timing.audio_start)-3,.20000000000000018);
    const source=decode(offset,`asetpts=PTS-(${imported.timing.origin.num}/${imported.timing.origin.den})/TB,aresample=4000:first_pts=0,atrim=duration=${v(imported.duration).toFixed(12)}`,4000,true),expected=Math.ceil(v(imported.duration)*4000),peaks=Array(1024).fill(0);source.forEach((value,i)=>{const j=Math.min(1023,Math.floor(i*1024/expected));peaks[j]=Math.max(peaks[j],Math.min(1,Math.abs(value)));});assert.deepEqual(imported.waveform.map(Math.fround),peaks.map(Math.fround));
    if(!mixed){const pulseBins=peaks.map((x,i)=>x>.05&&i).filter(x=>x!==false&&x<200);assert.ok(pulseBins.length&&pulseBins.every(i=>i%10!==0));report.source_waveform={bins:1024,decoded_samples:source.length,exact_f32_bin_maxima:true,audio_padding_seconds:.2,early_impulse_bins:pulseBins,old_sampling_would_drop_impulse:true};}
    state=await rpc('edit',{command:{type:'add_clip',media_id:'pulse',track_id:'A1',start:0,source_in:r(0),duration:480}});let id=state.clips[0].id;await check('full-'+mixed,state);
    const initial=curves(state);state=await rpc('edit',{command:{type:'trim',id,edge:'in',frame:360}});await check('tail-'+mixed,state);state=await rpc('edit',{command:{type:'move',ids:[id],delta:-360,track_id:null}});
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(2)}});await check('tail-fast-'+mixed,state);const fast=curves(state);await rpc('undo');state=await rpc('redo');assert.deepEqual(curves(state),fast);
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(1,2)}});await check('tail-slow-'+mixed,state);
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(7,6)}});await check('tail-rational-'+mixed,state);
    if(!mixed){const before=curves(state);const job=await rpc('proxy',{mediaId:'pulse'});let done;for(let i=0;i<300;i++){done=(await rpc('list')).find(x=>x.id===job.id);if(done.status==='complete')break;assert.ok(!['failed','cancelled'].includes(done.status));await new Promise(r=>setTimeout(r,100));}assert.equal(done.status,'complete');state=await rpc('set_proxy',{mediaId:'pulse',path:done.path});assert.deepEqual(curves(state),before);await check('tail-rational-proxy',state,true);report.proxy_source_curve_exact=true;}
    const saved=join(out,'saved-'+mixed+'.monocut'),before=curves(state);await rpc('save',{path:saved});await close();state=await open(saved);assert.deepEqual(curves(state),before);report.cases.at(-1).fresh_process_save_reopen_curve_exact=true;
    state=await rpc('load_project',{path:projectPath});state=await rpc('edit',{command:{type:'add_clip',media_id:'pulse',track_id:'A1',start:0,source_in:r(0),duration:120}});id=state.clips[0].id;state=await rpc('edit',{command:{type:'slip',id,delta:360}});await check('slip-'+mixed,state);
    state=await rpc('edit',{command:{type:'split',ids:[id],frame:30}});assert.equal(state.clips.length,2);const split=curves(state);state=await rpc('undo');state=await rpc('redo');assert.deepEqual(curves(state),split);await check('split-first-'+mixed,state,false,0);await check('split-second-'+mixed,state,false,1);report.cases.at(-1).split_undo_redo_curve_exact=true;
    state=await rpc('load_project',{path:projectPath});state=await rpc('edit',{command:{type:'add_clip',media_id:'pulse',track_id:'A1',start:0,source_in:r(0),duration:480}});id=state.clips[0].id;
    state=await rpc('edit',{command:{type:'trim',id,edge:'out',frame:90}});await check('trim-out-'+mixed,state);
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(2)}});await check('padding-fast-'+mixed,state);
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(1,2)}});await check('padding-slow-'+mixed,state);
    state=await rpc('load_project',{path:projectPath});state=await rpc('edit',{command:{type:'add_clip',media_id:'pulse',track_id:'A1',start:0,source_in:r(7,13),duration:120}});id=state.clips[0].id;
    state=await rpc('edit',{command:{type:'retime_clip',id,speed:r(7,6)}});await check('fractional-source-'+mixed,state);
    const telemetry=await rpc('telemetry');assert.equal(telemetry.active_managed_children,0);await close();
  }
  report.native_after=nativeHashes();assert.deepEqual(report.native_before,report.native_after);report.frontend_hashes=Object.fromEntries(['src/Timeline.tsx','src/timelineWaveform.ts','tests/timeline-harness.mjs'].map(n=>[n,sha(join(repo,n))]));report.verified=true;
}catch(error){report.failures.push(error.stack);process.exitCode=1;}finally{try{await close();}catch(e){report.failures.push(String(e));process.exitCode=1;}writeFileSync(join(out,'raw-results.json'),JSON.stringify(report,null,2));}
console.log(JSON.stringify({verified:report.verified,cases:report.cases.length,failures:report.failures}));

