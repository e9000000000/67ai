import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';

const source = await readFile('web/app.js','utf8');
let markup, submit, saved;
const ui = {
  data:{projects:[],settings:{max_workers:4,parallelism:false}}, models:[], projectId:'p',
  escape:s=>s??'', formEnd:()=>'', templates(){},
  $:()=>({insertAdjacentHTML(){}}), modal:(title,html)=>{markup=html;},
  formSubmit:handler=>{submit=handler;}, api:value=>{saved=value;},
  duration:()=>'', elapsed:()=>0,
};
runInNewContext(source.slice(source.indexOf('function settings(){'), source.indexOf("$('#project-tabs').onclick")),ui);
ui.settings();
assert.match(markup,/name="max_workers"[^>]*value="4"/);
assert.doesNotMatch(markup,/name="parallelism"[^>]*checked/);
await submit(new Map([['max_workers','3'],['parallelism','on']]));
assert.equal(saved.max_workers,3); assert.equal(saved.parallelism,true);
await submit(new Map([['max_workers','1']]));
assert.equal(saved.parallelism,false);
runInNewContext(source.slice(source.indexOf('const needsAttention'),source.indexOf('const duration')) + source.slice(source.indexOf('function card('),source.indexOf('function modal(')),ui);
const orange=ui.card({id:'t',title:'Completed',status:'done',awaiting_merge:true,error:'Conflict'});
assert.match(orange,/awaiting-merge/); assert.match(orange,/Merge needs attention/);
assert.match(orange,/draggable="false"/); assert.doesNotMatch(orange,/needs-attention/);
assert.doesNotMatch(ui.card({id:'t',title:'Merged',status:'done'}),/awaiting-merge/);

