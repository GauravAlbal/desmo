#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {Kernel} from '../../../../../../../typescript/dist/index.js';

const machinePath=path.join(path.dirname(fileURLToPath(import.meta.url)),'reservation.ik.toml');
const valid=x=>typeof x==='string'&&/^[A-Za-z0-9_-]{1,64}$/.test(x);
const json=value=>JSON.stringify(value);
const fail=error=>{process.stdout.write(json({error})+'\n');process.exit(1)};
const [cmd,id,a,b]=process.argv.slice(2),stateDir=process.env.STATE_DIR,service=process.env.SERVICE_URL;
if(!stateDir)fail('invalid');
let kernel,machine;
try{
  await fs.mkdir(stateDir,{recursive:true});
  kernel=new Kernel(path.join(stateDir,'g4-desmo.sqlite'),{binary:process.env.IK_BINARY});
  machine=await kernel.machine(machinePath);
}catch{fail('invalid')}

function facts(aggregate){return aggregate.state&&typeof aggregate.state==='object'&&!Array.isArray(aggregate.state)?aggregate.state:{}}
function view(aggregate){
  const f=facts(aggregate);
  return {reservation_id:aggregate.id,member_id:f.member_id??null,item_id:f.item_id??null,phase:aggregate.namedState(),attempts:Number(f.attempts??0),operation_id:f.operation_id??aggregate.id,equipment_id:f.equipment_id??null,hold_reason:f.hold_reason??null};
}
async function show(key){return machine.show(key)}
async function current(key){try{return await show(key)}catch{return null}}
function emit(value){process.stdout.write(json(value)+'\n');process.exit(0)}
async function transition(aggregate,name,args){return aggregate.command(name,args)}
async function request(method,url,data){
  const response=await fetch(url,{method,headers:data?{'content-type':'application/json'}:undefined,body:data?json(data):undefined,signal:AbortSignal.timeout(1500)});
  let body;try{body=await response.json()}catch{throw Error('invalid service response')}
  return {status:response.status,body};
}
async function finish(key,outcome){
  let aggregate=await current(key);if(!aggregate)return fail('not_found');
  try{
    if(outcome.outcome==='granted')aggregate=await transition(aggregate,'grant',{equipment_id:outcome.equipment_id});
    else if(outcome.outcome==='denied')aggregate=await transition(aggregate,'deny');
    else if(outcome.outcome==='conflict')aggregate=await transition(aggregate,'conflict');
    else return emit(view(aggregate));
  }catch{
    aggregate=await current(key);if(!aggregate)return fail('not_found');
  }
  emit(view(aggregate));
}
async function main(){
  if(cmd==='reserve'){
    if(!valid(id)||!valid(a)||!valid(b))return fail('invalid');
    if(await current(id))return fail('already_exists');
    try{
      const created=await machine.createWith({aggregateId:id,operationId:`reserve-${id}`,command:{name:'reserve',args:{member_id:a,item_id:b,operation_id:id}}});
      const aggregate=created?.machine?created:await show(id);
      return emit(view(aggregate));
    }catch{return fail('already_exists')}
  }
  if(cmd==='status'){
    if(!valid(id))return fail('invalid');
    const aggregate=await current(id);return aggregate?emit(view(aggregate)):fail('not_found');
  }
  if(cmd==='cancel'){
    if(!valid(id))return fail('invalid');
    let aggregate=await current(id);if(!aggregate)return fail('not_found');
    if(['cancelled','reserved','rejected'].includes(aggregate.namedState()))return emit(view(aggregate));
    try{aggregate=await transition(aggregate,'cancel')}catch{aggregate=await current(id);if(!aggregate)return fail('not_found')}
    return emit(view(aggregate));
  }
  if(cmd==='hold'){
    if(process.env.EVAL_STAGE!=='change')return fail('unknown_command');
    if(!valid(id)||typeof a!=='string'||!a.trim())return fail('invalid');
    let aggregate=await current(id);if(!aggregate)return fail('not_found');
    if(facts(aggregate).sent)return fail('already_dispatched');
    if(aggregate.namedState()!=='queued')return fail('invalid');
    try{aggregate=await transition(aggregate,'hold',{hold_reason:a});return emit(view(aggregate))}catch{return fail('invalid')}
  }
  if(cmd==='advance'){
    if(!valid(id))return fail('invalid');
    if(!service)return fail('invalid');
    let aggregate=await current(id);if(!aggregate)return fail('not_found');
    if(aggregate.namedState()==='hold')return fail('held');
    if(['reserved','rejected','cancelled'].includes(aggregate.namedState()))return emit(view(aggregate));
    if(aggregate.namedState()==='uncertain'){
      try{
        const lookup=await request('GET',`${service}/operations/${encodeURIComponent(id)}`);
        if(lookup.status===200&&lookup.body.found)return finish(id,lookup.body);
        if(lookup.status!==404&&lookup.status!==200)return emit(view(aggregate));
      }catch{return emit(view(aggregate))}
      aggregate=await current(id);if(!aggregate)return fail('not_found');
      if(['cancelled','reserved','rejected'].includes(aggregate.namedState()))return emit(view(aggregate));
    }else if(aggregate.namedState()==='queued'){
      try{aggregate=await transition(aggregate,'dispatch')}catch{aggregate=await current(id);if(!aggregate)return fail('not_found');if(aggregate.namedState()!=='uncertain')return emit(view(aggregate))}
    }
    if(aggregate.namedState()!=='uncertain')return emit(view(aggregate));
    let result;
    try{
      result=await request('POST',`${service}/reserve`,{operation_id:id,member_id:facts(aggregate).member_id,item_id:facts(aggregate).item_id});
    }catch{return emit(view(await current(id)??aggregate))}
    if(result.status===409)return finish(id,{outcome:'conflict'});
    if(result.status!==200||!['granted','denied'].includes(result.body.outcome))return emit(view(await current(id)??aggregate));
    return finish(id,result.body);
  }
  return fail(cmd?'unknown_command':'invalid');
}
main().catch(()=>fail('invalid'));
