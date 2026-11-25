import {spawn, spawnSync} from 'node:child_process';
import {mkdir, writeFile} from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';

const root = path.resolve('tmp', `shell-${Date.now()}`);
await mkdir(root, {recursive:true});
const fake = path.join(root, 'fake-codex.exe');
assert.equal(spawnSync('rustc', ['tests/fake_codex.rs', '-o', fake], {stdio:'inherit', windowsHide:true}).status, 0);
const url = 'http://127.0.0.1:3052';
const delay = ms => new Promise(r => setTimeout(r, ms));
let server, browser;
async function state() { return (await (await fetch(`${url}/api/state`)).json()).data; }
async function action(value) {
  const r = await fetch(`${url}/api/action`, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(value)});
  assert.equal(r.status, 200, await r.clone().text()); return r.json();
}
async function until(predicate) {
  for(let i=0;i<150;i++) { const d=await state(); if(predicate(d))return d; await delay(100); }
  throw Error('Timed out waiting for shell state');
}
async function start() {
  server=spawn(path.resolve('target/debug/67ai.exe'), [], {env:{...process.env,'67AI_PORT':'3052','67AI_DATA_DIR':root,'67AI_CODEX':fake}, windowsHide:true, stdio:['ignore','ignore','pipe']});
  server.stderr.on('data', b=>process.stderr.write(b));
  for(let i=0;i<80;i++) { try { await state(); return; } catch { await delay(100); } }
  throw Error('Server did not start');
}
async function stop() { const done=new Promise(r=>server.once('exit',r)); server.kill(); await done; server=null; }
try {
  await start();
  const createProject = async name => {
    const dir=path.join(root,name); await mkdir(dir);
    return (await action({action:'add_project',name,path:dir})).id;
  };
  const a=await createProject('Project A'), b=await createProject('Project B');
  const create = async (project_id,title) => (await action({action:'create_task',project_id,title,text:title,model:'',status:'todo'})).id;
  const shells = async () => {
    const r=await fetch(`${url}/api/shells`);
    assert.equal(r.status,200,await r.clone().text()); return r.json();
  };
  const waitShells = async predicate => {
    for(let i=0;i<150;i++) { const items=await shells(); if(predicate(items))return items; await delay(100); }
    throw Error('Timed out waiting for live processes');
  };
  // Command events alone must never appear as shells.
  const command=await create(a,'SHELL_TEST command history');
  await until(d=>d.tasks.find(t=>t.id===command)?.status==='done');
  assert.deepEqual(await shells(),[]);
  const id=await create(a,'SHELL_PROCESS first');
  await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');
  const first=(await waitShells(s=>s.some(s=>s.task_id===id))).find(s=>s.task_id===id);
  assert.equal(first.program,'fake-codex.exe');
  assert.ok(first.pid>0);
  const second=await create(b,'SHELL_PROCESS second');
  await until(d=>d.tasks.find(t=>t.id===second)?.status==='done');
  await waitShells(s=>s.length===2);

  const {chromium}=await import(pathToFileURL(path.resolve('tmp/browser/node_modules/playwright/index.mjs')).href);
  browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
  const page=await browser.newPage({viewport:{width:1280,height:900}});
  const errors=[]; page.on('pageerror',e=>errors.push(e.message));
  await page.goto(url);
  await page.getByRole('button',{name:'Project A',exact:true}).click();
  const open = async () => {
    await page.locator('header .app-menu > summary').click();
    await page.getByRole('button',{name:'Shell inspection',exact:true}).click();
  };
  await open();
  await page.waitForFunction(()=>document.querySelectorAll('#shell-commands details').length===1);
  await page.locator('#shell-commands summary').click();
  assert.match(await page.locator('#shell-commands').innerText(), /fake-codex.exe/);
  assert.match(await page.locator('#shell-commands').innerText(), new RegExp(`PID ${first.pid}`));
  assert.doesNotMatch(await page.locator('#shell-commands').innerText(), /Project B|echo <hello>/);
  assert.equal(await page.locator('#shell-commands pre *').count(),0);
  await page.screenshot({path:path.join(root,'shell-project.png')});
  await page.keyboard.press('Escape');
  await page.getByRole('button',{name:'All projects',exact:true}).click();
  await open();
  await page.waitForFunction(()=>document.querySelectorAll('#shell-commands details').length===2);
  await page.getByRole('button',{name:'Close dialog'}).click();

  const live=await create(a,'SHELL_PROCESS WAIT_TEST');
  await waitShells(s=>s.some(s=>s.task_id===live));
  await open();
  await page.waitForFunction(()=>document.querySelectorAll('#shell-commands details').length===3);
  await action({action:'stop_task',id:live});
  await until(d=>d.tasks.find(t=>t.id===live)?.status==='future');
  await waitShells(s=>!s.some(s=>s.task_id===live));
  await page.waitForFunction(()=>document.querySelectorAll('#shell-commands details').length===2);
  // A real process exit removes the row while the inspection stays open.
  await writeFile(path.join(root,'Project A','shell.stop'),'');
  await waitShells(s=>!s.some(s=>s.task_id===id));
  await page.waitForFunction(()=>document.querySelectorAll('#shell-commands details').length===1);
  await page.setViewportSize({width:390,height:844});
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:path.join(root,'shell-mobile.png')});
  await page.keyboard.press('Escape');
  const empty=await createProject('Empty');
  await page.getByRole('button',{name:'Empty',exact:true}).click();
  await open();
  await page.getByText('No running shells for this selection.').waitFor();
  assert.deepEqual(errors,[]);
  await browser.close(); browser=null;

  const failed=await create(a,'SHELL_PROCESS FAIL_TEST');
  await until(d=>d.tasks.find(t=>t.id===failed)?.status==='done');
  await waitShells(s=>!s.some(s=>s.task_id===failed));
  assert.equal((await shells()).length,1);
  await stop(); await start();
  assert.deepEqual(await shells(),[]);
  assert.ok(empty);
  console.log(`Live processes, completed agents, exit/stop/failure cleanup, project filtering and desktop/mobile UI checks passed. Screenshots: ${root}`);
} finally {
  if(browser)await browser.close();
  if(server)await stop();
}
