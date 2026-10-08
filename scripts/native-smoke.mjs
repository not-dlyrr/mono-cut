// SPDX-License-Identifier: GPL-3.0-or-later
// Run against the actual Tauri WebView2 process, never a browser substitute.
import { chromium } from 'playwright-core';
import { mkdirSync, readFileSync, writeFileSync, existsSync, renameSync } from 'node:fs';
import { resolve, join } from 'node:path';
import assert from 'node:assert/strict';
const [mediaArg, outputArg, address='http://127.0.0.1:9335'] = process.argv.slice(2);
if(!mediaArg || !outputArg) throw new Error('Usage: node scripts/native-smoke.mjs MEDIA_DIR OUTPUT_DIR [CDP_URL]');
const mediaDir=resolve(mediaArg), outputDir=resolve(outputArg); mkdirSync(outputDir,{recursive:true});
const browser=await chromium.connectOverCDP(address);
const context=browser.contexts()[0];
const page=context.pages().find(p=>/tauri|127\.0\.0\.1:1420/.test(p.url())) ?? context.pages()[0];
assert(page,'No native WebView2 page');
const mutePlayback=()=>{const mute=()=>document.querySelectorAll('video,audio').forEach(v=>{v.defaultMuted=true;v.muted=true;v.volume=0;});document.addEventListener('play',mute,true);document.addEventListener('volumechange',mute,true);new MutationObserver(mute).observe(document,{childList:true,subtree:true});mute();};
await page.addInitScript(mutePlayback);await page.evaluate(mutePlayback);
const errors=[]; page.on('pageerror',e=>errors.push(e.message));
await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
if(await page.getByRole('button',{name:'Skip tutorial',exact:true}).count()) await page.getByRole('button',{name:'Skip tutorial',exact:true}).click();
if(await page.getByRole('button',{name:'Keep current project'}).count()) await page.getByRole('button',{name:'Keep current project'}).click();
const invoke=(command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
const timings={};
async function timed(name,fn){const t=performance.now();const result=await fn();timings[name]=Math.round(performance.now()-t);return result;}
async function waitJob(job,limit=180000){
  const end=Date.now()+limit;
  while(Date.now()<end){const j=(await invoke('get_jobs')).find(x=>x.id===job.id);if(j && j.status!=='running'){if(j.status==='failed')throw new Error(j.error);return j;}await new Promise(r=>setTimeout(r,100));}
  throw new Error(`Timed out ${job.kind}`);
}
let p=await invoke('new_project',{name:'Mono Cut Validation',width:640,height:360,fps:{num:30,den:1}});
const files=['camera-a.mp4','camera-b.mp4','tone.wav','card.png'].map(f=>join(mediaDir,f));
p=await timed('import_ms',()=>invoke('import_media',{paths:files}));assert.equal(p.media.length,4);
assert(p.media.filter(m=>m.has_audio).every(m=>m.waveform.length>0),'Real waveforms');
assert(p.media.filter(m=>m.kind!=='audio').every(m=>m.thumbnail && existsSync(m.thumbnail)),'Real thumbnails');
const a=p.media.find(m=>m.name==='camera-a.mp4'),b=p.media.find(m=>m.name==='camera-b.mp4'),tone=p.media.find(m=>m.kind==='audio'),card=p.media.find(m=>m.kind==='image');
const video=p.tracks.find(t=>t.kind==='video').id,audio=p.tracks.find(t=>t.kind==='audio').id;
const edit=command=>invoke('edit',{command});
p=await edit({type:'add_bin',name:'Footage'});const bin=p.bins[0].id;
p=await edit({type:'assign_bin',media_id:a.id,bin_id:bin});
p=await edit({type:'add_clip',media_id:a.id,track_id:video,start:0,duration:150});const ca=p.clips.at(-1).id;
p=await edit({type:'add_clip',media_id:b.id,track_id:video,start:120,duration:90,source_in:{num:1,den:1}});const cb=p.clips.at(-1).id;
p=await edit({type:'update_clip',id:ca,patch:{fade_out:30}});
p=await edit({type:'update_clip',id:cb,patch:{fade_in:30,brightness:0.03,saturation:0.8}});
p=await edit({type:'add_clip',media_id:tone.id,track_id:audio,start:0,duration:210});const ct=p.clips.at(-1).id;
p=await edit({type:'update_clip',id:ct,patch:{volume:0.25,fade_in:15,fade_out:30}});
p=await edit({type:'add_track',kind:'video',name:'Titles'});const upper=p.tracks.at(-1).id;
p=await edit({type:'add_title',track_id:upper,start:15,duration:45,text:'MONO CUT'});
p=await edit({type:'marker',frame:45,name:'Title'});
const original=JSON.stringify(p.clips);
p=await edit({type:'move',ids:[cb],delta:15});assert.equal(p.clips.find(c=>c.id===cb).start,135);
p=await invoke('undo');assert.equal(JSON.stringify(p.clips),original);
p=await invoke('redo');assert.equal(p.clips.find(c=>c.id===cb).start,135);
p=await invoke('undo');
p=await edit({type:'split',ids:[ca],frame:60});assert.equal(p.clips.filter(c=>c.media_id===a.id).length,2);
assert.equal(p.clips.find(c=>c.media_id===a.id && c.start===60).source_in.num/p.clips.find(c=>c.media_id===a.id && c.start===60).source_in.den,2);
p=await invoke('undo');
p=await edit({type:'trim',id:ca,edge:'out',frame:120});assert.equal(p.clips.find(c=>c.id===ca).duration,120);p=await invoke('undo');
p=await edit({type:'slip',id:cb,delta:15});assert.equal(p.clips.find(c=>c.id===cb).source_in.num/p.clips.find(c=>c.id===cb).source_in.den,1.5);p=await invoke('undo');
p=await edit({type:'link',ids:[ca,ct]});assert(p.clips.find(c=>c.id===ca).linked_id);p=await edit({type:'unlink',ids:[ca,ct]});const expectedClips=p.clips.length;
const projectFile=join(outputDir,'validation.monocut');await invoke('save_project',{path:projectFile});
const saved=JSON.parse(readFileSync(projectFile,'utf8'));assert(saved.media.every(m=>!/^([A-Za-z]:|\\\\)/.test(m.path)),'Relative project source paths');
p=await timed('reopen_ms',()=>invoke('open_project',{path:projectFile}));assert.equal(p.clips.length,expectedClips);assert.equal(p.bins.length,1);
const renamed=join(mediaDir,'camera-b-relinked.mp4');renameSync(files[1],renamed);
try{p=await invoke('open_project',{path:projectFile});assert(p.media.find(m=>m.id===b.id).missing);p=await edit({type:'relink',media_id:b.id,path:renamed});assert(!p.media.find(m=>m.id===b.id).missing);}finally{renameSync(renamed,files[1]);}
p=await edit({type:'relink',media_id:b.id,path:files[1]});
const proxy=await timed('proxy_ms',async()=>waitJob(await invoke('generate_proxy',{mediaId:a.id})));assert.equal(proxy.status,'complete');
p=await invoke('get_project');assert(p.media.find(m=>m.id===a.id).proxy);
const source=await timed('source_prepare_ms',()=>invoke('prepare_source',{mediaId:a.id}));assert(existsSync(source));
// Proxy attachment notifies React and schedules its automatic preview. Let that
// finish before exercising manual preview commands, which supersede older jobs.
await page.waitForTimeout(700);
await page.waitForFunction(()=>document.querySelector('.preview-state')?.textContent.includes('Preview ready'),null,{timeout:120000});
const preview=await timed('preview_ms',async()=>waitJob(await invoke('render_preview',{height:360,useProxies:false})));
assert.equal(preview.status,'complete');
const previewProxy=await timed('proxy_preview_ms',async()=>waitJob(await invoke('render_preview',{height:360,useProxies:true})));assert.equal(previewProxy.status,'complete');
const exportPath=join(outputDir,'validation.mp4');
const settings={width:640,height:360,fps:{num:30,den:1},codec:'h264',crf:18,audio_bitrate:192,sample_rate:48000};
const exported=await timed('export_ms',async()=>waitJob(await invoke('export_project',{path:exportPath,settings})));assert.equal(exported.status,'complete');
const cancellationPath=join(outputDir,'cancelled.mp4');const cancel=await invoke('export_project',{path:cancellationPath,settings:{...settings,width:3840,height:2160,crf:10}});await invoke('cancel_job',{id:cancel.id});assert.equal((await waitJob(cancel)).status,'cancelled');assert(!existsSync(cancellationPath),'Cancelled jobs never finalize');
await invoke('save_project',{path:projectFile});await page.reload();await page.waitForSelector('.timeline-clip');
if(await page.getByRole('dialog').count()) {const dismiss=page.getByRole('button',{name:'Keep current project'});if(await dismiss.count())await dismiss.first().click();else await page.keyboard.press('Escape');}
if(await page.getByRole('button',{name:'Skip tutorial',exact:true}).count()) await page.getByRole('button',{name:'Skip tutorial',exact:true}).click();
await page.getByRole('button',{name:/camera-a.mp4, starts/}).first().click();
const program=page.locator('video').last();
await program.waitFor();await page.waitForFunction(()=>[...document.querySelectorAll('video')].some(v=>v.readyState>=2),null,{timeout:120000});
await page.waitForTimeout(300);
await page.screenshot({path:join(outputDir,'mono-cut-dark.png')});
const playback=await timed('playback_measurement_ms',async()=>{
 return page.evaluate(async()=>{const v=[...document.querySelectorAll('video')].at(-1);v.muted=true;v.currentTime=0;await v.play();const before=v.getVideoPlaybackQuality();await new Promise(r=>setTimeout(r,1800));v.pause();const after=v.getVideoPlaybackQuality();return {played_seconds:v.currentTime,total_frames:after.totalVideoFrames-before.totalVideoFrames,dropped_frames:after.droppedVideoFrames-before.droppedVideoFrames};});
});
assert(playback.played_seconds>1,'Native synchronized program playback advances');
const scrub=await page.evaluate(async()=>{const v=[...document.querySelectorAll('video')].at(-1),samples=[];for(const t of [0.2,1.8,3.5,6]){const start=performance.now();await new Promise(r=>{v.addEventListener('seeked',r,{once:true});v.currentTime=t;});samples.push(Math.round(performance.now()-start));}return samples;});
const report={native:true,muted:true,verified:['actual import metadata thumbnails waveforms','bins','multitrack layered timeline','cross dissolve fades','title','color adjustment','mixed source fps','move split trim slip link unlink','undo redo','relative save reopen','missing media relink','proxy generation and switching','shared preview/export graph','cancel export deletes temporary output','native program playback (muted; exported audio timing tested separately)'],timings_ms:timings,playback,scrub_seek_ms:scrub,errors,preview:preview.path,proxy_preview:previewProxy.path,export:exportPath};
writeFileSync(join(outputDir,'native-validation.json'),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({...report,preview:'local cache',proxy_preview:'local cache',export:'validation.mp4'},null,2));
assert.equal(errors.length,0,'No native UI JavaScript errors');
await browser.close();
