import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const {chromium}=await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE || 'tmp/browser/node_modules/playwright/index.mjs')).href);
const browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
const projects=[{id:'p',name:'Project',path:'C:/project'},{id:'other',name:'Other project',path:'C:/other'}];
const tasks=[{id:'t',project_id:'p',title:'Delete with cleanup',status:'future',messages:[],elapsed_ms:0,created_at:1}];
const actions=[],errors=[];
try {
  const page=await browser.newPage({viewport:{width:1280,height:900}});
  page.on('pageerror',e=>errors.push(e.message));
  await page.route('http://localhost:3058/**',async route=>{
    const url=new URL(route.request().url());
    if(url.pathname==='/api/state')return route.fulfill({json:{data:{projects,tasks,settings:{},content_plans:[],saved_at:1},models:[],server_time:Date.now()}});
    if(url.pathname==='/api/action'){
      const action=route.request().postDataJSON();actions.push(action);
      if(action.action==='delete_task')tasks.splice(0,tasks.length);
      return route.fulfill({json:{ok:true,sessions:2,worktrees:1}});
    }
    if(url.pathname.startsWith('/api/'))return route.fulfill({json:{}});
    const file=url.pathname==='/'?'index.html':url.pathname.slice(1);
    return route.fulfill({body:await readFile(path.join('web',file)),contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
  });
  await page.goto('http://localhost:3058/');
  await page.locator('[data-project="p"]').click();
  await page.locator('summary[aria-label="Application menu"]').click();
  await page.getByRole('button',{name:'Clear unused sessions and worktrees',exact:true}).click();
  await page.getByRole('heading',{name:'Clear unused sessions and worktrees'}).waitFor();
  assert.equal(await page.locator('#modal-form select').inputValue(),'p');
  await mkdir('tmp/cleanup-ui',{recursive:true});
  await page.screenshot({path:'tmp/cleanup-ui/desktop.png'});
  await page.getByRole('button',{name:'Clear unused',exact:true}).click();
  await page.locator('#dialog').waitFor({state:'hidden'});
  assert.deepEqual(actions.at(-1),{action:'clear_project',id:'p'});
  assert.match(await page.locator('#toast').innerText(),/Deleted 2 session/);
  await page.locator('[data-task="t"]').click();
  await page.locator('summary[aria-label="Task menu"]').click();
  await page.getByRole('button',{name:'Delete task',exact:true}).click();
  assert.match(await page.locator('#dialog').innerText(),/Codex sessions, task worktree, and branch/);
  await page.getByRole('button',{name:'Delete task',exact:true}).last().click();
  await page.locator('#dialog').waitFor({state:'hidden'});
  assert.deepEqual(actions.at(-1),{action:'delete_task',id:'t'});
  assert.equal(await page.locator('[data-task="t"]').count(),0);
  await page.setViewportSize({width:390,height:844});
  await page.locator('summary[aria-label="Application menu"]').click();
  await page.getByRole('button',{name:'Clear unused sessions and worktrees',exact:true}).click();
  await page.locator('#modal-form select').selectOption('other');
  await page.screenshot({path:'tmp/cleanup-ui/mobile.png'});
  assert(await page.locator('#modal-form button[type="submit"]').isVisible());
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.getByRole('button',{name:'Clear unused',exact:true}).click();
  await page.locator('#dialog').waitFor({state:'hidden'});
  assert.deepEqual(actions.at(-1),{action:'clear_project',id:'other'});
  assert.deepEqual(errors,[]);
  console.log('PASS: headless desktop/mobile project cleanup and task deletion menu flows.');
} finally {await browser.close();}
