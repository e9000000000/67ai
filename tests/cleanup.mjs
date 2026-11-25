import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile, access } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';

const root = path.resolve('tmp', `cleanup-${Date.now()}`);
await mkdir(root, {recursive:true});
const repo = path.join(root, 'project'), other = path.join(root, 'project-other');
await mkdir(repo); await mkdir(other);
function git(...args) {
  const r=spawnSync('git', args, {cwd:repo, encoding:'utf8', windowsHide:true});
  assert.equal(r.status,0,r.stderr); return r.stdout.trim();
}
git('init','-b','main'); git('config','user.name','Test'); git('config','user.email','test@example.com');
await writeFile(path.join(repo,'base.txt'),'base'); git('add','.'); git('commit','-m','Initial');
const fake=path.join(root,'fake-codex.exe');
assert.equal(spawnSync('rustc',['tests/fake_codex.rs','-o',fake],{stdio:'inherit',windowsHide:true}).status,0);
const url='http://127.0.0.1:3047'; let server;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function state(){return (await (await fetch(`${url}/api/state`)).json()).data;}
async function action(body,expected=200){
  const r=await fetch(`${url}/api/action`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
  const text=await r.text(); assert.equal(r.status,expected,text); return expected===200?JSON.parse(text):text;
}
async function start(){
  server=spawn(path.resolve('target/debug/67ai.exe'),[],{env:{...process.env,'67AI_PORT':'3047','67AI_DATA_DIR':root,'67AI_CODEX':fake},windowsHide:true,stdio:['ignore','ignore','pipe']});
  server.stderr.on('data',b=>process.stderr.write(b));
  for(let i=0;i<100;i++){try{await state();return;}catch{await delay(100);}} throw Error('Server did not start');
}
async function stop(){const exited=new Promise(r=>server.once('exit',r));server.kill();await exited;server=null;}
const exists=async p=>{try{await access(p);return true;}catch{return false;}};
const deleted=async()=>{try{return (await readFile(path.join(root,'deleted-sessions.jsonl'),'utf8')).trim().split('\n').map(s=>JSON.parse(s).params.threadId);}catch{return [];}};
async function seed(change){await stop();const file=path.join(root,'67ai.json');const d=JSON.parse(await readFile(file));change(d);await writeFile(file,JSON.stringify(d));await start();}
try{
  await start(); await action({action:'settings',...(await state()).settings,paused:true});
  const project_id=(await action({action:'add_project',name:'Project',path:repo})).id;
  await action({action:'add_project',name:'Other',path:other});
  const create=async title=>(await action({action:'create_task',project_id,title,text:'Work',model:'',status:'future'})).id;
  const taskId=await create('Delete me'), keepId=await create('Keep me'), expiredId=await create('Expired');
  const tree=path.join(root,'worktrees',taskId), keepTree=path.join(root,'worktrees',keepId), orphanTree=path.join(root,'worktrees','orphan'), expiredTree=path.join(root,'worktrees',expiredId);
  for(const [id,dir] of [[taskId,tree],[keepId,keepTree],['orphan',orphanTree],[expiredId,expiredTree]])git('worktree','add','-b',`67ai/${id}`,dir,'main');
  await writeFile(path.join(tree,'untracked.txt'),'Discard with task');
  await seed(d=>{
    Object.assign(d.tasks.find(t=>t.id===taskId),{worktree:tree,thread_id:'original',session_ids:['original','review','recovery']});
    Object.assign(d.tasks.find(t=>t.id===keepId),{worktree:keepTree,thread_id:'keep',session_ids:['keep-review']});
    Object.assign(d.tasks.find(t=>t.id===expiredId),{worktree:expiredTree,thread_id:'expired',status:'done',cleanup_at:Date.now()-1});
  });
  for(let i=0;i<60&&(await state()).tasks.some(t=>t.id===expiredId);i++)await delay(100);
  assert(!(await state()).tasks.some(t=>t.id===expiredId)); assert(!await exists(expiredTree)); assert((await deleted()).includes('expired'));
  await action({action:'delete_task',id:taskId});
  assert(!(await state()).tasks.some(t=>t.id===taskId)); assert(!await exists(tree));
  for(const id of ['original','review','recovery'])assert((await deleted()).includes(id));
  assert(!git('branch','--list',`67ai/${taskId}`)); assert(await exists(keepTree));
  const failed=await create('Failed cleanup');
  await seed(d=>{d.tasks.find(t=>t.id===failed).thread_id='fail-delete';});
  await action({action:'delete_task',id:failed},400); assert((await state()).tasks.some(t=>t.id===failed));
  await writeFile(path.join(root,'sessions.json'),JSON.stringify({data:[{id:'keep',cwd:keepTree},{id:'keep-review',cwd:keepTree},{id:'loose',cwd:repo},{id:'other',cwd:other}],nextCursor:'page2'}));
  await writeFile(path.join(root,'sessions-page2.json'),JSON.stringify({data:[{id:'orphan',cwd:orphanTree},{id:'nested',cwd:path.join(repo,'subfolder')},{id:'prefix',cwd:repo+'-unrelated'},{id:'escaped',cwd:path.join(repo,'..','outside')}],nextCursor:null}));
  await writeFile(path.join(root,'archived-sessions.json'),JSON.stringify({data:[{id:'archived',cwd:repo}],nextCursor:null}));
  const result=await action({action:'clear_project',id:project_id}); assert.equal(result.sessions,4); assert.equal(result.worktrees,1);
  const ids=await deleted(); for(const id of ['loose','orphan','nested','archived'])assert(ids.includes(id));
  for(const id of ['keep','keep-review','other','prefix','escaped'])assert(!ids.includes(id));
  assert(!await exists(orphanTree)); assert(await exists(repo)); assert(await exists(keepTree)); assert(!git('branch','--list','67ai/orphan'));
  await action({action:'clear_project',id:'missing'},400);
  // Existing tasks can share a session in migrated data; deleting one must preserve it.
  const shared=await create('Shared session');
  await seed(d=>{d.tasks.find(t=>t.id===shared).thread_id='keep';});
  await action({action:'delete_task',id:shared}); assert(!(await deleted()).includes('keep'));
  const active=(await action({action:'create_task',project_id,title:'WAIT_TEST cleanup',text:'WAIT_TEST',model:'',status:'todo'})).id;
  await action({action:'settings',...(await state()).settings,paused:false});
  for(let i=0;i<80&&!(await state()).tasks.find(t=>t.id===active).thread_id;i++)await delay(100);
  assert((await state()).tasks.find(t=>t.id===active).thread_id);
  await action({action:'clear_project',id:project_id},400);
  await action({action:'delete_task',id:active},400);
  await action({action:'stop_task',id:active});
  for(let i=0;i<80&&(await state()).tasks.find(t=>t.id===active).status==='now';i++)await delay(100);
  await action({action:'settings',...(await state()).settings,paused:true});
  // Render and submit the project menu form against its real handler.
  const source=await readFile('web/app.js','utf8'); let markup,submit,saved;
  const ui={data:{projects:[{id:project_id,name:'Project'}]},projectId:project_id,escape:s=>s,formEnd:s=>s,modal:(title,html)=>{markup=html;},formSubmit:f=>{submit=f;},api:async body=>{saved=body;return {sessions:2,worktrees:1};},toast:()=>{}};
  runInNewContext(source.slice(source.indexOf('function clearProject('),source.indexOf('function genTask(')),ui); ui.clearProject();
  assert.match(markup,/selected/); assert.match(markup,/Uncommitted files/); await submit(new Map([['id',project_id]])); assert.equal(saved.action,'clear_project'); assert.equal(saved.id,project_id);
  assert.match(await readFile('web/index.html','utf8'),/id="clear-project-button"/);
  console.log('PASS: task and timed deletion, auxiliary sessions, failure retention, project-scoped paginated/archived cleanup, assigned resources, UI form.');
}finally{if(server)await stop();}
