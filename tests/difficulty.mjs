import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import assert from 'node:assert/strict';

const source = await readFile('web/app.js', 'utf8');
const form = {};
let submit, action, markup;
const context = {
  data: {projects:[{id:'p',name:'Project'}], settings:{hard_model:'hard-model',easy_model:'easy-model',default_model:'old-model'}},
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
for (const difficulty of ['hard','easy']) {
  await submit(fields, {value:difficulty});
  assert.equal(action.model, `${difficulty}-model`);
  assert.equal(action.status, 'future');
  assert.equal(action.attachments[0], 'attachment');
}
await submit(fields);
assert.equal(action.model, 'easy-model');
context.data.settings.easy_model = '';
await submit(fields, {value:'easy'});
assert.equal(action.model, 'old-model');
let shortcut;
form.onkeydown({key:'Enter',ctrlKey:true,preventDefault(){},currentTarget:{requestSubmit:button=>{shortcut=button.value;}}});
assert.equal(shortcut, 'easy');
console.log('Task difficulty checks passed');
