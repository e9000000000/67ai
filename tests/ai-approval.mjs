import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile, rename } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import http from 'node:http';

const root = path.resolve('tmp', `ai-approval-${Date.now()}`);
await mkdir(root, {recursive:true});
const fake = path.join(root, 'fake-codex.exe');
const compile = spawnSync('rustc', ['tests/fake_codex.rs', '-o', fake], {stdio:'inherit', windowsHide:true});
assert.equal(compile.status, 0);
const url = 'http://127.0.0.1:3035';
let server;
const delay = ms => new Promise(r=>setTimeout(r,ms));
async function state() { const r=await fetch(`${url}/api/state`);assert.equal(r.status,200);return (await r.json()).data; }
async function action(value, expected=200) {
  const r=await fetch(`${url}/api/action`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(value)});
  const text=await r.text();assert.equal(r.status,expected,text);return expected===200?JSON.parse(text):text;
}
async function until(predicate, timeout=15000) {
  const start=Date.now();while(Date.now()-start<timeout){const d=await state();if(predicate(d))return d;await delay(100);}throw Error('Timed out waiting for state');
}
async function start() {
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3035','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','pipe','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<50;i++){try{await state();return;}catch{await delay(100);}}throw Error('Server failed to start');
}
async function stop() { const done=new Promise(r=>server.once('exit',r));server.kill();await done; }
try {
  await start();
  const p = (await action({action:'add_project',name:'Review project',path:root})).id;
  const otherDir=path.join(root,'other');await mkdir(otherDir);
  const other=(await action({action:'add_project',name:'Other project',path:otherDir})).id;
  await action({action:'settings',...(await state()).settings,hard_model:'hard-review-model',easy_model:'easy-model'});
  async function create(text='Check result', difficulty='easy', project_id=p) {
    const {id}=await action({action:'create_task',project_id,title:'Review test',text,model:'easy-model',difficulty,status:'todo'});
    await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');return id;
  }
  const id=await create();
  const original=(await state()).tasks.find(t=>t.id===id);
  await action({action:'ai_approve_task',id});
  await action({action:'ai_approve_task',id},400);
  let d=await until(d=>d.tasks.find(t=>t.id===id)?.cleanup_at);
  let t=d.tasks.find(t=>t.id===id);
  assert.equal(t.thread_id,original.thread_id);assert.equal(t.model,'easy-model');
  assert.equal(t.messages.at(-1).role,'reviewer');assert.match(t.messages.at(-1).text,/Approved/);
  await action({action:'ai_approve_task',id},400);

  const feedback=await create('REVIEW_FEEDBACK');
  await action({action:'ai_approve_task',id:feedback});
  await until(d=>d.tasks.find(t=>t.id===feedback)?.status==='now');
  await action({action:'settings',...(await state()).settings,paused:true});
  d=await until(d=>d.tasks.find(t=>t.id===feedback)?.status==='todo');
  t=d.tasks.find(t=>t.id===feedback);
  assert.equal(t.messages.at(-1).role,'reviewer');assert.equal(t.cleanup_at,null);
  await action({action:'settings',...d.settings,paused:false});
  await until(d=>d.tasks.find(t=>t.id===feedback)?.status==='done');
  assert.match(await readFile(path.join(root,'last-prompt.txt'),'utf8'),/reviewer: Review verdict/);
  const args=await readFile(path.join(root,'last-args.json'),'utf8');
  assert.match(args,/"resume", "11111111-1111-4111-8111-111111111111"/);
  assert.match(args,/"--model", "easy-model"/);

  const race=await create();
  await action({action:'ai_approve_task',id:race});
  await until(d=>d.tasks.find(t=>t.id===race)?.status==='now');
  await action({action:'settings',...(await state()).settings,paused:true});
  await action({action:'send_message',id:race,text:'New requirements'});
  d=await until(d=>d.tasks.find(t=>t.id===race)?.status==='todo');
  assert.equal(d.tasks.find(t=>t.id===race).cleanup_at,null);
  assert.match(d.tasks.find(t=>t.id===race).messages.at(-1).text,/discarded/);
  await action({action:'settings',...d.settings,paused:false});
  await until(d=>d.tasks.find(t=>t.id===race)?.status==='done');

  const interrupted=await create();
  await action({action:'ai_approve_task',id:interrupted});
  await until(d=>d.tasks.find(t=>t.id===interrupted)?.status==='now');
  await action({action:'stop_task',id:interrupted});
  await until(d=>d.tasks.find(t=>t.id===interrupted)?.status==='future');
  await action({action:'move_task',id:interrupted,status:'todo'});
  await until(d=>d.tasks.find(t=>t.id===interrupted)?.status==='now');
  await stop();await start();
  t=(await state()).tasks.find(t=>t.id===interrupted);
  assert.equal(t.status,'future');assert.equal(t.review_pending,true);assert.equal(t.cleanup_at,null);
  await action({action:'move_task',id:interrupted,status:'todo'});
  await until(d=>d.tasks.find(t=>t.id===interrupted)?.cleanup_at);

  for(const text of ['REVIEW_FAIL','REVIEW_INVALID']) {
    const failed=await create(text);
    await action({action:'ai_approve_task',id:failed});
    d=await until(d=>!!d.tasks.find(t=>t.id===failed)?.error);
    assert.equal(d.tasks.find(t=>t.id===failed).cleanup_at,null);
    await action({action:'approve_task',id:failed},400);
    // Retry remains an AI review, never an implicit approval.
    await action({action:'ai_approve_task',id:failed});
    await until(d=>!!d.tasks.find(t=>t.id===failed)?.error);
  }
  // Clear earlier tasks so enabling automation tests new work only.
  for(const t of (await state()).tasks)await action({action:'delete_task',id:t.id});
  await action({action:'project_settings',id:p,auto_approve_easy:true,auto_approve_hard:false});
  const easy=await create();
  await until(d=>d.tasks.find(t=>t.id===easy)?.cleanup_at);
  const hard=await create('Check hard','hard');
  const isolated=await create('Other project','easy',other);
  await delay(1200);d=await state();
  assert.equal(d.tasks.find(t=>t.id===hard).cleanup_at,null);
  assert.equal(d.tasks.find(t=>t.id===isolated).cleanup_at,null);
  await action({action:'project_settings',id:p,auto_approve_easy:false,auto_approve_hard:true});
  await until(d=>d.tasks.find(t=>t.id===hard)?.cleanup_at);
  await stop();await start();
  d=await state();
  assert.equal(d.projects.find(pj=>pj.id===p).auto_approve_hard,true);
  assert.equal(d.projects.find(pj=>pj.id===other).auto_approve_hard,false);
  assert.equal(d.tasks.find(t=>t.id===easy).difficulty,'easy');
  console.log('AI approval, feedback, stale verdict, failures, project settings and restart checks passed');
} finally { if(server&&!server.killed)await stop(); }