const root = path.resolve('tmp', `parallelism-${Date.now()}`);
await mkdir(root, {recursive:true});
const repo = path.join(root, 'project');
await mkdir(repo);
function git(cwd, ...args) {
  const result = spawnSync('git', args, {cwd, encoding:'utf8', windowsHide:true});
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}
git(repo, 'init', '-b', 'main');
git(repo, 'config', 'core.autocrlf', 'false');
git(repo, 'config', 'user.name', 'Test');
git(repo, 'config', 'user.email', 'test@example.com');
await writeFile(path.join(repo, 'base.txt'), 'base\n');
git(repo, 'add', '.'); git(repo, 'commit', '-m', 'Initial');
const fake = path.join(root, 'fake-codex.exe');
assert.equal(spawnSync('rustc', ['tests/fake_codex.rs', '-o', fake], {stdio:'inherit', windowsHide:true}).status, 0);
const url = 'http://127.0.0.1:3034';
const delay = ms => new Promise(resolve=>setTimeout(resolve,ms));
let server;
async function state() { return (await (await fetch(`${url}/api/state`)).json()).data; }
async function action(body, expected=200) {
  const r = await fetch(`${url}/api/action`, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)});
  const text = await r.text(); assert.equal(r.status, expected, text);
  return expected===200 ? JSON.parse(text) : text;
}
async function until(predicate) {
  for(let i=0;i<200;i++) { const d=await state(); if(predicate(d)) return d; await delay(100); }
  throw Error(`Timed out: ${JSON.stringify(await state())}`);
}
async function start() {
  server=spawn(path.resolve('target/debug/taskdesk.exe'), [], {env:{...process.env,TASKDESK_PORT:'3034',TASKDESK_DATA_DIR:root,TASKDESK_CODEX:fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data', b=>process.stderr.write(b));
  for(let i=0;i<50;i++) { try { await state(); return; } catch { await delay(100); } }
  throw Error('Server did not start');
}
async function stop() { const exited=new Promise(resolve=>server.once('exit',resolve)); server.kill(); await exited; }
try {
  await start();
  let d=await state();
  assert.equal(d.settings.max_workers,4); assert.equal(d.settings.parallelism,false);
  let settings={...d.settings,paused:true,parallelism:true,max_workers:2};
  await action({action:'settings',...settings,max_workers:0},400);
  await action({action:'settings',...settings});
  const project_id=(await action({action:'add_project',name:'Git project',path:repo})).id;
  const create=async title=>(await action({action:'create_task',project_id,title,text:'PARALLEL_TEST',model:'',status:'todo'})).id;
  const first=await create('First'), second=await create('Second'), third=await create('Third');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>[first,second].every(id=>d.tasks.find(t=>t.id===id).thread_id));
  assert.equal(d.tasks.filter(t=>t.status==='now').length,2);
  assert.equal(d.tasks.find(t=>t.id===third).status,'todo');
  const paths=d.tasks.filter(t=>t.status==='now').map(t=>t.worktree);
  assert.equal(new Set(paths).size,2); assert(paths.every(p=>p!==repo));
  assert.equal(git(repo,'ls-files'),'base.txt');
  await action({action:'settings',...settings});
  d=await until(d=>d.tasks.filter(t=>t.awaiting_merge).length===2);
  assert(d.tasks.filter(t=>t.awaiting_merge).every(t=>t.status==='done'));
  for(const body of [
    {action:'approve_task',id:first}, {action:'delete_task',id:first},
    {action:'move_task',id:first,status:'todo'}, {action:'send_message',id:first,text:'Follow-up'},
  ]) await action(body,400);
  await stop(); await start();
  d=await state(); assert.equal(d.tasks.filter(t=>t.awaiting_merge).length,2);
  // Switching parallelism off drains merges and keeps subsequent runs sequential.
  settings={...settings,parallelism:false};
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>d.tasks.every(t=>t.status==='done'&&!t.awaiting_merge));
  assert(d.tasks.every(t=>!t.error));
  assert(git(repo,'ls-files').includes('First.txt'));
  assert(git(repo,'ls-files').includes('Second.txt'));
  // Third ran directly in main because parallelism was disabled before its first run.
  assert.equal(d.tasks.find(t=>t.id===third).worktree,null);
  git(repo,'add','.'); git(repo,'commit','-m','Sequential task');

  settings={...settings,parallelism:true,paused:true};
  await action({action:'settings',...settings});
  const conflict=await create('Conflict');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>d.tasks.find(t=>t.id===conflict).thread_id);
  const tree=d.tasks.find(t=>t.id===conflict).worktree;
  await action({action:'settings',...settings});
  await until(d=>d.tasks.find(t=>t.id===conflict).awaiting_merge);
  await writeFile(path.join(repo,'base.txt'),'main edit\n');
  git(repo,'add','.'); git(repo,'commit','-m','Main edit');
  await writeFile(path.join(tree,'base.txt'),'task edit\n');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>d.tasks.find(t=>t.id===conflict).error);
  assert.equal(d.tasks.find(t=>t.id===conflict).awaiting_merge,true);
  assert.match(d.tasks.find(t=>t.id===conflict).error,/Merge failed/);
  assert.equal(git(repo,'status','--porcelain'),'');
  assert.equal(await readFile(path.join(repo,'base.txt'),'utf8'),'main edit\n');
  git(tree,'merge','-s','ours','main','-m','Resolve test conflict');
  await action({action:'retry_merge',id:conflict});
  d=await until(d=>!d.tasks.find(t=>t.id===conflict).awaiting_merge);
  assert.equal(d.tasks.find(t=>t.id===conflict).error,null);
  assert.equal(await readFile(path.join(repo,'base.txt'),'utf8'),'task edit\n');
  await action({action:'approve_task',id:conflict});
  // Dirty main is reported without running Codex or changing the user's files.
  await writeFile(path.join(repo,'base.txt'),'uncommitted\n');
  const dirty=await create('Dirty');
  d=await until(d=>d.tasks.find(t=>t.id===dirty).error);
  assert.match(d.tasks.find(t=>t.id===dirty).error,/Commit or stash/);
  assert.equal(d.tasks.find(t=>t.id===dirty).thread_id,null);
  assert.equal(await readFile(path.join(repo,'base.txt'),'utf8'),'uncommitted\n');
  console.log('PASS: defaults, worker limit, isolated worktrees, sequential merges, orange pending state, restart, settings changes, conflicts, retry, dirty-main protection.');
} finally { if(server?.exitCode===null) await stop(); }
