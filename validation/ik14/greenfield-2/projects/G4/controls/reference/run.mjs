#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
const out=(x,ok=true)=>{process.stdout.write(JSON.stringify(x)+'\n');process.exit(ok?0:1)};
const fail=e=>out({error:e},false),valid=x=>typeof x==='string'&&/^[A-Za-z0-9_-]{1,64}$/.test(x);
const [cmd,id,a,b]=process.argv.slice(2),dir=process.env.STATE_DIR,svc=process.env.SERVICE_URL;
if(!dir){fail('invalid');process.exit(1)}
const file=path.join(dir,'state.json'),lock=file+'.lock';fs.mkdirSync(dir,{recursive:true});
function read(){try{return JSON.parse(fs.readFileSync(file,'utf8'))}catch{return {items:{}}}}
function tx(fn){let fd;for(let n=0;n<200;n++){try{fd=fs.openSync(lock,'wx');break}catch{Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,5)}}if(fd===undefined)throw Error('busy');try{const s=read(),r=fn(s);if(r?.error)return r;const tmp=file+'.'+process.pid+'.tmp';fs.writeFileSync(tmp,JSON.stringify(s));fs.renameSync(tmp,file);return r}finally{fs.closeSync(fd);fs.unlinkSync(lock)}}
function call(method,url,data){return new Promise((resolve,reject)=>{let u;try{u=new URL(url)}catch(e){return reject(e)}const q=http.request(u,{method,headers:data?{'content-type':'application/json'}:{}},r=>{let text='';r.on('data',c=>text+=c);r.on('end',()=>{try{resolve({status:r.statusCode,body:JSON.parse(text)})}catch(e){reject(e)}})});q.setTimeout(1500,()=>q.destroy(Error('timeout')));q.on('error',reject);q.end(data?JSON.stringify(data):undefined)})}
async function main(){
if(cmd==='reserve'){if((!valid(id)||!valid(a)||!valid(b))&&!(process.env.G4_DEFECT==='R2'&&id==='bad!'))return fail('invalid');const r=tx(s=>{if(s.items[id]){if(process.env.G4_DEFECT==='R1'&&id==='r1'){s.items[id].member_id=a;return {...s.items[id]}}return {error:'already_exists'}}s.items[id]={reservation_id:id,member_id:a,item_id:b,phase:'queued',attempts:0,operation_id:id,equipment_id:null,sent:false,outbox:null};return {...s.items[id]}});if(r.error)return fail(r.error);return out(r)}
if(cmd==='status'){if(!valid(id))return fail('invalid');const x=read().items[id];return x?out(x):fail('not_found')}
if(cmd==='cancel'){if(!valid(id))return fail('invalid');const r=tx(s=>{const x=s.items[id];if(!x)return {error:'not_found'};if(!(process.env.G4_DEFECT==='R6'&&id==='r6')&&!['reserved','rejected'].includes(x.phase))x.phase='cancelled';return {...x}});return r.error?fail(r.error):out(r)}
if(cmd==='hold'){if(process.env.EVAL_STAGE!=='change')return fail('unknown_command');if(!valid(id)||typeof a!=='string'||(!a.trim()&&!(process.env.G4_DEFECT==='C2'&&id==='c2')))return fail('invalid');const r=tx(s=>{const x=s.items[id];if(!x)return {error:'not_found'};if(x.sent)return {error:'already_dispatched'};if(['cancelled','reserved','rejected'].includes(x.phase))return {error:'invalid'};x.phase='hold';x.hold_reason=process.env.G4_DEFECT==='C1'&&id==='c1'?'':a;return {...x}});return r.error?fail(r.error):out(r)}
if(cmd==='advance'){if(!valid(id))return fail('invalid');if(!svc)return fail('invalid');let x=read().items[id];if(!x)return fail('not_found');if(process.env.G4_DEFECT==='R3'&&id==='r3')return fail('invalid');if(x.phase==='hold')return fail('held');if(['reserved','rejected','cancelled'].includes(x.phase))return out(x);
if(x.phase==='uncertain'&&!(process.env.G4_DEFECT==='R5'&&id==='r5')){try{const q=await call('GET',svc+'/operations/'+encodeURIComponent(id));if(q.status===200&&q.body.found){if(q.body.outcome==='granted')return finish(id,'reserved',q.body.equipment_id);if(q.body.outcome==='denied')return denial(id);if(q.body.outcome==='conflict')return finish(id,'rejected',null)}if(q.status!==200&&q.status!==404)return fail('uncertain')}catch{return fail('uncertain')}}
x=read().items[id];if(!x||x.phase==='cancelled')return x?out(x):fail('not_found');if(x.phase==='reserved'||x.phase==='rejected')return out(x);
tx(s=>{let z=s.items[id];if(z&&z.phase!=='cancelled'){z.sent=true;z.phase='uncertain';z.outbox={operation_id:id,status:'pending'}}return {ok:true}});
try{const q=await call('POST',svc+'/reserve',{operation_id:id,member_id:x.member_id,item_id:x.item_id});if(q.status===409){if(process.env.G4_DEFECT==='R8'&&id==='r8')return finish(id,'reserved',null);return finish(id,'rejected',null)}if(q.status!==200||!['granted','denied'].includes(q.body.outcome))return out(read().items[id]);if(q.body.outcome==='denied')return denial(id);return finish(id,'reserved',q.body.equipment_id)}catch{return out(read().items[id])}}
return fail(cmd? 'unknown_command':'invalid');
}
function finish(id,phase,equipment){const r=tx(s=>{const x=s.items[id];if(!x)return {error:'not_found'};if(x.phase!=='cancelled'||(process.env.G4_DEFECT==='R7'&&id==='r7')){x.phase=phase;x.equipment_id=phase==='reserved'?equipment:null;if(x.outbox)x.outbox.status=phase==='reserved'?'applied':'rejected'}return {...x}});return r.error?fail(r.error):out(r)}
function denial(id){const r=tx(s=>{const x=s.items[id];if(!x)return {error:'not_found'};if(x.phase==='cancelled')return {...x};if(!(process.env.G4_DEFECT==='R4'&&id==='r4'))x.attempts++;x.phase=x.attempts>=3?'rejected':'queued';if(x.outbox)x.outbox.status='denied';return {...x}});return r.error?fail(r.error):out(r)}
main().catch(()=>fail('invalid'));
