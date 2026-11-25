import http from 'node:http';
import fs from 'node:fs';
import {fork} from 'node:child_process';
import path from 'node:path';
const readyFile=process.argv[2];
if(!readyFile) throw new Error('usage: node server.mjs <ready-file>');
const effects=new Map(), requests=[]; let config={drop_next:0,delay_ms:0,hold:false}; const held=[];
const body=req=>new Promise((resolve,reject)=>{let b='';req.on('data',x=>b+=x);req.on('end',()=>resolve(b));req.on('error',reject)});
const json=(res,status,x)=>{const b=JSON.stringify(x);res.writeHead(status,{'content-type':'application/json','content-length':Buffer.byteLength(b)});res.end(b)};
const publicServer=http.createServer(async(req,res)=>{const u=new URL(req.url,'http://localhost'); if(req.method==='POST'&&u.pathname==='/v1/deliveries'){let x;try{x=JSON.parse(await body(req))}catch{return json(res,400,{error:'invalid'})} const id=x.delivery_id; requests.push({...x,method:'POST'}); let rec=effects.get(id); if(rec&&JSON.stringify(rec.input)!==JSON.stringify(x))return json(res,409,{error:'conflict'}); if(!rec){rec={input:x,result:{delivery_id:id,provider_id:'provider-'+id,delivered:true}};effects.set(id,rec)} const send=()=>json(res,200,rec.result); if(config.drop_next>0){config.drop_next--;res.destroy();return} if(config.hold){held.push(send);return} if(config.delay_ms)await new Promise(r=>setTimeout(r,config.delay_ms));return send()}
if(req.method==='GET'&&u.pathname.startsWith('/v1/deliveries/')){const id=decodeURIComponent(u.pathname.slice('/v1/deliveries/'.length));requests.push({method:'GET',delivery_id:id});const r=effects.get(id);return r?json(res,200,r.result):json(res,404,{error:'not_found'})} json(res,404,{error:'not_found'})});
const listen=s=>new Promise(r=>s.listen(0,'127.0.0.1',r));
await listen(publicServer);
const sidecar=fork(path.join(path.dirname(new URL(import.meta.url).pathname),'control-sidecar.mjs'),[],{stdio:['ignore','ignore','ignore','ipc']});
const sidecarReady=new Promise(resolve=>sidecar.on('message',m=>{if(m?.ready)resolve(m.port)}));
sidecar.on('message',m=>{if(!m?.action)return;let value={ok:true};if(m.action==='state')value={requests,effects:[...effects.values()].map(x=>x.input)};else if(m.action==='reset'){effects.clear();requests.length=0;held.length=0;config={drop_next:0,delay_ms:0,hold:false}}else if(m.action==='config'){config={...config,...m.value};if(!config.hold)for(const f of held.splice(0))f()}else if(m.action==='release'){const count=held.length;for(const f of held.splice(0))f();value={released:count}}sidecar.send({replyTo:m.id,value})});
const controlPort=await sidecarReady;const info={publicPort:publicServer.address().port,controlPort};fs.writeFileSync(readyFile,JSON.stringify(info));console.log(JSON.stringify(info));
process.on('SIGTERM',()=>{sidecar.kill('SIGTERM');publicServer.close();process.exit(0)});
