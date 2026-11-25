import {spawn, spawnSync} from 'node:child_process';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const root=path.resolve('tmp',`content-${Date.now()}`), projectPath=path.join(root,'project');
await mkdir(projectPath,{recursive:true});
await writeFile(path.join(projectPath,'encounters.txt'),'Existing encounter system');
const fake=path.join(root,'fake-codex.exe');
assert.equal(spawnSync('rustc',['tests/fake_codex.rs','-o',fake],{stdio:'inherit',windowsHide:true}).status,0);
const url='http://127.0.0.1:3049', delay=ms=>new Promise(r=>setTimeout(r,ms));
let server, projectId;
async function state(){const r=await fetch(`${url}/api/state`);assert.equal(r.status,200);return (await r.json()).data;}
async function action(value,expected=200){
  const r=await fetch(`${url}/api/action`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(value)});
  const text=await r.text();assert.equal(r.status,expected,text);return expected===200?JSON.parse(text):text;
}
async function until(predicate){for(let i=0;i<200;i++){const d=await state();if(predicate(d))return d;await delay(100);}throw Error('Content test timed out');}
async function start(){
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3049','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<80;i++){try{await state();return;}catch{await delay(100);}}throw Error('Server failed');
}
async function stop(){const done=new Promise(r=>server.once('exit',r));server.kill();await done;server=null;}
async function editSaved(edit){await stop();const file=path.join(root,'67ai.json');const d=JSON.parse(await readFile(file,'utf8'));edit(d);await writeFile(file,JSON.stringify(d));await start();}
const brief={direction:'Quiet exploration',sample_scope:'One encounter',systems:'Encounter loader in encounters.txt; no levels',deliverables:'One encounter in sample.txt',integration:'Register sample.txt with encounters.txt',checks:'Loader check; respect no-build project rules',try_steps:'Open encounter menu and select Sample'};
const plan=(d,id)=>d.content_plans.find(p=>p.id===id);
const linked=(d,id)=>d.tasks.filter(t=>t.content_plan_id===id);
async function save(direction=brief.direction){return (await action({action:'save_content_brief',project_id:projectId,brief:{...brief,direction}})).id;}
const decide=(id,decision,feedback='Keep the quiet pacing',expected=200)=>action({action:'sample_decision',id,decision,feedback},expected);
const expand=(id,expected=200)=>action({action:'expand_content',id},expected);

