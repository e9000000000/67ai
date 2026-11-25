import {spawn, spawnSync} from 'node:child_process';
import {mkdir} from 'node:fs/promises';
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
  const id=await create(a,'SHELL_TEST first');
  await until(d=>d.tasks.find(t=>t.id===id)?.shell_commands.some(c=>c.status==='running' && c.output==='<live output>'));
  let d=await until(d=>d.tasks.find(t=>t.id===id)?.status==='done');
  assert.equal(d.tasks.find(t=>t.id===id).shell_commands[0].exit_code,0);
  const second=await create(b,'SHELL_TEST second');
  await until(d=>d.tasks.find(t=>t.id===second)?.status==='done');

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
  assert.equal(await page.locator('#shell-commands details').count(),1);
  await page.locator('#shell-commands summary').click();
  assert.match(await page.locator('#shell-commands').innerText(), /<hello> done/);
  assert.doesNotMatch(await page.locator('#shell-commands').innerText(), /Project B/);
  assert.equal(await page.locator('#shell-commands pre *').count(),0);
  await page.screenshot({path:path.join(root,'shell-project.png')});
  await page.keyboard.press('Escape');
  await page.getByRole('button',{name:'All projects',exact:true}).click();
  await open();
  assert.equal(await page.locator('#shell-commands details').count(),2);
  await page.getByRole('button',{name:'Close dialog'}).click();

  const live=await create(a,'SHELL_TEST SHELL_WAIT WAIT_TEST');
  await until(d=>d.tasks.find(t=>t.id===live)?.shell_commands.some(c=>c.status==='running'));
  await open();
  await page.locator('.shell-status.running').waitFor();
  await page.getByText('<live output>',{exact:true}).waitFor();
  await action({action:'stop_task',id:live});
  await until(d=>d.tasks.find(t=>t.id===live)?.shell_commands[0].status==='interrupted');
  await page.locator('.shell-status.interrupted').waitFor();
  assert.equal(await page.locator('details[open] .shell-status.interrupted').count(),1);
  await page.setViewportSize({width:390,height:844});
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:path.join(root,'shell-mobile.png')});
  await page.keyboard.press('Escape');
  const empty=await createProject('Empty');
  await page.getByRole('button',{name:'Empty',exact:true}).click();
  await open();
  await page.getByText('No shell commands recorded for this selection yet.').waitFor();
  assert.deepEqual(errors,[]);
  await browser.close(); browser=null;

  const restart=await create(a,'SHELL_TEST SHELL_WAIT WAIT_TEST restart');
  await until(d=>d.tasks.find(t=>t.id===restart)?.shell_commands[0]?.status==='running');
  await stop(); await start();
  d=await state();
  assert.equal(d.tasks.find(t=>t.id===restart).shell_commands[0].status,'interrupted');
  assert.equal(d.tasks.find(t=>t.id===id).shell_commands[0].output,'<hello> done');
  assert.ok(empty);
  console.log(`Shell lifecycle, persistence, filtering, live UI and mobile checks passed. Screenshots: ${root}`);
} finally {
  if(browser)await browser.close();
  if(server)await stop();
}
