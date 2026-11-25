import {spawnSync} from 'node:child_process';
import path from 'node:path';
const base=path.resolve(process.argv[2]);
const stage=process.argv[3]||'initial';
const change=stage==='change';
const ids=['R1','R2','R3','R4','R5','R6','R7','R8',...(change?['C1']:[])];
const evaluator=path.resolve('validation/ik14/greenfield-2/projects/G5/evaluator/run.sh');
let valid=true;
function invoke(project,id,defect=false){const cwd=path.resolve(project);const env={...process.env,PROJECT_DIR:cwd,EVAL_CASE:'all',...(defect?{DEFECT:id}:{})};const r=spawnSync('/bin/sh',[evaluator,stage],{encoding:'utf8',env,timeout:30000});const stdout=r.stdout||'',stderr=r.stderr||String(r.error||'');let output;try{output=JSON.parse(stdout.trim().split('\n').at(-1));}catch{}return {status:r.status,stdout:stdout.trim(),stderr:stderr.trim(),output};}
const reference=process.env.ARM==='B'?'reference-B':'reference';
const ref=invoke(path.join(base,'controls',reference),'all');console.log(JSON.stringify({kind:'reference',project:path.join(base,'controls',reference),stage,arm:process.env.ARM||'A',status:ref.status,output:ref.output,stdout:ref.stdout,stderr:ref.stderr}));if(ref.status!==0||!Object.values(ref.output?.requirements||{}).every(Boolean))valid=false;
for(const id of ids){const project=process.env.ARM==='B'?path.join(base,'controls','reference-B'):path.join(base,'controls',id);const result=invoke(project,id,process.env.ARM==='B');const req=result.output?.requirements||{};const exact=Object.keys(req).length===ids.length&&ids.every(name=>req[name]===(name!==id));console.log(JSON.stringify({kind:'defect',arm:process.env.ARM||'A',stage,requirement:id,status:result.status,output:result.output,stdout:result.stdout,exact}));if(!exact)valid=false;}
process.exitCode=valid?0:1;
