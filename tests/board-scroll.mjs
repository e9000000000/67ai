import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

const {chromium} = await import(pathToFileURL(path.resolve('tmp/browser/node_modules/playwright/index.mjs')).href);
const browser = await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
try {
  const page = await browser.newPage({viewport:{width:1280,height:900}});
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  const states = ['future','todo','now','done'];
  const tasks = states.flatMap(status => Array.from({length:50}, (_, i) => ({
    id:`${status}-${i}`, project_id:'project', title:`${status} task ${i}`, status,
    created_at:100-i, queued_at:i, elapsed_ms:0, started_at:status==='now'?Date.now():null,
  })));
  let revision = 0;
  await page.route('http://localhost:3059/**', async route => {
    const url = new URL(route.request().url());
    if (url.pathname === '/api/state') {
      tasks.find(t => t.id==='now-0').title = `Running update ${++revision}`;
      return route.fulfill({json:{data:{projects:[{id:'project',name:'Project'}],tasks,
        settings:{},content_plans:[],saved_at:revision},models:[],server_time:Date.now()}});
    }
    if (url.pathname.startsWith('/api/')) return route.fulfill({json:{}});
    const file = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
    const contentType = file.endsWith('.css') ? 'text/css' : file.endsWith('.js') ? 'text/javascript' : 'text/html';
    return route.fulfill({body:await readFile(path.join('web',file)),contentType});
  });
  await page.goto('http://localhost:3059/');
  await page.waitForFunction(() => document.querySelectorAll('.task-card').length===200);
  const scrollColumns = () => [...document.querySelectorAll('.column-tasks')].map(el => el.scrollTop);
  await page.evaluate(() => document.querySelectorAll('.column-tasks').forEach((el,i) => { el.scrollTop=300+i*150; }));
  const positions = await page.evaluate(scrollColumns);
  assert.ok(positions.every(top => top>0));
  for (let i=0;i<3;i++) {
    await page.evaluate(() => refresh());
    assert.deepEqual(await page.evaluate(scrollColumns),positions,'Live refresh must preserve every column scroll position');
  }
  tasks.find(t => t.id==='now-1').status='done';
  await page.evaluate(() => refresh());
  assert.equal(await page.locator('[data-status="done"] [data-task="now-1"]').count(),1);
  assert.deepEqual(await page.evaluate(scrollColumns),positions,'Task status changes must preserve scroll');

  await page.setViewportSize({width:390,height:844});
  await page.evaluate(() => { document.querySelector('#board').scrollTop=1500; });
  const mobileTop = await page.locator('#board').evaluate(el => el.scrollTop);
  assert.ok(mobileTop>0);
  await page.evaluate(() => refresh());
  assert.equal(await page.locator('#board').evaluate(el => el.scrollTop),mobileTop,'Mobile board must preserve scroll');
  assert.deepEqual(errors,[]);
  console.log('Task board scroll survives live updates and status changes on desktop and mobile.');
} finally {
  await browser.close();
}
