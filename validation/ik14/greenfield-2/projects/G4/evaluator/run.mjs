import {spawn} from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
const projectEnv=process.env.PROJECT_DIR,stage=process.env.EVAL_STAGE||'initial';if(!projectEnv)throw Error('PROJECT_DIR required');if(!['initial','change'].includes(stage))throw Error('EVAL_STAGE must be initial or change');const project=path.resolve(projectEnv);
const ids=stage==='initial'?['R1','R2','R3','R4','R5','R6','R7','R8']:['R1','R2','R3','R4','R5','R6','R7','R8','C1','C2'];
const COMMAND_TIMEOUT_MS=20_000,GLOBAL_WATCHDOG_MS=200_000;
const activeFixtures=new Set();
const request=(url,method='GET',data)=>new Promise((resolve,reject)=>{const u=new URL(url),q=http.request(u,{method,headers:data?{'content-type':'application/json'}:{}},r=>{let b='';r.on('data',x=>b+=x);r.on('end',()=>{try{resolve({status:r.statusCode,body:JSON.parse(b)})}catch(e){reject(e)}})});q.setTimeout(2500,()=>q.destroy(Error('fixture request timed out')));q.on('error',reject);q.end(data?JSON.stringify(data):undefined)});
const signalGroup=(pid,signal)=>{if(!pid)return;try{process.kill(process.platform==='win32'?pid:-pid,signal)}catch{}};
async function fixture(fn){
 const dir=await fs.mkdtemp(path.join(os.tmpdir(),'ik14-g4-')),ready=path.join(dir,'ready.json'),fixtureDir=process.env.STUB_DIR?path.resolve(process.env.STUB_DIR):path.join(path.dirname(new URL(import.meta.url).pathname),'fixture');
 const srv=spawn(process.execPath,[path.join(fixtureDir,'server.mjs'),ready],{stdio:'ignore',detached:true}),srvClosed=new Promise(resolve=>srv.once('close',resolve));
 let info,ctl,cleanupPromise;const activeCommands=new Set();
 const cleanup=()=>cleanupPromise??=(async()=>{
  for(const c of activeCommands)c.stop();
  if(ctl)await request(ctl+'/control/config','POST',{hold:false,deny:0,drop:0,conflict:false}).catch(()=>{});
  await Promise.race([Promise.allSettled([...activeCommands].map(c=>c.done)),sleep(1500)]);
  signalGroup(srv.pid,'SIGTERM');await Promise.race([srvClosed,sleep(500)]);signalGroup(srv.pid,'SIGKILL');
  await fs.rm(dir,{recursive:true,force:true});
 })();
 activeFixtures.add(cleanup);
 try{
  for(let i=0;i<150;i++){try{info=JSON.parse(await fs.readFile(ready,'utf8'));break}catch{}await sleep(20)}
  if(!info)throw Error('fault service failed to start');
  const base=`http://127.0.0.1:${info.port}`;ctl=`http://127.0.0.1:${info.controlPort}`;
  const env={...process.env,STATE_DIR:path.join(dir,'state'),SERVICE_URL:base,EVAL_STAGE:stage};
  const config=async value=>{const r=await request(ctl+'/control/config','POST',value);return {code:r.status===200?0:1,json:r.body}};
  const state=()=>request(ctl+'/control/state');
  const cmd=(args,extra={})=>{
   const argv=[path.join(project,'run'),...args.map(String)];let record={};
   const done=new Promise(resolve=>{
    const p=spawn(argv[0],argv.slice(1),{cwd:project,env:{...env,...extra},stdio:['ignore','pipe','pipe'],detached:true});
    let out='',err='',timedOut=false,finished=false,escalation;
    const finish=result=>{if(finished)return;finished=true;clearTimeout(timeout);clearTimeout(escalation);activeCommands.delete(record);resolve(result)};
    record.stop=()=>{if(finished)return;signalGroup(p.pid,'SIGTERM');clearTimeout(escalation);escalation=setTimeout(()=>signalGroup(p.pid,'SIGKILL'),500)};
    const timeout=setTimeout(async()=>{if(finished)return;timedOut=true;try{await request(ctl+'/control/config','POST',{hold:false})}catch{}record.stop()},COMMAND_TIMEOUT_MS);
    p.stdout.on('data',x=>out+=x);p.stderr.on('data',x=>err+=x);
    p.on('error',e=>finish({code:1,out,err:String(e),argv}));
    p.on('close',code=>{let result;try{result=JSON.parse(out)}catch{}finish(timedOut?{code:1,json:result,out,err,timedOut:true,argv}:{code:code??1,json:result,out,err,argv})});
   });
   record.done=done;activeCommands.add(record);return done;
  };
  await fn({cmd,config,state,env,base});
 }finally{await cleanup();activeFixtures.delete(cleanup)}
}
const checked=r=>{if(r.timedOut)throw Error(`command timed out: ${r.argv.join(' ')}`);return r};
const ok=r=>{r=checked(r);if(r.code!==0||!r.json)throw Error(`expected success: ${r.out} ${r.err}`);return r.json},bad=(r,reason)=>{r=checked(r);if(r.code===0)throw Error(`expected refusal: ${r.out}`);if(reason&&r.json?.error!==reason)throw Error(`expected ${reason}, got ${r.out}`)},eq=(a,b)=>{if(JSON.stringify(a)!==JSON.stringify(b))throw Error(`expected ${JSON.stringify(b)}, got ${JSON.stringify(a)}`)},sleep=n=>new Promise(r=>setTimeout(r,n));
const cases={
R1:async f=>{const a=ok(await f.cmd(['reserve','r1','m1','i1']));if(a.phase!=='queued'||a.attempts!==0||a.operation_id!=='r1')throw Error('reserve facts wrong');ok(await f.cmd(['status','r1']));bad(await f.cmd(['reserve','r1','m1','i1']),'already_exists');const s=(await f.state()).body;if(s.requests.length)throw Error('reserve/status/duplicate called service');},
R2:async f=>{bad(await f.cmd(['reserve','bad!','m','i']),'invalid');bad(await f.cmd(['status','missing']),'not_found');bad(await f.cmd(['bogus']),'unknown_command');bad(await f.cmd(['reserve','x','m','i'],{STATE_DIR:''}),'invalid');bad(await f.cmd(['advance','missing'],{SERVICE_URL:''}),'invalid');if((await f.state()).body.requests.length)throw Error('invalid request called service');},
R3:async f=>{ok(await f.cmd(['reserve','r3','m3','i3']));const a=ok(await f.cmd(['advance','r3'])),b=ok(await f.cmd(['advance','r3']));if(a.phase!=='reserved'||a.equipment_id!=='equipment-i3'||b.phase!=='reserved')throw Error('grant mismatch');const s=(await f.state()).body;if(s.records.length!==1||s.requests.filter(x=>x.method==='POST').length!==1)throw Error('allocation duplicated');},
R4:async f=>{ok(await f.config({deny:3}));ok(await f.cmd(['reserve','r4','m4','i4']));for(let n=1;n<=3;n++){const r=ok(await f.cmd(['advance','r4']));if(r.attempts!==n||r.phase!==(n===3?'rejected':'queued'))throw Error(`denial attempt ${n} mismatch`);if(n===1){const restarted=ok(await f.cmd(['status','r4']));if(restarted.attempts!==1)throw Error('denial count not durable')}}const before=(await f.state()).body.requests.length;ok(await f.cmd(['advance','r4']));if((await f.state()).body.requests.length!==before)throw Error('terminal rejection contacted service');},
R5:async f=>{ok(await f.cmd(['reserve','r5','m5','i5']));ok(await f.config({drop:1}));const first=ok(await f.cmd(['advance','r5']));if(first.phase!=='uncertain'||first.attempts!==0)throw Error('lost response not uncertain');const second=ok(await f.cmd(['advance','r5']));if(second.phase!=='reserved'||second.equipment_id!=='equipment-i5')throw Error('recovery failed');const s=(await f.state()).body;if(s.requests.filter(x=>x.method==='GET'&&x.operation_id==='r5').length<1||s.requests.filter(x=>x.method==='POST').length!==1||s.records.length!==1)throw Error('recovery did not lookup same id before retry');},
R6:async f=>{ok(await f.cmd(['reserve','r6','m6','i6']));ok(await f.config({hold:true}));const advancing=f.cmd(['advance','r6']);let sent=false;for(let i=0;i<100;i++){if((await f.state()).body.requests.some(x=>x.operation_id==='r6'&&x.method==='POST')){sent=true;break}await sleep(10)}if(!sent)throw Error('grant not held');const c=ok(await f.cmd(['cancel','r6']));if(c.phase!=='cancelled')throw Error('cancel not durable');await f.config({hold:false});const late=ok(await advancing);if(late.phase!=='cancelled'||late.equipment_id!==null)throw Error('late grant changed cancellation');const n=(await f.state()).body.requests.length;ok(await f.cmd(['advance','r6']));if((await f.state()).body.requests.length!==n)throw Error('terminal cancel contacted service');},
R7:async f=>{ok(await f.cmd(['reserve','r7','m7','i7']));ok(await f.config({hold:true}));const advancing=f.cmd(['advance','r7']);let dispatched=false;for(let i=0;i<100;i++){if((await f.state()).body.requests.some(x=>x.method==='POST')){dispatched=true;break}await sleep(10)}if(!dispatched)throw Error('request not held in flight');ok(await f.cmd(['status','r7']));const c=ok(await f.cmd(['cancel','r7']));if(c.phase!=='cancelled')throw Error('cancel while pending failed');await f.config({hold:false});const late=ok(await advancing);if(late.phase!=='cancelled'||late.equipment_id!==null)throw Error('late response overwrote cancellation');const final=ok(await f.cmd(['status','r7']));if(final.phase!=='cancelled'||final.equipment_id!==null)throw Error('status lost concurrent cancellation');},
R8:async f=>{ok(await f.cmd(['reserve','r8','m8','i8']));ok(await f.config({conflict:true}));const r=ok(await f.cmd(['advance','r8']));if(r.phase!=='rejected'||r.equipment_id!==null)throw Error('conflict not terminal rejected');const s=(await f.state()).body;if(s.records.length!==0||s.requests.some(x=>x.operation_id!=='r8'))throw Error('conflict allocated/rekeyed');ok(await f.cmd(['reserve','r8good','m','item']));if(ok(await f.cmd(['advance','r8good'])).phase!=='reserved')throw Error('legitimate grant refused');},
C1:async f=>{ok(await f.cmd(['reserve','c1','m','i']));const h=ok(await f.cmd(['hold','c1','inspection']));if(h.phase!=='hold'||h.hold_reason!=='inspection')throw Error('hold reason/phase missing');bad(await f.cmd(['advance','c1']),'held');if((await f.state()).body.requests.length)throw Error('held item contacted service');ok(await f.cmd(['cancel','c1']));if(ok(await f.cmd(['status','c1'])).phase!=='cancelled')throw Error('held cancellation failed');},
C2:async f=>{ok(await f.cmd(['reserve','c2','m','i']));bad(await f.cmd(['hold','c2','   ']),'invalid');ok(await f.cmd(['advance','c2']));bad(await f.cmd(['hold','c2','too-late']),'already_dispatched');},
};
const results=[];
const watchdog=setTimeout(async()=>{
 const timedOut=[...results,...ids.slice(results.length).map(requirement=>({requirement,passed:false,error:'evaluator watchdog'}))];
 await Promise.race([Promise.allSettled([...activeFixtures].map(cleanup=>cleanup())),sleep(3000)]);
 process.stdout.write(JSON.stringify({stage,results:timedOut,passed:false})+'\n',()=>process.exit(1));
},GLOBAL_WATCHDOG_MS);
for(const id of ids){try{await fixture(cases[id]);results.push({requirement:id,passed:true})}catch(e){results.push({requirement:id,passed:false,error:String(e.message||e)})}}
clearTimeout(watchdog);
const passed=results.every(x=>x.passed);console.log(JSON.stringify({stage,results,passed}));process.exitCode=passed?0:1;
