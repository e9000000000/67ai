import {spawn, spawnSync} from 'node:child_process';
import {mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const root = path.resolve('tmp', `blockers-${Date.now()}`);
const projectPath = path.join(root, 'project');
await mkdir(projectPath, {recursive:true});
const fake = path.join(root, 'fake-codex.exe');
assert.equal(spawnSync('rustc', ['tests/fake_codex.rs','-o',fake], {stdio:'inherit',windowsHide:true}).status, 0);
const url = 'http://127.0.0.1:3049';
const delay = ms=>new Promise(r=>setTimeout(r,ms));
let server, browser;
async function state() { return (await (await fetch(`${url}/api/state`)).json()).data; }
async function action(value, status=200) {
  const response=await fetch(`${url}/api/action`, {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(value)});
  const text=await response.text(); assert.equal(response.status,status,text);
  return status===200 ? JSON.parse(text) : text;
}
async function until(predicate) {
  for(let i=0;i<100;i++) { const d=await state(); if(predicate(d))return d; await delay(100); }
  throw Error('Timed out waiting for blocker state');
}
async function start() {
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3049','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<80;i++){try{await state();return;}catch{await delay(100);}}
  throw Error('Test server did not start');
}
async function stop() { const done=new Promise(r=>server.once('exit',r)); server.kill(); await done; server=null; }
try {
  await start();
  const projectId=(await action({action:'add_project',name:'Blocker project',path:projectPath})).id;
  const create=async(title,status='future',blocker_id=null,text='Work')=>(await action({action:'create_task',project_id:projectId,title,text,status,model:'',blocker_id})).id;
  const blocker=await create('Prerequisite');
  let dependent;
  if(process.env.PLAYWRIGHT_MODULE) {
    const {chromium}=await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
    browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
    const page=await browser.newPage({viewport:{width:1280,height:900}});
    const errors=[]; page.on('pageerror',e=>errors.push(e.message));
    await page.goto(url);
    await page.locator('header .app-menu > summary').click();
    await page.getByRole('button',{name:'New task',exact:true}).click();
    await page.locator('[name="title"]').fill('Dependent');
    await page.locator('#task-instructions').fill('Work');
    await page.locator('[name="blocker_id"]').selectOption(blocker);
    await page.screenshot({path:path.join(root,'create-blocker.png')});
    await page.getByRole('button',{name:'Easy',exact:true}).click();
    await page.locator('#dialog').waitFor({state:'hidden'});
    const d=await until(d=>d.tasks.some(t=>t.title==='Dependent'));
    dependent=d.tasks.find(t=>t.title==='Dependent').id;
    await page.locator(`[data-task="${dependent}"]`).click();
    await page.locator('#task-blocker-status').filter({hasText:'Waiting for blocker: Prerequisite'}).waitFor();
    await page.locator('.task-menu > summary').click();
    assert.equal(await page.locator('#task-blocker').inputValue(),blocker);
    assert.equal(await page.locator(`#task-blocker option[value="${dependent}"]`).count(),0);
    const alternative=await create('Alternative');
    await page.locator(`#task-blocker option[value="${alternative}"]`).waitFor({state:'attached'});
    await page.locator('#task-blocker').selectOption(alternative);
    await until(d=>d.tasks.find(t=>t.id===dependent).blocker_id===alternative);
    await page.locator('#task-blocker').selectOption(blocker);
    await until(d=>d.tasks.find(t=>t.id===dependent).blocker_id===blocker);
    await page.screenshot({path:path.join(root,'task-blocker.png')});
    await page.getByRole('button',{name:'Close task',exact:true}).click();
    await page.setViewportSize({width:390,height:844});
    await page.locator('header .app-menu > summary').click();
    await page.getByRole('button',{name:'New task',exact:true}).click();
    await page.locator('[name="blocker_id"]').selectOption(blocker);
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:path.join(root,'blocker-mobile.png')});
    assert.deepEqual(errors,[]);
    await browser.close(); browser=null;
  } else { dependent=await create('Dependent','todo',blocker); }
  await delay(700);
  assert.equal((await state()).tasks.find(t=>t.id===dependent).status,'todo');
  await action({action:'set_blocker',id:blocker,blocker_id:dependent},400);
  await action({action:'delete_task',id:blocker},400);
  await stop(); await start();
  assert.equal((await state()).tasks.find(t=>t.id===dependent).blocker_id,blocker);
  await action({action:'move_task',id:blocker,status:'todo'});
  const d=await until(d=>d.tasks.find(t=>t.id===dependent)?.status==='done');
  assert.equal(d.tasks.find(t=>t.id===blocker).cleanup_at,null);
  assert.equal(d.tasks.find(t=>t.id===dependent).attention,null);
  const question=await create('Question','todo',null,'QUESTION_TEST');
  const waiting=await create('Wait for answer','todo',question);
  await until(d=>d.tasks.find(t=>t.id===question)?.attention==='question');
  await delay(700);
  assert.equal((await state()).tasks.find(t=>t.id===waiting).status,'todo');
  await action({action:'send_message',id:question,text:'ANSWER_TEST'});
  await until(d=>d.tasks.find(t=>t.id===waiting)?.status==='done');
  console.log('Blocker creation, editing, persistence, completion, question, and queue checks passed');
} finally { if(browser)await browser.close(); if(server)await stop(); }
