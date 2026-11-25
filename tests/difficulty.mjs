import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import assert from 'node:assert/strict';

const source = await readFile('web/app.js', 'utf8');
const form = {};
let submit, action, markup;
const context = {
  data: {projects:[{id:'p',name:'Project'}], settings:{hard_model:'hard-model',easy_model:'easy-model',gen_model:'gen-model'}},
  projectId:'p', escape:s=>s,
  $:selector=>selector.startsWith('button') ? {value:'easy'} : form,
  modal:(title, html)=>{markup=html;},
  attachFiles:()=>({upload:async()=>['attachment']}),
  formSubmit:handler=>{submit=handler;},
  api:async value=>{action=value;},
};
runInNewContext(source.slice(source.indexOf('function newTask('), source.indexOf('function openTask(')), context);
context.newTask('future');
assert.match(markup, /value="hard"/);
assert.match(markup, /value="easy"/);
const fields = new Map([['title','Task'],['text','Instructions'],['project_id','p']]);
for (const difficulty of ['hard','easy','gen']) {
  await submit(fields, {value:difficulty});
  assert.equal(action.model, `${difficulty}-model`);
  assert.equal(action.difficulty, difficulty);
  assert.equal(action.status, 'future');
  assert.equal(action.attachments[0], 'attachment');
}
await submit(fields);
assert.equal(action.model, 'easy-model');
assert.equal(action.difficulty, 'easy');
context.data.settings.easy_model = '';
await submit(fields, {value:'easy'});
assert.equal(action.model, '');
context.data.settings.easy_model = 'easy-model';
for (const modifier of ['ctrlKey','metaKey']) {
  let pending;
  form.onkeydown({key:'Enter',[modifier]:true,preventDefault(){},currentTarget:{requestSubmit:button=>{pending=submit(fields, button);}}});
  await pending;
  assert.equal(action.model, 'easy-model');
}
context.models = [];
context.document = {querySelectorAll:()=>[]};
context.$ = ()=>({insertAdjacentHTML(){}});
context.formEnd = ()=>'';
context.templates = ()=>{};
runInNewContext(source.slice(source.indexOf('function modelOptions('), source.indexOf('function renderBoard(')), context);
runInNewContext(source.slice(source.indexOf('function settingsModel('), source.indexOf("$('#project-tabs').onclick")), context);
context.settings();
assert.match(markup, /Hard task model/);
assert.match(markup, /Easy task model/);
assert.match(markup, /Gen task model/);

assert.doesNotMatch(markup, /default_model|Default model|Use default model/);
assert.match(markup, /Use Codex configuration/);
runInNewContext(source.slice(source.indexOf('function difficultyBadge('), source.indexOf('function modal(')), context);
for (const difficulty of ['easy','hard']) {
  assert.match(context.difficultyBadge({difficulty,model:'custom-model'}), new RegExp(`class="task-difficulty ${difficulty}"`));
  assert.match(context.difficultyBadge({model:`${difficulty}-model`}), new RegExp(`>${difficulty === 'easy' ? 'Easy' : 'Hard'}<`));
}
assert.equal(context.difficultyBadge({model:'unknown-model'}), '');
context.data.settings.hard_model = 'easy-model';
assert.equal(context.difficultyBadge({model:'easy-model'}), '');
assert.match(context.difficultyBadge({difficulty:'hard',model:'easy-model'}), />Hard</);
context.project = ()=>({name:'Project'});
context.needsAttention = ()=>false;
context.elapsed = ()=>0;
context.duration = ()=>'0s';
assert.match(context.card({id:'t',title:'Task',difficulty:'easy',status:'todo'}), /class="task-difficulty easy">Easy</);
console.log('Task difficulty and model settings checks passed');

context.project = key => context.data.projects.find(p=>p.id===key);
context.projectSettings('p');
assert.match(markup, /AI auto approve easy tasks/);
assert.match(markup, /AI auto approve hard tasks/);
await submit(new Map([['auto_approve_easy','on']]));
assert.equal(action.action,'project_settings');
assert.equal(action.id,'p');
assert.equal(action.auto_approve_easy,true);
assert.equal(action.auto_approve_hard,false);
await submit(new Map([['auto_approve_hard','on']]));
assert.equal(action.auto_approve_easy,false);
assert.equal(action.auto_approve_hard,true);
console.log('Per-project AI approval form checks passed');

assert.match(context.difficultyBadge({difficulty:'gen',model:'custom-model'}), />Gen</);
assert.equal(context.difficultyBadge({model:'gen-model'}), '');
runInNewContext(source.slice(source.indexOf('function genTask('), source.indexOf('function openTask(')), context);
context.$ = selector=>selector.startsWith('button') ? {value:'gen'} : form;
for (const kind of ['issues','content','features']) {
  context.genTask(kind);
  assert.match(markup, /Start Gen/);
  assert.doesNotMatch(markup, /value="hard"/);
  await submit(fields);
  assert.equal(action.difficulty, 'gen');
  assert.equal(action.model, 'gen-model');
  assert.equal(action.status, 'todo');
}
context.projectId='';
context.genTask('issues');
assert.match(markup, /Select a project/);
console.log('Gen preset and badge checks passed');
