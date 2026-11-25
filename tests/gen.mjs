import {spawn, spawnSync} from 'node:child_process';
import {mkdir, readFile, writeFile, readdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const root = path.resolve('tmp', `gen-${Date.now()}`);
const projectPath = path.join(root, 'project');
await mkdir(projectPath, {recursive:true});
await writeFile(path.join(projectPath, 'existing.txt'), 'Preserve local work');
const fake = path.join(root, 'fake-codex.exe');
assert.equal(spawnSync('rustc', ['tests/fake_codex.rs','-o',fake], {stdio:'inherit',windowsHide:true}).status, 0);
const url = 'http://127.0.0.1:3048';
const delay = ms=>new Promise(r=>setTimeout(r,ms));
let server, projectId;
async function state() { const r=await fetch(`${url}/api/state`); assert.equal(r.status,200); return (await r.json()).data; }
async function action(value) {
  const r=await fetch(`${url}/api/action`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(value)});
  const text=await r.text(); assert.equal(r.status,200,text); return JSON.parse(text);
}
async function until(predicate) {
  for(let i=0;i<180;i++) { const d=await state(); if(predicate(d))return d; await delay(100); }
  throw Error('Timed out waiting for Gen state');
}
async function start() {
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3048','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<80;i++){try{await state();return;}catch{await delay(100);}}
  throw Error('Gen test server did not start');
}
async function stop() { const done=new Promise(r=>server.once('exit',r)); server.kill(); await done; server=null; }
async function create(title) { return (await action({action:'create_task',project_id:projectId,title,text:'Investigate the project',difficulty:'gen',model:'gen-test-model',status:'todo'})).id; }
const generated = d=>d.tasks.filter(t=>t.difficulty==='hard');
try {
  await start();
  projectId=(await action({action:'add_project',name:'Gen project',path:projectPath})).id;
  await action({action:'settings',...(await state()).settings,gen_model:'gen-test-model',hard_model:'hard-review-model',parallelism:true});
  await action({action:'project_settings',id:projectId,auto_approve_easy:true,auto_approve_hard:true});
  const id=await create('GEN_SUCCESS');
  let d=await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');
  assert.equal(generated(d).length,2);
  for(const t of generated(d)) {
    assert.equal(t.status,'future'); assert.equal(t.model,'hard-review-model'); assert.equal(t.project_id,projectId);
    assert.match(t.messages[1].text,new RegExp(id));
  }
  let source=d.tasks.find(t=>t.id===id);
  assert.equal(source.worktree,null); assert.equal(source.awaiting_merge,false); assert.equal(source.attention,null);
  assert.equal(source.review_pending,false); assert.equal(source.cleanup_at,null);
  assert.match(source.messages.at(-2).text,/Created 2 Future task\(s\); skipped 1/);
  assert.deepEqual(await readdir(projectPath),['existing.txt']);
  assert.equal(await readFile(path.join(projectPath,'existing.txt'),'utf8'),'Preserve local work');

  await action({action:'send_message',id,text:'GEN_MORE_REQUEST: research another extension, do not implement'});
  d=await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');
  assert.equal(generated(d).length,3);
  source=d.tasks.find(t=>t.id===id);
  assert.equal(source.thread_id,'gen-session');
  const log=await readFile(path.join(root,'gen-runs.jsonl'),'utf8');
  assert.match(log,/resume/); assert.match(log,/Repair broken save/);
  await stop(); await start();
  assert.equal(generated(await state()).length,3);

  for(const [title,attention] of [['GEN_QUESTION','question'],['GEN_BLOCKED','blocked'],['GEN_REFUSED','refused'],['GEN_INVALID','failed'],['GEN_BAD_TASK','failed'],['GEN_FAIL','failed'],['GEN_EMPTY',null]]) {
    const key=await create(title);
    d=await until(d=>d.tasks.find(t=>t.id===key)?.status==='done');
    assert.equal(d.tasks.find(t=>t.id===key).attention,attention,title);
    assert.equal(generated(d).length,3,title);
  }
  const stopId=await create('GEN_STOP');
  await until(d=>d.tasks.find(t=>t.id===stopId)?.thread_id);
  await action({action:'stop_task',id:stopId});
  await until(d=>d.tasks.find(t=>t.id===stopId)?.status==='future');
  assert.equal(generated(await state()).length,3);
  const pendingId=await create('GEN_PENDING');
  await until(d=>d.tasks.find(t=>t.id===pendingId)?.thread_id);
  await action({action:'send_message',id:pendingId,text:'Continue researching, do not implement.'});
  d=await until(d=>d.tasks.find(t=>t.id===pendingId)?.status==='done');
  assert.ok(d.tasks.find(t=>t.id===pendingId).messages.some(m=>m.text==='Run ended. Your follow-up is queued.'));
  assert.equal(generated(d).length,3);
  const interruptedId=await create('GEN_STOP_SHUTDOWN');
  await until(d=>d.tasks.find(t=>t.id===interruptedId)?.thread_id);
  await stop(); await start();
  d=await state();
  assert.equal(d.tasks.find(t=>t.id===interruptedId).status,'future');
  assert.equal(generated(d).length,3);

  // Manual AI approval reviews proposals without starting implementation.
  await action({action:'ai_approve_task',id});
  d=await until(d=>!!d.tasks.find(t=>t.id===id)?.cleanup_at);
  assert.equal(generated(d).length,3);
  await action({action:'delete_task',id});
  assert.equal(generated(await state()).length,3);

  if(process.env.PLAYWRIGHT_MODULE) {
    const {chromium}=await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
    const browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
    try {
      const page=await browser.newPage({viewport:{width:1280,height:900}});
      const errors=[]; page.on('pageerror',e=>errors.push(e.message));
      await page.goto(url); await page.getByRole('button',{name:'Gen project',exact:true}).click();
      await page.locator('header .app-menu > summary').click();
      const menu=page.locator('header .menu-items');
      assert.deepEqual((await menu.locator('button').allTextContents()).slice(0,3),['Find issues','Gen more content','Gen new features']);
      await page.screenshot({path:path.join(root,'gen-menu.png')});
      await page.getByRole('button',{name:'Settings',exact:true}).click();
      await page.getByText('Gen task model',{exact:true}).waitFor();
      assert.equal(await page.locator('[data-model-input="gen_model"]').inputValue(),'gen-test-model');
      await page.getByRole('button',{name:'Close dialog'}).click();
      for(const label of ['Find issues','Gen more content','Gen new features']) {
        await page.locator('header .app-menu > summary').click();
        await page.getByRole('button',{name:label,exact:true}).click();
        assert.equal(await page.locator('[name="project_id"]').inputValue(),projectId);
        assert.ok((await page.locator('#task-instructions').inputValue()).length>80);
        if(label==='Find issues') {
          await page.getByRole('button',{name:'Start Gen',exact:true}).click();
          await page.locator('#dialog').waitFor({state:'hidden'});
          await until(d=>d.tasks.some(t=>t.title===label && t.difficulty==='gen' && t.status==='done'));
        } else {
          if(label==='Gen more content')await page.screenshot({path:path.join(root,'gen-content.png')});
          await page.getByRole('button',{name:'Close dialog'}).click();
        }
      }
      await page.locator('header .app-menu > summary').click();
      await page.getByRole('button',{name:'New task',exact:true}).click();
      await page.getByRole('button',{name:'Gen',exact:true}).waitFor();
      await page.getByRole('button',{name:'Close dialog'}).click();
      await page.locator('.task-difficulty.gen').first().waitFor();
      await page.locator('[data-status="future"] .task-card').filter({hasText:'Repair broken save'}).waitFor();
      await page.screenshot({path:path.join(root,'gen-board.png')});
      await page.getByRole('button',{name:'All projects',exact:true}).click();
      await page.locator('header .app-menu > summary').click();
      await page.getByRole('button',{name:'Find issues',exact:true}).click();
      assert.equal(await page.locator('[name="project_id"]').inputValue(),'');
      await page.getByRole('button',{name:'Close dialog'}).click();
      await page.setViewportSize({width:390,height:844});
      await page.locator('header .app-menu > summary').click();
      await page.getByRole('button',{name:'New task',exact:true}).click();
      await page.getByRole('button',{name:'Gen',exact:true}).waitFor();
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      await page.screenshot({path:path.join(root,'gen-mobile.png')});
      await page.getByRole('button',{name:'Close dialog'}).click();
      assert.deepEqual(errors,[]);
    } finally { await browser.close(); }
  }
  await action({action:'settings',...(await state()).settings,parallelism:false});
  const sequentialId=await create('GEN_EMPTY_SEQUENTIAL');
  d=await until(d=>d.tasks.find(t=>t.id===sequentialId)?.status==='done');
  assert.equal(d.tasks.find(t=>t.id===sequentialId).worktree,null);
  assert.equal(generated(d).length,3);
  const limitId=await create('GEN_LIMIT');
  d=await until(d=>d.codex_paused_until>Date.now() && d.tasks.find(t=>t.id===limitId)?.status==='todo');
  assert.equal(d.tasks.find(t=>t.id===limitId).status,'todo');
  assert.equal(generated(d).length,3);
  assert.deepEqual(await readdir(projectPath),['existing.txt']);
  await stop(); await start();
  assert.equal(generated(await state()).length,3);
  console.log('Gen research, Future task creation, lifecycle, and persistence checks passed');
} finally { if(server)await stop(); }
