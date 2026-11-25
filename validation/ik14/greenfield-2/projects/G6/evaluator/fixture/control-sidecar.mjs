import http from 'node:http';
let seq=0;const pending=new Map();
process.on('message',m=>{if(m?.replyTo){const p=pending.get(m.replyTo);if(p){pending.delete(m.replyTo);p(m.value)}}});
function parent(action,value){return new Promise(resolve=>{const id=++seq;pending.set(id,resolve);process.send({id,action,value})})}
const body=req=>new Promise((ok,no)=>{let b='';req.on('data',x=>b+=x);req.on('end',()=>ok(b));req.on('error',no)});
const send=(res,status,obj)=>{const b=JSON.stringify(obj);res.writeHead(status,{'content-type':'application/json','content-length':Buffer.byteLength(b)});res.end(b)};
const server=http.createServer(async(req,res)=>{const u=new URL(req.url,'http://localhost');if(req.method==='GET'&&u.pathname==='/control/health')return send(res,200,{ok:true});if(req.method==='GET'&&u.pathname==='/control/state')return send(res,200,await parent('state'));if(req.method==='POST'&&['/control/reset','/control/config','/control/release'].includes(u.pathname)){let data={};try{data=JSON.parse(await body(req)||'{}')}catch{return send(res,400,{error:'invalid'})}const result=await parent(u.pathname.split('/').at(-1),data);return send(res,200,result)}send(res,404,{error:'not_found'})});
server.listen(0,'127.0.0.1',()=>process.send({ready:true,port:server.address().port}));
