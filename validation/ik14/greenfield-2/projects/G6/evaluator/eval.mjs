import {spawn} from 'node:child_process';import fs from 'node:fs/promises';import os from 'node:os';import path from 'node:path';import http from 'node:http';
const project=process.argv[2],stub=process.argv[3],selected=process.argv[4];
const stage=process.argv[5]||'initial';
if(!['initial','change'].includes(stage))throw Error('stage must be initial or change');
const reqs=stage==='initial'?['R1','R2','R3','R4','R5','R6','R7']:['R1','R2','R3','R4','R5','R6','R7','C1'];
function request(url,method='GET',data,timeoutMs=5000){return new Promise((resolve,reject)=>{const u=new URL(url);const q=http.request(u,{method,headers:data?{'content-type':'application/json'}:{}},r=>{let b='';r.on('data',x=>b+=x);r.on('end',()=>{clearTimeout(timer);try{resolve({status:r.statusCode,body:JSON.parse(b)})}catch{reject(Error('non-JSON provider response'))}})});const timer=setTimeout(()=>q.destroy(Error('request timed out')),timeoutMs);q.on('error',e=>{clearTimeout(timer);reject(e)});if(data)q.end(JSON.stringify(data));else q.end()})}
function killGroup(child,signal){if(!child?.pid)return;try{process.kill(-child.pid,signal)}catch{try{child.kill(signal)}catch{}}}
const activeChildren=new Set();let activeProvider,activeFixtureCleanup;
async function fixture(fn){
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),'g6-eval-')),ready=path.join(dir,'ready.json');
  const p=spawn(process.execPath,[path.join(stub,'server.mjs'),ready],{stdio:'ignore',detached:true});
  let ctl,cleaned=false;
  async function cleanup(){
    if(cleaned)return;cleaned=true;
    if(ctl){
      for(const [route,data] of [['config',{hold:false}],['release',{}],['reset',{}]]){
        try{await request(`${ctl}/control/${route}`,'POST',data,1000)}catch{}
      }
    }
    killGroup(p,'SIGTERM');
    await Promise.race([new Promise(resolve=>p.once('exit',resolve)),new Promise(resolve=>setTimeout(resolve,250))]);
    if(p.exitCode===null&&p.signalCode===null)killGroup(p,'SIGKILL');
    await fs.rm(dir,{recursive:true,force:true});
  }
  activeProvider=p;activeFixtureCleanup=cleanup;
  try{
    let info;
    for(let i=0;i<100;i++){try{info=JSON.parse(await fs.readFile(ready,'utf8'));break}catch{}await new Promise(r=>setTimeout(r,20))}
    if(!info)throw Error('provider did not start');
    const base=`http://127.0.0.1:${info.publicPort}`;ctl=`http://127.0.0.1:${info.controlPort}`;
    const env={...process.env,STATE_DIR:path.join(dir,'state'),SERVICE_URL:base};
    const cmd=(args,extra={})=>new Promise((resolve,reject)=>{
      const argv=['./run',...args].join(' '),c=spawn('./run',args,{cwd:project,env:{...env,...extra},stdio:['ignore','pipe','pipe'],detached:true});
      let out='',err='',timedOut=false;
      activeChildren.add(c);
      const timer=setTimeout(()=>{timedOut=true;killGroup(c,'SIGKILL')},20_000);
      c.stdout.on('data',x=>out+=x);c.stderr.on('data',x=>err+=x);
      c.on('error',e=>{clearTimeout(timer);activeChildren.delete(c);reject(e)});
      c.on('close',(code,signal)=>{clearTimeout(timer);activeChildren.delete(c);let json;try{json=JSON.parse(out)}catch{}resolve({code,signal,json,out,err,timedOut,argv})});
    });
    const state=()=>request(ctl+'/control/state');
    await fn({cmd,state,config:x=>request(ctl+'/control/config','POST',x)});
  }finally{
    await cleanup();
    if(activeProvider===p)activeProvider=undefined;
    if(activeFixtureCleanup===cleanup)activeFixtureCleanup=undefined;
  }
}
function checked(r){if(r.timedOut)throw Error(`command timed out: ${r.argv}`);return r}
function ok(r){r=checked(r);if(r.code!==0||!r.json)throw Error(`expected success, got ${r.code}: ${r.out} ${r.err}`);return r.json}
function fail(r){r=checked(r);if(r.code===0)throw Error(`expected refusal: ${r.out}`);if(!r.json||typeof r.json!=='object'||Array.isArray(r.json)||typeof r.json.error!=='string')throw Error(`expected refusal JSON error object: ${r.out} ${r.err}`)}function eq(a,b){if(JSON.stringify(a)!==JSON.stringify(b))throw Error(`expected ${JSON.stringify(b)}, got ${JSON.stringify(a)}`)}
const cases={
R1:async f=>{let x=ok(await f.cmd(['subscribe','acct','email']));if(x.version!==1)throw Error('subscribe version not 1');eq((await f.state()).body.effects,[]);const y=ok(await f.cmd(['status','acct']));if(y.enabled!==true||y.channel!=='email'||y.version!==1)throw Error('status mismatch');const before=(await f.state()).body.requests.length;fail(await f.cmd(['subscribe','bad!','email']));fail(await f.cmd(['subscribe','acct','push']));fail(await f.cmd(['deliver','acct','bad!','hello']));if((await f.state()).body.requests.length!==before)throw Error('invalid input called provider');const after=ok(await f.cmd(['status','acct']));if(after.channel!=='email'||after.version!==1)throw Error('invalid subscribe changed preference');const beforeUnknown=(await f.state()).body.requests.length;fail(await f.cmd(['deliver','unknown','m1','hello']));if((await f.state()).body.requests.length!==beforeUnknown)throw Error('unknown account called provider')},
R2:async f=>{ok(await f.cmd(['subscribe','acct','sms']));let a=ok(await f.cmd(['unsubscribe','acct'])),b=ok(await f.cmd(['unsubscribe','acct']));if(a.version!==2||a.enabled!==false||a.channel!=='sms'||b.version!==2||b.enabled!==false||b.channel!=='sms')throw Error('unsubscribe idempotence/version mismatch')},
R3:async f=>{ok(await f.cmd(['subscribe','acct','email']));ok(await f.cmd(['unsubscribe','acct']));const before=(await f.state()).body.requests.length;const a=ok(await f.cmd(['deliver','acct','m1','hello']));if(a.phase!=='suppressed')throw Error('disabled delivery not suppressed');const s=await f.state();if(s.body.requests.length!==before||s.body.effects.length!==0)throw Error('disabled delivery called provider');},
R4:async f=>{ok(await f.cmd(['subscribe','acct','email']));await f.config({drop_next:1});const lost=ok(await f.cmd(['deliver','acct','m1','hello']));if(lost.phase!=='uncertain'||Object.hasOwn(lost,'provider_id'))throw Error('lost reply not recorded uncertain');const recovered=ok(await f.cmd(['deliver','acct','m1','hello']));if(recovered.phase!=='delivered'||recovered.delivery_id!=='acct:m1:v1'||recovered.provider_id!=='provider-acct:m1:v1')throw Error('recovery did not reconcile delivered effect');const s=await f.state();if(s.body.effects.length>1)throw Error('recovery duplicated effect');if(s.body.effects.length===0)throw Error('recovery recorded no effect');const exact=s.body.requests.filter(x=>x.delivery_id==='acct:m1:v1'),firstPost=exact.findIndex(x=>x.method==='POST'),recoveryGet=exact.findIndex((x,i)=>i>firstPost&&x.method==='GET');if(firstPost<0||recoveryGet<0)throw Error('recovery did not lookup exact id');if(exact.slice(firstPost+1,recoveryGet).some(x=>x.method==='POST')||exact.filter(x=>x.method==='POST').length!==1)throw Error('recovery posted before exact lookup');},
R5:async f=>{ok(await f.cmd(['subscribe','acct','email']));await f.config({hold:true});const late=f.cmd(['deliver','acct','m1','hello']);let postSeen=false;for(let i=0;i<100;i++){if((await f.state()).body.requests.some(x=>x.delivery_id==='acct:m1:v1'&&x.method==='POST')){postSeen=true;break}await new Promise(r=>setTimeout(r,10))}if(!postSeen)throw Error('held delivery POST not observed');ok(await f.cmd(['unsubscribe','acct']));ok(await f.cmd(['subscribe','acct','sms']));await f.config({hold:false});const old=ok(await late);if(old.phase!=='delivered'||old.version!==1||old.delivery_id!=='acct:m1:v1'||old.provider_id!=='provider-acct:m1:v1')throw Error('in-flight delivery changed captured version');const s=ok(await f.cmd(['status','acct']));if(s.enabled!==true||s.version!==3||s.channel!=='sms')throw Error('late response rewrote current preference');const newer=ok(await f.cmd(['deliver','acct','m2','hello']));if(newer.delivery_id!=='acct:m2:v3'||newer.version!==3)throw Error('new delivery used captured old version');},
R6:async f=>{ok(await f.cmd(['subscribe','acct','email']));const first=ok(await f.cmd(['deliver','acct','m1','hello']));if(first.phase!=='delivered')throw Error('initial message not delivered');const before=(await f.state()).body.effects.length;fail(await f.cmd(['deliver','acct','m1','different']));if((await f.state()).body.effects.length!==before)throw Error('conflicting message created provider effect');const n=ok(await f.cmd(['deliver','acct','m2','new']));if(n.phase!=='delivered')throw Error('new message refused')},
R7:async f=>{ok(await f.cmd(['subscribe','acct','email']));const before=(await f.state()).body.requests.length;ok(await f.cmd(['status','acct']));if((await f.state()).body.requests.length!==before)throw Error('status called provider');fail(await f.cmd(['nonsense']));fail(await f.cmd(['deliver','acct','m1','hello'],{SERVICE_URL:''}))},
 C1:async f=>{ok(await f.cmd(['subscribe','acct','email']));const q=ok(await f.cmd(['quiet','acct']));if(q.enabled!==true||q.channel!=='email'||q.version!==1)throw Error('quiet changed preference');const q2=ok(await f.cmd(['quiet','acct']));if(q2.enabled!==true||q2.channel!=='email'||q2.version!==1)throw Error('repeated quiet changed preference');const pref=ok(await f.cmd(['status','acct']));if(pref.enabled!==true||pref.channel!=='email'||pref.version!==1)throw Error('quiet changed preference');const d0=await f.state(),qdel=ok(await f.cmd(['deliver','acct','m1','hello']));if(qdel.phase!=='suppressed')throw Error('quiet delivery not suppressed');if((await f.state()).body.effects.length!==d0.body.effects.length)throw Error('quiet caused provider call');const u=ok(await f.cmd(['unquiet','acct']));if(u.enabled!==true||u.channel!=='email'||u.version!==1)throw Error('unquiet changed preference');const u2=ok(await f.cmd(['unquiet','acct']));if(u2.enabled!==true||u2.channel!=='email'||u2.version!==1)throw Error('repeated unquiet changed preference');const old=ok(await f.cmd(['deliver','acct','m1','hello']));if(old.phase!=='suppressed')throw Error('previously quiet delivery became deliverable');const d=ok(await f.cmd(['deliver','acct','m2','hello']));if(d.phase!=='delivered'||d.delivery_id!=='acct:m2:v1')throw Error('unquiet did not permit new delivery');ok(await f.cmd(['subscribe','acct2','email']));const disabled=ok(await f.cmd(['unsubscribe','acct2']));if(disabled.enabled!==false||disabled.version!==2)throw Error('disabled setup mismatch');const qd=ok(await f.cmd(['quiet','acct2']));if(qd.enabled!==false||qd.channel!=='email'||qd.version!==2)throw Error('quiet changed disabled preference');const qd2=ok(await f.cmd(['quiet','acct2']));if(qd2.enabled!==false||qd2.channel!=='email'||qd2.version!==2)throw Error('repeated quiet changed disabled preference');const ud=ok(await f.cmd(['unquiet','acct2']));if(ud.enabled!==false||ud.channel!=='email'||ud.version!==2)throw Error('unquiet changed disabled preference');const ud2=ok(await f.cmd(['unquiet','acct2']));if(ud2.enabled!==false||ud2.channel!=='email'||ud2.version!==2)throw Error('repeated unquiet changed disabled preference');const end=ok(await f.cmd(['status','acct2']));if(end.enabled!==false||end.channel!=='email'||end.version!==2)throw Error('unquiet changed disabled status');fail(await f.cmd(['quiet','unknown']));fail(await f.cmd(['unquiet','bad!']))}
};
const results=[];const activeReqs=reqs.filter(id=>selected==='all'||selected===id);const watchdog=setTimeout(()=>{for(const child of activeChildren)killGroup(child,'SIGKILL');if(activeProvider)killGroup(activeProvider,'SIGKILL');if(activeFixtureCleanup)void activeFixtureCleanup();const done=new Set(results.map(x=>x.requirement));const pending=activeReqs.filter(id=>!done.has(id)).map(requirement=>({requirement,pass:false,error:'evaluator watchdog'}));process.stdout.write(JSON.stringify({stage,results:[...results,...pending],pass:false})+'\n',()=>process.exit(1))},200_000);for(const id of activeReqs){try{await fixture(cases[id]);results.push({requirement:id,pass:true})}catch(e){results.push({requirement:id,pass:false,error:String(e.message||e)})}}clearTimeout(watchdog);console.log(JSON.stringify({stage,results,pass:results.every(x=>x.pass)}));process.exitCode=results.every(x=>x.pass)?0:1;
