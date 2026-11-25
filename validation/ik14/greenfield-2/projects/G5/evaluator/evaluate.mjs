import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const dir = process.env.PROJECT_DIR;
const scope = process.argv[2];
const change = scope === 'change' || scope === 'change/project';
const selected = process.env.EVAL_CASE || 'all';
const requirements = ['R1','R2','R3','R4','R5','R6','R7','R8', ...(change ? ['C1'] : [])].filter(x=>selected==='all'||x===selected);
const results = Object.fromEntries(requirements.map(x => [x, false]));
const tmp = await mkdtemp(path.join(os.tmpdir(), 'ik14-g5-'));
let service;
let serviceUrl;
let serviceCalls = [];
let dropTask = null, holdTask = null, releaseHeld = null;
const tasks = new Map();
service = (await import('node:http')).createServer(async (req,res) => {
  const chunks=[]; for await (const x of req) chunks.push(x);
  const send=(code,obj)=>{res.writeHead(code,{'content-type':'application/json'});res.end(JSON.stringify(obj));};
  if(req.method==='POST' && req.url==='/tasks') { let x;try{x=JSON.parse(Buffer.concat(chunks));}catch{return send(400,{error:'invalid JSON'});}if(typeof x.task_id!=='string'||!x.task_id)return send(400,{error:'task_id required'});serviceCalls.push(x.task_id);if(!tasks.has(x.task_id))tasks.set(x.task_id,{task_id:x.task_id,outcome:'confirmed'});if(dropTask===x.task_id){dropTask=null;res.destroy();return;}if(holdTask===x.task_id)await new Promise(resolve=>{releaseHeld=resolve;});return send(200,tasks.get(x.task_id));}
  if(req.method==='GET' && req.url.startsWith('/tasks/')) { const id=decodeURIComponent(req.url.slice(7)); serviceCalls.push('lookup:'+id); return tasks.has(id)?send(200,tasks.get(id)):send(404,{error:'not found'}); }
  send(404,{error:'not found'});
});
await new Promise(r=>service.listen(0,'127.0.0.1',r)); serviceUrl=`http://127.0.0.1:${service.address().port}`;
const state=path.join(tmp,'state');
const timedOutCommands=[],active=new Set();
async function run(...args) {
  if(timedOutCommands.length)throw Error(`command timed out: ${timedOutCommands.join('; ')}`);
  let extra={};if(args[0]==='__env'){extra=args[1];args=args.slice(2);}
  const command=args.join(' '),p=spawn('./run',args,{cwd:dir,env:{...process.env,STATE_DIR:state,SERVICE_URL:serviceUrl,...extra},stdio:['ignore','pipe','pipe'],detached:true});active.add(p);
  let out='',err='',timedOut=false;
  p.stdout.on('data',x=>out+=x);p.stderr.on('data',x=>err+=x);
  const timer=setTimeout(()=>{timedOut=true;timedOutCommands.push(command);try{process.kill(-p.pid,'SIGKILL');}catch{}try{p.kill('SIGKILL');}catch{};holdTask=null;const release=releaseHeld;releaseHeld=null;release?.();},20000);
  const code=await new Promise((resolve,reject)=>{p.on('error',e=>{clearTimeout(timer);active.delete(p);reject(e);});p.on('close',code=>{clearTimeout(timer);active.delete(p);resolve(code);});});
  let json;try{json=JSON.parse(out);}catch{}
  return {code:timedOut?null:code,timedOut,json:timedOut?undefined:json,out,err,args};
}
const ok=(x)=>x.code===0&&x.json;
const record=(x)=>x;
const assert=(v,id)=>{if(timedOutCommands.length)throw Error(`command timed out: ${timedOutCommands.join('; ')}`);if(!v)throw Error(id);};
async function verify(id,fn){if(selected!=='all'&&selected!==id)return;try{await fn();results[id]=true;}catch(e){const message=timedOutCommands.length?`command timed out: ${timedOutCommands.join('; ')}`:e.message;timedOutCommands.length=0;console.error(`${id}: ${message}`);}}
const watchdog=setTimeout(()=>{console.error('evaluator watchdog');for(const p of active){try{process.kill(-p.pid,'SIGKILL');}catch{}try{p.kill('SIGKILL');}catch{}}holdTask=null;const release=releaseHeld;releaseHeld=null;release?.();process.stdout.write(JSON.stringify({requirements:results})+'\n',()=>process.exit(1));},200000);
try {
  await verify('R1',async()=>{serviceCalls=[];let x=await run('schedule','p1','s1','annual',2);assert(ok(x),'schedule accepted');x=await run('status','p1');const r=record(x.json);assert(ok(x)&&r.plan_id==='p1'&&r.count===2&&r.next_index===1&&r.phase==='ready','schedule status');x=await run('schedule','p1','s1','annual',2);assert(x.code!==0&&typeof x.json?.error==='string','duplicate refused');});
  await verify('R2',async()=>{serviceCalls=[];for(const args of [['schedule','bad id','s','t','1'],['nonsense'],['status','missing'],['advance','missing']]){const x=await run(...args);assert(x.code!==0&&typeof x.json?.error==='string','invalid refusal');}const missing=await run('__env',{STATE_DIR:'',SERVICE_URL:''},'status','missing-env');assert(missing.code!==0&&typeof missing.json?.error==='string','missing environment refusal');assert(serviceCalls.length===0,'invalid requests made no service call');let x=await run('schedule','pvalid','s','t',1);assert(ok(x),'valid schedule command accepted');x=await run('status','pvalid');const r=record(x.json);assert(ok(x)&&r.plan_id==='pvalid'&&r.phase==='ready'&&r.next_index===1,'valid schedule effect visible');});
  await verify('R3',async()=>{await run('schedule','p3','s','annual',2);serviceCalls=[];let x=await run('advance','p3');assert(ok(x),'advance one');x=await run('status','p3');let r=record(x.json);assert(r.completed?.includes('p3:1')&&r.next_index===2,'first occurrence recorded');await run('advance','p3');await run('advance','p3');x=await run('status','p3');r=record(x.json);assert(r.phase==='complete'&&r.completed?.length===2&&r.next_index===3,'completed finite schedule');assert(serviceCalls.filter(y=>y==='p3:1').length===1&&serviceCalls.filter(y=>y==='p3:2').length===1&&!serviceCalls.includes('p3:3'),'at most once');});
  await verify('R4',async()=>{await run('schedule','pu','s','t',1);dropTask='pu:1';serviceCalls=[];await run('advance','pu');let x=await run('status','pu'),r=record(x.json);assert(r.phase==='uncertain'&&r.next_index===1&&!r.completed?.includes('pu:1'),'lost reply remains visibly uncertain without confirmed completion');serviceCalls=[];x=await run('advance','pu');assert(ok(x),'recovery after lost reply');assert(serviceCalls.includes('lookup:pu:1')&&!serviceCalls.includes('pu:1'),'recovery lookup before resend');});
  await verify('R5',async()=>{const crashState=path.join(tmp,'crash');let x=await run('__env',{STATE_DIR:crashState,CRASH_AT:'after_schedule'},'schedule','pcr','s','t',1);assert(x.code!==0,'after_schedule must crash');x=await run('__env',{STATE_DIR:crashState},'status','pcr');assert(ok(x)&&record(x.json).next_index===1&&record(x.json).phase==='ready','after schedule restart');});
  await verify('R6',async()=>{const sentState=path.join(tmp,'sent');await run('__env',{STATE_DIR:sentState},'schedule','ps','s','t',2);let x=await run('__env',{STATE_DIR:sentState,CRASH_AT:'after_send'},'advance','ps');assert(x.code!==0,'after_send must crash');x=await run('__env',{STATE_DIR:sentState},'status','ps');assert(record(x.json).phase==='uncertain','after_send persisted uncertainty');serviceCalls=[];x=await run('__env',{STATE_DIR:sentState},'advance','ps');assert(ok(x)&&record(x.json).completed.includes('ps:1'),'after_send recovery');assert(serviceCalls.includes('lookup:ps:1')&&!serviceCalls.includes('ps:1'),'recovery reconciled same effect');});
  await verify('R7',async()=>{await run('schedule','pc','s','t',3);await run('advance','pc');holdTask='pc:2';const late=run('advance','pc');for(let i=0;i<100&&!serviceCalls.includes('pc:2');i++)await new Promise(resolve=>setTimeout(resolve,10));assert(serviceCalls.includes('pc:2'),'second occurrence reached service');const cancelled=await run('cancel','pc');assert(ok(cancelled)&&record(cancelled.json).phase==='cancelled','cancel during send');holdTask=null;releaseHeld?.();const result=await late;assert(!result.timedOut,'late command timed out');const x=await run('status','pc'),r=record(x.json);assert(r.phase==='cancelled'&&r.completed?.length===1&&r.completed[0]==='pc:1','cancel wins over late completion');serviceCalls=[];const later=await run('advance','pc');assert(ok(later)&&record(later.json).phase==='cancelled','advance after cancellation');assert(!serviceCalls.includes('pc:2')&&!serviceCalls.includes('pc:3'),'no later occurrence sent');});
  await verify('R8',async()=>{await run('schedule','pa','s','t',1);await run('schedule','pb','s','t',1);let x=await run('advance','pa');assert(ok(x),'first plan remains addressable');x=await run('status','pb');assert(ok(x)&&record(x.json).next_index===1,'independent plan unchanged');});
  if(change)await verify('C1',async()=>{await run('schedule','pp','s','t',2);let x=await run('pause','pp');assert(ok(x)&&record(x.json).phase==='paused','pause');serviceCalls=[];await run('advance','pp');assert(serviceCalls.length===0,'paused plan made no external dispatch');x=await run('status','pp');assert(record(x.json).phase==='paused'&&record(x.json).next_index===1,'paused dispatch blocked');x=await run('resume','pp');assert(ok(x)&&record(x.json).phase==='ready','resume');serviceCalls=[];holdTask='pp:1';const inFlight=run('advance','pp');for(let i=0;i<100&&!serviceCalls.includes('pp:1');i++)await new Promise(resolve=>setTimeout(resolve,10));assert(serviceCalls.includes('pp:1'),'in-flight task reached service');x=await run('pause','pp');assert(ok(x)&&record(x.json).phase==='paused'&&record(x.json).current_task_id==='pp:1','pause during send');holdTask=null;releaseHeld?.();x=await inFlight;const reconciled=record(x.json);assert(ok(x)&&reconciled.phase==='paused'&&reconciled.completed.length===1&&reconciled.completed[0]==='pp:1'&&reconciled.next_index===2&&reconciled.current_task_id===null,'reconcile in-flight task while paused');serviceCalls=[];x=await run('advance','pp');assert(ok(x)&&record(x.json).phase==='paused'&&serviceCalls.length===0,'paused advance after reconciliation');x=await run('status','pp');assert(record(x.json).phase==='paused'&&record(x.json).next_index===2,'paused schedule did not dispatch next occurrence');x=await run('resume','pp');assert(ok(x)&&record(x.json).phase==='ready','resume after paused reconciliation');x=await run('advance','pp');assert(ok(x)&&record(x.json).phase==='complete','resumed schedule progresses');});
} finally { clearTimeout(watchdog); holdTask=null; releaseHeld?.(); await new Promise(r=>service.close(r)); await rm(tmp,{recursive:true,force:true}); }
process.stdout.write(JSON.stringify({requirements:results})+'\n');
if(Object.values(results).some(x=>!x)) process.exitCode=1;
