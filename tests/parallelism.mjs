import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';
import { pathToFileURL } from 'node:url';

const source = await readFile('web/app.js','utf8');
let markup, submit, saved;
const ui = {
  data:{projects:[],settings:{max_workers:4,parallelism:false}}, models:[], projectId:'p',
  escape:s=>s??'', formEnd:()=>'', templates(){}, document:{querySelectorAll:()=>[]},
  $:()=>({insertAdjacentHTML(){}}), modal:(title,html)=>{markup=html;},
  formSubmit:handler=>{submit=handler;}, api:value=>{saved=value;},
  duration:()=>'', elapsed:()=>0,
};
runInNewContext(source.slice(source.indexOf('function modelOptions('), source.indexOf('function renderBoard(')),ui);
runInNewContext(source.slice(source.indexOf('function settingsModel('), source.indexOf("$('#project-tabs').onclick")),ui);
ui.settings();
assert.match(markup,/name="max_workers"[^>]*value="4"/);
assert.doesNotMatch(markup,/name="parallelism"[^>]*checked/);
await submit(new Map([['max_workers','3'],['parallelism','on']]));
assert.equal(saved.max_workers,3); assert.equal(saved.parallelism,true);
await submit(new Map([['max_workers','1']]));
assert.equal(saved.parallelism,false);
runInNewContext(source.slice(source.indexOf('const needsAttention'),source.indexOf('const duration')) + source.slice(source.indexOf('function difficultyBadge('),source.indexOf('function modal(')),ui);
const orange=ui.card({id:'t',title:'Completed',status:'done',awaiting_merge:true,error:'Conflict'});
assert.match(orange,/awaiting-merge/); assert.match(orange,/Merge needs attention/);
assert.match(orange,/draggable="false"/); assert.doesNotMatch(orange,/needs-attention/);
assert.doesNotMatch(ui.card({id:'t',title:'Merged',status:'done'}),/awaiting-merge/);
const receipt={role:'system',text:'Merged into main.\nCommit: abc123\nProject: C:\\project'};
const completed={id:'t',title:'Completed',status:'done',messages:[{role:'system',text:'Codex started working.'},receipt]};
assert.match(ui.card(completed),/class="merge-receipt"/);
assert.match(ui.card(completed),/Commit: abc123/);
assert.match(ui.mergeReceipt({...completed,messages:[{role:'system',text:'Merged into main.'}]}),/>Merged into main</);
for(const changed of [{status:'todo'},{status:'now'},{awaiting_merge:true},{merging:true},{attention:'blocked'},{error:'failed'},
  {messages:[receipt,{role:'system',text:'Codex started working.'}]}])assert.equal(ui.mergeReceipt({...completed,...changed}),'');
