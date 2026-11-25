import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import {fork} from 'node:child_process';
const ready=process.argv[2],records=new Map(),requests=[];let config={deny:0,drop:0,hold:false,conflict:false};const wait=[];
const json=(r,s,x)=>{const b=JSON.stringify(x);r.writeHead(s,{'content-type':'application/json'});r.end(b)};
const body=req=>new Promise((ok,no)=>{let b='';req.on('data',x=>b+=x);req.on('end',()=>ok(b));req.on('error',no)});
const sidecar=fork(path.join(path.dirname(new URL(import.meta.url).pathname),'control-sidecar.mjs'),[],{stdio:['ignore','ignore','ignore','ipc']});let controlPort;const sideReady=new Promise(resolve=>sidecar.on('message',m=>{if(m?.ready)resolve(m.port);if(m?.action==='state')sidecar.send({replyTo:m.id,value:{requests,records:[...records.entries()]}});if(m?.action==='config'){config={...config,...m.value};if(!config.hold)for(const f of wait.splice(0))f();sidecar.send({replyTo:m.id,value:{ok:true}})}if(m?.action==='reset'){requests.length=0;records.clear();config={deny:0,drop:0,hold:false,conflict:false};sidecar.send({replyTo:m.id,value:{ok:true}})}}));
const server=http.createServer(async(req,res)=>{
 const u=new URL(req.url,'http://local');
 if(req.method==='POST'&&u.pathname==='/reserve'){
  let x;try{x=JSON.parse(await body(req))}catch{return json(res,400,{error:'invalid'})}
  requests.push({method:'POST',...x});
  if(config.conflict){config.conflict=false;return json(res,409,{error:'conflict'})}
  let r=records.get(x.operation_id);
  if(!r){if(config.deny>0){config.deny--;r={outcome:'denied'};records.set(x.operation_id,r)}else{r={outcome:'granted',equipment_id:'equipment-'+x.item_id};records.set(x.operation_id,r)}}
  const send=()=>json(res,200,r);
  if(config.drop>0){config.drop--;res.destroy();return}
  if(config.hold){wait.push(send);return}
  return send();
 }
 if(req.method==='GET'&&u.pathname.startsWith('/operations/')){
  const id=decodeURIComponent(u.pathname.slice('/operations/'.length));
  requests.push({method:'GET',operation_id:id});
  const r=records.get(id);
  return r?json(res,200,{found:true,...r}):json(res,200,{found:false});
 }
 return json(res,404,{error:'not_found'});
});
server.listen(0,'127.0.0.1',async()=>{controlPort=await sideReady;fs.writeFileSync(ready,JSON.stringify({port:server.address().port,controlPort}))});process.on('SIGTERM',()=>{server.close();sidecar.kill('SIGTERM');process.exit(0)});