try {
  await start();projectId=(await action({action:'add_project',name:'Sample game',path:projectPath})).id;
  await action({action:'settings',...(await state()).settings,hard_model:'hard-review-model',gen_model:'gen-test-model',paused:true});
  const id=await save();
  await expand(id,400);await decide(id,'accepted','Not built',400);
  const samples=await Promise.all(Array.from({length:5},()=>action({action:'make_sample',id})));
  assert.equal(new Set(samples.map(r=>r.id)).size,1);
  let d=await state(), sampleId=samples[0].id;
  assert.equal(linked(d,id).length,1);assert.equal(d.tasks.find(t=>t.id===sampleId).difficulty,'hard');
  assert.equal(plan(d,id).sample_status,'Queued');
  await decide(id,'accepted','Queued',400);
  await action({action:'move_task',id:sampleId,status:'done'});
  await decide(id,'accepted','Dragged to done is not completion',400);
  await action({action:'move_task',id:sampleId,status:'todo'});
  await action({action:'project_settings',id:projectId,auto_approve_easy:false,auto_approve_hard:true});
  await action({action:'settings',...(await state()).settings,paused:false});
  d=await until(d=>!!d.tasks.find(t=>t.id===sampleId)?.cleanup_at);
  assert.equal(plan(d,id).sample_ready,true);assert.equal(plan(d,id).decision,'pending');
  assert.match(plan(d,id).sample_report,/sample.txt/);await expand(id,400);
  await decide(id,'accepted','',400);
  await decide(id,'set_aside','Pacing too slow');await expand(id,400);
  await decide(id,'revise','Keep exploration; shorten the pause');
  d=await state();assert.equal(plan(d,id).sample_id,sampleId);assert.deepEqual(plan(d,id).sample_brief,brief);
  assert.equal(plan(d,id).decision,'pending');await expand(id,400);
  d=await until(d=>!!d.tasks.find(t=>t.id===sampleId)?.cleanup_at);
  assert.match(await readFile(path.join(root,'sample-prompt.txt'),'utf8'),/shorten the pause/);
  assert.equal(plan(d,id).decision,'pending');

  // Required merges, review, failed runs and manual Done cannot become creative acceptance.
  await editSaved(d=>{d.settings.paused=true;const t=d.tasks.find(t=>t.id===sampleId);t.cleanup_at=null;t.awaiting_merge=true;});
  assert.match(plan(await state(),id).sample_status,/merge/i);await decide(id,'accepted','Unmerged',400);await expand(id,400);
  await editSaved(d=>{const t=d.tasks.find(t=>t.id===sampleId);t.awaiting_merge=false;});
  await decide(id,'accepted','This pacing works');
  // Cleanup removes the conversation, but retains report, brief, acceptance and feedback.
  await action({action:'approve_task',id:sampleId});
  await editSaved(d=>{d.tasks.find(t=>t.id===sampleId).cleanup_at=1;});
  d=await until(d=>!d.tasks.some(t=>t.id===sampleId));
  assert.equal(plan(d,id).decision,'accepted');assert.match(plan(d,id).sample_report,/Try:/);
  assert.equal(plan(d,id).feedback.at(-1).text,'This pacing works');
  await stop();await start();assert.equal(plan(await state(),id).decision,'accepted');
  assert.equal((await action({action:'make_sample',id})).id,sampleId);
  assert.equal(linked(await state(),id).length,0);

  const research=(await expand(id)).id;assert.equal((await expand(id)).id,research);
  await action({action:'settings',...(await state()).settings,paused:false});
  d=await until(d=>d.tasks.find(t=>t.id===research)?.status==='done');
  let batch=linked(d,id).filter(t=>t.difficulty==='hard');
  assert.equal(batch.length,3);assert.ok(batch.every(t=>t.status==='future'));
  for(const t of batch){assert.match(t.messages.map(m=>m.text).join('\n'),/This pacing works/);assert.match(t.messages.map(m=>m.text).join('\n'),/sample.txt/);}
  assert.match(await readFile(path.join(root,'gen-runs.jsonl'),'utf8'),/Existing project tasks/);
  assert.ok(!d.tasks.some(t=>t.title==='Over budget'));
  await action({action:'delete_task',id:batch[2].id});
  await action({action:'send_message',id:research,text:'Research more units for this batch'});
  d=await until(d=>d.tasks.find(t=>t.id===research)?.status==='done');
  assert.equal(linked(d,id).filter(t=>t.difficulty==='hard').length,2);
  assert.equal(plan(d,id).batch_task_ids.length,3);
  assert.equal(d.tasks.find(t=>t.id===research).attention,null);
  await decide(id,'set_aside','Reconsider direction');
  await action({action:'move_task',id:batch[0].id,status:'todo'});
  await delay(900);assert.equal((await state()).tasks.find(t=>t.id===batch[0].id).status,'todo');
  await expand(id,400);
  await decide(id,'revise','Try a shorter encounter');
  d=await until(d=>plan(d,id).sample_ready);
  assert.notEqual(plan(d,id).sample_id,sampleId);assert.equal(plan(d,id).decision,'pending');
  // Setting aside during Gen discards the response rather than creating stale production work.
  await decide(id,'accepted','Short encounter works');
  const secondResearch=(await expand(id)).id;
  await until(d=>d.tasks.find(t=>t.id===secondResearch)?.thread_id);
  await decide(id,'set_aside','Hold this direction');
  d=await until(d=>d.tasks.find(t=>t.id===secondResearch)?.status==='done');
  assert.equal(d.tasks.find(t=>t.id===secondResearch).attention,'blocked');
  assert.equal(linked(d,id).filter(t=>t.difficulty==='hard').length,3);

  await action({action:'project_settings',id:projectId,auto_approve_easy:false,auto_approve_hard:false});
  for(const direction of ['SAMPLE_FAIL','SAMPLE_BLOCKED']){
    const key=await save(direction);const t=(await action({action:'make_sample',id:key})).id;
    d=await until(d=>d.tasks.find(x=>x.id===t)?.status==='done');
    assert.equal(plan(d,key).sample_ready,false);await decide(key,'accepted','Unfinished',400);await expand(key,400);
    assert.match(plan(d,key).sample_status,direction==='SAMPLE_FAIL'?/Failed/:/blocked/);
  }

  if(process.env.PLAYWRIGHT_MODULE){
    const {chromium}=await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
    const browser=await chromium.launch({channel:'msedge',headless:true,args:['--mute-audio']});
    try {
      const page=await browser.newPage({viewport:{width:1280,height:900}}),errors=[];
      page.on('pageerror',e=>errors.push(e.message));
      await page.goto(url);await page.getByRole('button',{name:'Sample game',exact:true}).click();
      await page.locator('header .app-menu > summary').click();await page.getByRole('button',{name:'Gen more content',exact:true}).click();
      await page.getByRole('button',{name:'New content brief'}).click();
      for(const [name,value] of Object.entries({...brief,direction:'Browser sample'}))await page.locator(`[name="${name}"]`).fill(value);
      await page.getByRole('button',{name:'Save brief',exact:true}).click();
      await page.getByRole('button',{name:'Make a sample',exact:true}).click();
      await page.getByText('Ready to try',{exact:true}).waitFor();
      const browserPlan=plan(await state(),(await state()).content_plans.find(p=>p.sample_brief?.direction==='Browser sample').id);
      assert.equal(linked(await state(),browserPlan.id).length,1);
      assert.equal(await page.getByRole('button',{name:'Research next bounded batch'}).isDisabled(),true);
      await page.locator('#sample-feedback').fill('Make the intro shorter');await page.getByRole('button',{name:'Revise sample',exact:true}).click();
      await page.locator('#sample-history').getByText('Make the intro shorter',{exact:false}).waitFor();
      await page.getByText('Ready to try',{exact:true}).waitFor();
      await page.locator('#sample-feedback').fill('Not this direction');await page.getByRole('button',{name:'Set aside',exact:true}).click();
      await page.locator('#sample-history').getByText('Not this direction',{exact:false}).waitFor();
      assert.equal(await page.getByRole('button',{name:'Research next bounded batch'}).isDisabled(),true);
      await page.locator('#sample-feedback').fill('After trying it, keep this pace');await page.getByRole('button',{name:'Use this direction',exact:true}).click();
      await page.getByRole('button',{name:'Research next bounded batch'}).click();
      await until(d=>linked(d,browserPlan.id).some(t=>t.difficulty==='gen'&&t.status==='done'));
      await page.screenshot({path:path.join(root,'sample-desktop.png'),fullPage:true});
      await page.setViewportSize({width:390,height:844});await page.screenshot({path:path.join(root,'sample-mobile.png'),fullPage:true});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      await page.reload();await page.locator('header .app-menu > summary').click();await page.getByRole('button',{name:'Gen more content',exact:true}).click();
      await page.getByRole('button',{name:/Browser sample ·/}).click();
      await page.getByText('After trying it, keep this pace',{exact:false}).waitFor();
      await page.getByRole('button',{name:'Open sample task',exact:true}).click();
      await page.getByRole('button',{name:'Content plan & sample feedback'}).click();
      await page.getByText('Ready to try',{exact:true}).waitFor();assert.deepEqual(errors,[]);
    } finally {await browser.close();}
  }
  console.log(`Content samples, decisions, cleanup and bounded research passed${process.env.PLAYWRIGHT_MODULE?' (including headless muted browser)':''}. Artifacts: ${root}`);
} finally {if(server)await stop();}
