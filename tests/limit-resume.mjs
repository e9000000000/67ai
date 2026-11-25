import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';

const root = path.resolve('tmp', `limit-resume-${Date.now()}`);
await mkdir(root, {recursive:true});
const fake = path.join(root, 'fake-codex.exe');
const compile = spawnSync('rustc', ['tests/fake_codex.rs', '-o', fake], {stdio:'inherit', windowsHide:true});
assert.equal(compile.status, 0);
const url = 'http://127.0.0.1:3038';
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
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3038','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','pipe','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<50;i++){try{await state();return;}catch{await delay(100);}}throw Error('Server failed to start');
}
async function stop() { const done=new Promise(r=>server.once('exit',r));server.kill();await done; }
try {
  await start();
  const projectPath = path.join(root, 'project');
  await mkdir(projectPath);
  const project_id = (await action({action:'add_project',name:'Limit test',path:projectPath})).id;
  const id = (await action({action:'create_task',project_id,title:'LIMIT_TEST',text:'Keep working',model:'',status:'todo'})).id;
  let d = await until(d=>d.tasks.find(t=>t.id===id)?.status==='todo' && d.codex_paused_until > Date.now());
  const reset = d.codex_paused_until;
  const paused = d.tasks.find(t=>t.id===id);
  assert.equal(paused.error, null);
  assert.equal(paused.sent_count, 0);
  assert(paused.thread_id);
  assert.equal(d.settings.paused, false);
  assert(paused.messages.some(m=>m.text.includes('will resume automatically')));
  const second = (await action({action:'create_task',project_id,title:'After limit',text:'Next task',model:'',status:'todo'})).id;
  await stop();
  await start();
  d = await state();
  assert.equal(d.codex_paused_until, reset);
  assert(d.tasks.every(t=>t.status==='todo'));
  await action({action:'settings',...d.settings,paused:true});
  await delay(Math.max(0, reset-Date.now()) + 600);
  assert((await state()).tasks.every(t=>t.status==='todo'));
  await action({action:'settings',...d.settings,paused:false});
  await until(d=>d.tasks.find(t=>t.id===id).status==='done');
  assert((await readFile(path.join(projectPath,'last-args.json'),'utf8')).includes('resume'));
  d = await until(d=>d.tasks.find(t=>t.id===second).status==='done');
  assert(d.tasks.every(t=>!t.error));
  assert.equal(d.tasks.find(t=>t.id===id).messages.filter(m=>m.role==='assistant').length, 1);
  console.log('Limit pause, persistence, manual pause, and automatic resume checks passed');
} finally {
  if (server && server.exitCode === null) await stop();
}
