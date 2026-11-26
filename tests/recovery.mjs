import {spawn, spawnSync} from 'node:child_process';
import {mkdir, readFile, writeFile, rm} from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';

const root=path.resolve('tmp', `recovery-${Date.now()}`);
await mkdir(root,{recursive:true});
const fake=path.join(root,'fake-codex.exe');
assert.equal(spawnSync('rustc',['tests/fake_codex.rs','-o',fake],{windowsHide:true,stdio:'inherit'}).status,0);
const url='http://127.0.0.1:3045';
let server;
function start(){server=spawn(path.resolve('target/debug/67ai.exe'),[],{windowsHide:true,stdio:['ignore','ignore','pipe'],env:{...process.env,'67AI_PORT':'3045','67AI_DATA_DIR':root,'67AI_CODEX':fake}});server.stderr.on('data',b=>process.stderr.write(b));}
async function stop(){const stopped=new Promise(r=>server.once('exit',r));server.kill();await stopped;}
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const state=async()=>(await (await fetch(`${url}/api/state`)).json()).data;
const action=async body=>{const r=await fetch(`${url}/api/action`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});assert.equal(r.status,200,await r.clone().text());return r.json();};
async function until(predicate) {
  for(let i=0;i<200;i++){const d=await state();if(predicate(d))return d;await delay(100);}
  throw Error(`Timed out: ${JSON.stringify(await state())}`);
}
const git=(cwd,...args)=>{const r=spawnSync('git',args,{cwd,windowsHide:true,encoding:'utf8'});assert.equal(r.status,0,r.stderr);return r.stdout.trim();};
let heartbeat;
try {
  start();
  for(let i=0;i<50;i++){try{await state();break;}catch{await delay(100);}}
  // Native audio stays suppressed throughout this isolated test.
  const presence=()=>fetch(`${url}/api/presence`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:'silent-recovery',focused:true,rendered_at:0})}).catch(()=>{});
  await presence();heartbeat=setInterval(presence,500);
  const project=(await action({action:'add_project',name:'Recovery',path:root})).id;
  let settings={...(await state()).settings,hard_model:'hard-recovery-model',paused:false};
  await action({action:'settings',...settings});
  const create=async(title,project_id=project,text=title)=>(await action({action:'create_task',project_id,title,text,model:'easy-model',status:'todo'})).id;
  for(const problem of ['PROCESS_FAIL','BLOCKED','REFUSED','UNCONFIRMED','ALWAYS_FAIL','QUESTION']) {
    const id=await create(`RECOVERY_TEST ${problem}`);
    if(problem!=='QUESTION') {
      const recovering=await until(d=>d.tasks.find(t=>t.id===id)?.messages.some(m=>m.text.startsWith('Recovering automatically')));
      assert.equal(recovering.tasks.find(t=>t.id===id).status,'now');
      assert.equal(recovering.tasks.find(t=>t.id===id).error,null);
    }
    const d=await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');
    const t=d.tasks.find(t=>t.id===id);
    const attempts=t.messages.filter(m=>m.role==='activity'&&m.text.startsWith('Recovering automatically'));
    assert.equal(attempts.length,problem==='QUESTION'?0:1,problem);
    assert.equal(t.model,'easy-model');
    if(problem==='QUESTION')assert.equal(t.attention,'question');
    else if(problem==='ALWAYS_FAIL')assert.equal(t.attention,'failed');
    else {assert.equal(t.error,null);assert.equal(t.attention,null);assert(t.session_ids.includes('recovery-true'));}
  }
  const gen=(await action({action:'create_task',project_id:project,title:'RECOVERY_TEST GEN_BLOCKED',text:'Research only',model:'gen-model',difficulty:'gen',status:'todo'})).id;
  let researched=await until(d=>d.tasks.find(t=>t.id===gen)?.status==='done');
  assert.equal(researched.tasks.find(t=>t.id===gen).attention,null);
  assert(researched.tasks.find(t=>t.id===gen).messages.some(m=>m.text.startsWith('Recovering automatically')));
  const review=await create('RECOVERY_TEST REVIEW');
  await until(d=>d.tasks.find(t=>t.id===review)?.status==='done');
  await action({action:'ai_approve_task',id:review});
  const reviewed=await until(d=>d.tasks.find(t=>t.id===review)?.cleanup_at);
  assert.equal(reviewed.tasks.find(t=>t.id===review).error,null);
  assert.match(reviewed.tasks.find(t=>t.id===review).messages.at(-1).text,/Review recovered/);
  // Dirty-main recovery preserves files, then retries setup and merging without user input.
  const repo=path.join(root,'repo');await mkdir(repo);
  git(repo,'init','-b','main');git(repo,'config','user.name','Test');git(repo,'config','user.email','test@example.com');
  await writeFile(path.join(repo,'base.txt'),'base\n');git(repo,'add','.');git(repo,'commit','-m','Initial');
  const gitProject=(await action({action:'add_project',name:'Git recovery',path:repo})).id;
  settings={...settings,parallelism:true};await action({action:'settings',...settings});
  await writeFile(path.join(repo,'pending.txt'),'keep this work\n');
  const dirty=await create('RECOVERY_TEST DIRTY',gitProject);
  let d=await until(d=>{const t=d.tasks.find(t=>t.id===dirty);return t?.status==='done'&&!t.awaiting_merge;});
  assert.equal(d.tasks.find(t=>t.id===dirty).error,null);
  assert.equal(await readFile(path.join(repo,'pending.txt'),'utf8'),'keep this work\n');
  assert.equal(git(repo,'status','--porcelain'),'');
  // A claimed recovery with no actual fix must surface only after verification fails.
  await writeFile(path.join(repo,'recovery-noop'),'');
  const noop=await create('RECOVERY_TEST NOOP',gitProject);
  d=await until(d=>d.tasks.find(t=>t.id===noop)?.error);
  const failed=d.tasks.find(t=>t.id===noop);
  assert.match(failed.error,/Hard-model recovery failed/);
  assert.equal(failed.messages.filter(m=>m.text.startsWith('Recovering automatically')).length,1);
  await rm(path.join(repo,'recovery-noop'));
  // A commit hook failure is repaired and the merge is retried automatically.
  const hookTask=await create('RECOVERY_TEST HOOK',gitProject,'PARALLEL_TEST');
  await until(d=>d.tasks.find(t=>t.id===hookTask)?.thread_id);
  await action({action:'settings',...settings,paused:true});
  await until(d=>d.tasks.find(t=>t.id===hookTask)?.awaiting_merge);
  await writeFile(path.join(repo,'.git','hooks','pre-commit'),'#!/bin/sh\nexit 1\n');
  await action({action:'settings',...settings});
  d=await until(d=>!d.tasks.find(t=>t.id===hookTask)?.awaiting_merge);
  assert.equal(d.tasks.find(t=>t.id===hookTask).error,null);
  assert(d.tasks.find(t=>t.id===hookTask).messages.some(m=>m.text.startsWith('Recovering automatically')));
  assert.equal(git(repo,'status','--porcelain'),'');
  // Approved cleanup retries safely, including deletion of the recovery session.
  await stop();
  const stateFile=path.join(root,'67ai.json');
  d=JSON.parse(await readFile(stateFile,'utf8'));
  Object.assign(d.tasks.find(t=>t.id===review),{thread_id:'recoverable-delete',session_ids:[],cleanup_at:0});
  d.settings.paused=true;
  await writeFile(stateFile,JSON.stringify(d));await writeFile(path.join(root,'cleanup-blocked'),'');
  start();
  for(let i=0;i<50;i++){try{await state();break;}catch{await delay(100);}}
  await presence();
  await until(d=>!d.tasks.some(t=>t.id===review));
  const deleted=await readFile(path.join(root,'deleted-sessions.jsonl'),'utf8');
  assert.match(deleted,/recoverable-delete/);assert.match(deleted,/cleanup-recovery/);
  console.log('PASS: hard-model recovery, bounded retries, questions, Gen/review restrictions, dirty main, verified recovery failure, merge prerequisite repair, and cleanup recovery.');
} finally {
  clearInterval(heartbeat);
  if(server?.exitCode===null)await stop();
}