assert.equal(ui.mergeReceipt({...completed,messages:[{role:'assistant',text:'Merged into main.'}]}),'');

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
let server, presenceTimer;
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
  server=spawn(path.resolve('target/debug/67ai.exe'), [], {env:{...process.env,'67AI_PORT':'3034','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data', b=>process.stderr.write(b));
  for(let i=0;i<50;i++) { try {
    await state();
    // Keep this isolated test server silent through the normal focus suppression.
    const presence=()=>fetch(`${url}/api/presence`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:'silent-test',focused:true,rendered_at:0})}).catch(()=>{});
    await presence(); presenceTimer=setInterval(presence,500); return;
  } catch { await delay(100); } }
  throw Error('Server did not start');
}
async function stop() { clearInterval(presenceTimer); const exited=new Promise(resolve=>server.once('exit',resolve)); server.kill(); await exited; }
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

  settings={...settings,parallelism:true,paused:true,hard_model:'merge-fail'};
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
  // A successful model response alone must not count as resolving conflicts.
  settings={...settings,hard_model:'merge-noop'};
  await action({action:'settings',...settings,paused:false});
  await action({action:'retry_merge',id:conflict});
  d=await until(d=>d.tasks.find(t=>t.id===conflict).error);
  assert.match(d.tasks.find(t=>t.id===conflict).error,/unresolved conflicts/);
  assert.equal(git(repo,'status','--porcelain'),'');
  // Retry invokes the resolver again, then commits without manual Git repair.
  settings={...settings,hard_model:''};
  await action({action:'settings',...settings,paused:false});
  await action({action:'retry_merge',id:conflict});
  d=await until(d=>!d.tasks.find(t=>t.id===conflict).awaiting_merge);
  const merged=d.tasks.find(t=>t.id===conflict);
  assert.equal(merged.error,null);
  assert.equal(merged.thread_id,'parallel-test');
  assert(merged.session_ids.includes('parallel-test'));assert(merged.session_ids.includes('merge-repair'));
  assert.equal(merged.attention,null);
  assert(merged.messages.some(m=>m.text.includes('Resolving merge conflicts automatically')));
  const mergeMessage=merged.messages.findLast(m=>m.role==='system'&&m.text.startsWith('Merged into main.'));
  assert(mergeMessage.text.includes(`Commit: ${git(repo,'rev-parse','HEAD')}`));
  assert(mergeMessage.text.includes(`Project: ${repo}`));
  assert(mergeMessage.text.includes('not restarted'));
  assert.equal(await readFile(path.join(repo,'base.txt'),'utf8'),'main edit\ntask edit\n');
  assert.equal(git(repo,'status','--porcelain'),'');
  git(repo,'merge-base','--is-ancestor',git(tree,'rev-parse','HEAD'),'HEAD');
  assert.equal(git(repo,'rev-list','--parents','-n','1','HEAD').split(' ').length,3);
  await action({action:'approve_task',id:conflict});
  // A fresh conflicting task completes its merge without any retry action.
  await action({action:'settings',...settings});
  const automatic=await create('Automatic');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>d.tasks.find(t=>t.id===automatic).thread_id);
  const automaticTree=d.tasks.find(t=>t.id===automatic).worktree;
  await action({action:'settings',...settings});
  await until(d=>d.tasks.find(t=>t.id===automatic).awaiting_merge);
  await writeFile(path.join(repo,'base.txt'),'new main edit\n');
  git(repo,'add','.'); git(repo,'commit','-m','Another main edit');
  await writeFile(path.join(automaticTree,'base.txt'),'new task edit\n');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>!d.tasks.find(t=>t.id===automatic).awaiting_merge);
  assert.equal(d.tasks.find(t=>t.id===automatic).error,null);
  assert(d.tasks.find(t=>t.id===automatic).messages.some(m=>m.text.includes('Resolving merge conflicts automatically')));
  assert.equal(git(repo,'status','--porcelain'),'');
  // Raw tasks run in the project folder, including folders without Git.
  settings={...settings,paused:true};
  await action({action:'settings',...settings});
  const rawPath=path.join(root,'raw-project'); await mkdir(rawPath);
  const rawProject=(await action({action:'add_project',name:'Raw project',path:rawPath})).id;
  let raw;
  if(process.env.PLAYWRIGHT_MODULE) {
    const {chromium}=await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
    const browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
    try {
      const page=await browser.newPage({viewport:{width:1280,height:900}});
      await page.goto(url);
      const mergedCard=page.locator(`[data-task="${conflict}"]`);
      await mergedCard.locator('.merge-receipt').waitFor();
      assert.match(await mergedCard.locator('.merge-receipt').getAttribute('title'),/Commit: [0-9a-f]{40}/);
      await mergedCard.click();
      await page.locator('#panel-difficulty .merge-receipt').waitFor();
      assert.equal(await page.locator('#panel-difficulty .merge-receipt').innerText(),'Merged into main');
      await page.screenshot({path:path.join(root,'merge-receipt.png')});
      await page.locator('#close-task').click();
      await page.locator('header .app-menu > summary').click();
      await page.getByRole('button',{name:'New task',exact:true}).click();
      assert.deepEqual(await page.locator('[name="execution_mode"] option').allTextContents(),['parallel-worktree','parallel-raw']);
      assert.equal(await page.locator('[name="execution_mode"]').inputValue(),'parallel-worktree');
      const boxes=await Promise.all(['project_id','blocker_id','execution_mode'].map(name=>page.locator(`[name="${name}"]`).boundingBox()));
      assert(boxes.every(box=>box&&Math.abs(box.y-boxes[0].y)<2));
      await page.locator('[name="title"]').fill('Raw');
      await page.locator('[name="text"]').fill('PARALLEL_TEST');
      await page.locator('[name="project_id"]').selectOption(rawProject);
      await page.locator('[name="execution_mode"]').selectOption('parallel-raw');
      await page.screenshot({path:path.join(root,'raw-task.png')});
      await page.getByRole('button',{name:'Easy',exact:true}).click();
      await page.locator('#dialog').waitFor({state:'hidden'});
      raw=(await state()).tasks.find(t=>t.title==='Raw').id;
      await action({action:'settings',...settings,parallelism:false});
      await page.reload();
      await page.locator('header .app-menu > summary').click();
      await page.getByRole('button',{name:'New task',exact:true}).click();
      assert.equal(await page.locator('[name="execution_mode"]').count(),0);
    } finally { await browser.close(); }
  } else {
    raw=(await action({action:'create_task',project_id:rawProject,title:'Raw',text:'PARALLEL_TEST',status:'todo',model:'',execution_mode:'parallel-raw'})).id;
  }
  await stop(); await start();
  assert.equal((await state()).tasks.find(t=>t.id===raw).execution_mode,'parallel-raw');
  await action({action:'settings',...settings,paused:false});
  d=await until(d=>d.tasks.find(t=>t.id===raw).status==='done');
  const rawTask=d.tasks.find(t=>t.id===raw);
  assert.equal(rawTask.error,null); assert.equal(rawTask.worktree,null); assert.equal(rawTask.awaiting_merge,false);
  const rawPrompt=await readFile(path.join(rawPath,'Raw.txt'),'utf8');
  assert.doesNotMatch(rawPrompt,/Work only in this task's Git worktree/);
  assert.equal(rawTask.execution_mode,'parallel-raw');
  await action({action:'send_message',id:raw,text:'Follow up'});
  await until(d=>d.tasks.find(t=>t.id===raw).status==='done');
  assert.equal((await state()).tasks.find(t=>t.id===raw).worktree,null);
  // Dirty main is reported without running Codex or changing the user's files.
  await writeFile(path.join(repo,'base.txt'),'uncommitted\n');
  const dirty=await create('Dirty');
  d=await until(d=>d.tasks.find(t=>t.id===dirty).error);
  assert.match(d.tasks.find(t=>t.id===dirty).error,/Select a hard-task model/);
  assert.equal(d.tasks.find(t=>t.id===dirty).thread_id,null);
  assert.equal(await readFile(path.join(repo,'base.txt'),'utf8'),'uncommitted\n');
  console.log('PASS: defaults, worker limit, isolated worktrees, sequential merges, orange pending state, restart, settings changes, conflicts, retry, raw execution, persisted modes, follow-ups, dirty-main protection.');
} finally { if(server?.exitCode===null) await stop(); }
