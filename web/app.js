const $ = (s, root = document) => root.querySelector(s);
const escape = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
let data, models = [], projectId = '', selectedTask = '', lastBoard = '', lastMessages = '', serverOffset = 0;
const states = ['future', 'todo', 'now', 'done'];
const project = key => data.projects.find(p => p.id === key);
const task = key => data.tasks.find(t => t.id === key);
const needsAttention = t => t.status === 'done' && !!(t.attention || t.error);
const attentionLabel = t => ({question:'Question — answer needed',blocked:'Blocked — needs your input',refused:'Refused — needs your input',failed:'Run failed',unconfirmed:'Completion not confirmed'}[t.attention] || 'Needs attention');
const duration = ms => { const seconds = Math.max(0, Math.floor(ms / 1000)); return seconds >= 3600 ? `${Math.floor(seconds/3600)}h ${Math.floor(seconds%3600/60)}m` : seconds >= 60 ? `${Math.floor(seconds/60)}m ${seconds%60}s` : `${seconds}s`; };
const cleanupLabel = t => `Approved - cleanup in ${duration(t.cleanup_at-Date.now()-serverOffset)}`;
const elapsed = t => t.elapsed_ms + (t.started_at ? Math.max(0, Date.now()+serverOffset-t.started_at) : 0);
function toast(text) { $('#toast').textContent = text; $('#toast').hidden = false; clearTimeout(toast.timer); toast.timer = setTimeout(() => $('#toast').hidden = true, 4500); }
async function api(action) { const res = await fetch('/api/action', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(action)}); if (!res.ok) throw Error(await res.text()); const result = await res.json(); await refresh(); return result; }
async function act(action) { try { return await api(action); } catch (e) { toast(e.message); } }
function renderWeeklyLimit(result) {
  const el = $('#weekly-limit');
  const limits = result.rateLimitsByLimitId?.codex ?? result.rateLimits;
  const week = [limits?.primary, limits?.secondary].find(w => w?.windowDurationMins === 7 * 24 * 60);
  const available = typeof week?.usedPercent === 'number' && Number.isFinite(week.usedPercent);
  const remaining = available ? Math.max(0, Math.min(100, 100 - week.usedPercent)) : null;
  el.textContent = available ? `Codex weekly: ${Number(remaining.toFixed(1))}% left` : 'Codex weekly: unavailable';
  el.title = available ? `Weekly Codex limit remaining${week.resetsAt ? `. Resets ${new Date(week.resetsAt * 1000).toLocaleString()}` : ''}` : result.error || 'No weekly limit reported for this Codex account.';
  el.classList.toggle('low', available && remaining <= 10);
}
async function pollLimits() {
  try {
    const res = await fetch('/api/rate-limits');
    if (!res.ok) throw Error('Could not read Codex usage.');
    renderWeeklyLimit(await res.json());
  } catch (e) { renderWeeklyLimit({error:e.message}); }
  setTimeout(pollLimits, 60000);
}
async function refresh() {
  const res = await fetch('/api/state'); if (!res.ok) throw Error(await res.text());
  const result = await res.json(); data = result.data; models = result.models; serverOffset = result.server_time-Date.now();
  disconnected = false;
  document.title = data.tasks.some(needsAttention) ? 'QUESTION' : `67ai-${data.tasks.filter(t => t.status === 'todo').length}-${data.tasks.filter(t => t.status === 'now').length}`;
  renderBoard(); if (selectedTask) refreshPanel();
}
function modelOptions(value = '', withDefault = true) {
  const all = [...new Set([...models, data.settings.default_model, value].filter(Boolean))];
  return (withDefault ? `<option value="">Default${data.settings.default_model ? ' · '+escape(data.settings.default_model) : ' · Codex configuration'}</option>` : '') + all.map(m => `<option value="${escape(m)}" ${m===value?'selected':''}>${escape(m)}</option>`).join('');
}
function renderBoard() {
  const signature = JSON.stringify([data, projectId]);
  if (lastBoard === signature) return; lastBoard = signature;
  $('#project-tabs').innerHTML = [{id:'',name:'All projects'}, ...data.projects].map(p => `<button class="project-tab ${p.id===projectId?'active':''}" data-project="${escape(p.id)}" aria-current="${p.id===projectId?'page':'false'}" title="${escape(p.path || 'All projects')}">${escape(p.name)}</button>`).join('');
  const tasks = data.tasks.filter(t => !projectId || t.project_id === projectId);
  $('#board').innerHTML = states.map(status => {
    const items = tasks.filter(t => t.status === status).sort((a,b) => status === 'todo' ? a.queued_at-b.queued_at : b.created_at-a.created_at);
    return `<section class="column" data-status="${status}" aria-label="${status} tasks"><div class="column-head"><h2>${status[0].toUpperCase()+status.slice(1)} <span class="column-count">${items.length}</span></h2><button class="icon-button" data-new="${status === 'future' ? 'future' : 'todo'}" aria-label="Add task from ${status}">+</button></div><div class="column-tasks">${items.map(card).join('') || '<p class="empty-column">No tasks</p>'}</div></section>`;
  }).join('');
}
function card(t) {
  return `<button class="task-card ${t.status==='now'?'running':''} ${needsAttention(t)?'needs-attention':''}" data-task="${t.id}" draggable="${t.status!=='now'}"><h3>${escape(t.title)}</h3>${t.cleanup_at ? `<span class="approval-countdown" data-cleanup="${t.id}">${cleanupLabel(t)}</span>` : ''}<span class="card-meta">${!projectId ? '<span>'+escape(project(t.project_id)?.name)+'</span>' : ''}${needsAttention(t)||t.error ? `<span class="card-error">${escape(attentionLabel(t))}</span>` : ''}<span data-timer="${t.id}">${duration(elapsed(t))}</span></span></button>`;
}
function modal(title, body, wide = false, fullscreen = false) {
  const dialog = $('#dialog'); dialog.classList.toggle('wide', wide); dialog.classList.toggle('fullscreen', fullscreen);
  $('#dialog-content').innerHTML = `<div class="dialog-head"><h2>${escape(title)}</h2><button class="icon-button" id="close-dialog" aria-label="Close dialog">×</button></div><div class="dialog-body">${body}</div>`;
  $('#close-dialog').onclick = () => dialog.close(); if (!dialog.open) dialog.showModal();
}
function formSubmit(handler, onSuccess = () => $('#dialog').close()) {
  $('#modal-form').onsubmit = async e => {
    e.preventDefault(); const button = $('button[type=submit]', e.target); if(button.disabled)return; button.disabled = true;
    try { await handler(new FormData(e.target)); onSuccess(); }
    catch (err) { $('#form-error').textContent = err.message; }
    finally { button.disabled = false; }
  };
}
const formEnd = (label='Save', extra='') => `<p class="form-error" id="form-error" role="alert"></p><div class="dialog-actions">${extra}<button type="submit" class="button primary">${label}</button></div></form>`;
function attachmentMarkup(file, url) {
  return `${['image/png','image/jpeg','image/webp','image/gif'].includes(file.media_type) ? `<img src="${escape(url)}" alt="${escape(file.name)}">` : '<span aria-hidden="true">▤</span>'}<span>${escape(file.name)}</span>`;
}
function attachFiles(input) {
  const zone = input.parentElement;
  const area = document.createElement('div'); area.className = 'attachment-editor';
  area.innerHTML = '<div class="attachment-list" aria-live="polite"></div><button type="button" class="button small attach-button">Attach files</button><span class="attachment-hint">Paste or drop images and files · Up to 25 MB each</span><input type="file" multiple hidden>';
  input.after(area);
  const picker = $('input', area), list = $('.attachment-list', area);
  let files = [];
  const render = () => {
    input.required = !files.length;
    list.innerHTML = files.map((f,i) => `<div class="attachment-chip">${attachmentMarkup({name:f.file.name,media_type:f.file.type}, f.url)}<button type="button" data-remove="${i}" class="icon-button" aria-label="Remove ${escape(f.file.name)}">×</button></div>`).join('');
  };
  const add = items => {
    for (const file of items) {
      if (file.size > 25*1024*1024) { toast(`${file.name} exceeds 25 MB`); continue; }
      files.push({file, url:URL.createObjectURL(file)});
    }
    render();
  };
  $('.attach-button', area).onclick = () => picker.click();
  picker.onchange = () => { add(picker.files); picker.value = ''; };
  list.onclick = e => { const button = e.target.closest('[data-remove]'); if(button){const [f]=files.splice(Number(button.dataset.remove),1);URL.revokeObjectURL(f.url);render();} };
  input.addEventListener('paste', e => { if(e.clipboardData?.files.length){e.preventDefault();add(e.clipboardData.files);} });
  zone.addEventListener('dragover', e => { if([...e.dataTransfer.types].includes('Files')){e.preventDefault();e.dataTransfer.dropEffect='copy';zone.classList.add('file-dragover');} });
  zone.addEventListener('dragleave', e => { if(!zone.contains(e.relatedTarget))zone.classList.remove('file-dragover'); });
  zone.addEventListener('drop', e => { zone.classList.remove('file-dragover');if(e.dataTransfer.files.length){e.preventDefault();e.stopPropagation();add(e.dataTransfer.files);} });
  const clear = () => {files.forEach(f=>URL.revokeObjectURL(f.url));files=[];render();};
  const dialog = input.closest('dialog'); if(dialog)dialog.addEventListener('close', clear, {once:true});
  return {
    clear,
    async upload() {
      const ids = [];
      for(const item of files){
        if(!item.id){
          const query = new URLSearchParams({name:item.file.name,media_type:item.file.type});
          const res = await fetch(`/api/attachments?${query}`,{method:'POST',body:item.file});
          if(!res.ok)throw Error(`Could not attach ${item.file.name}: ${await res.text()}`);
          item.id=(await res.json()).id;
        }
        ids.push(item.id);
      }
      return ids;
    }
  };
}
function addProject() {
  modal('Add a project', `<form id="modal-form"><p class="help">A project is an existing folder on your computer. Codex works directly in that folder.</p><label class="field"><span>Project name</span><input name="name" placeholder="My project" required autofocus></label><label class="field"><span>Folder path</span><input name="path" placeholder="C:\\Users\\you\\Projects\\my-project" required></label>${formEnd('Add project')}`);
  formSubmit(async f => { const result = await api({action:'add_project',name:f.get('name'),path:f.get('path')}); projectId=result.id;renderBoard();toast('Project added'); });
}
function newTask(status = 'todo') {
  if (!data.projects.length) { addProject(); toast('Add your first project folder, then create a task.'); return; }
  modal('New task', `<form id="modal-form" class="new-task-form"><label class="field"><span>Title</span><input name="title" placeholder="What should the agent work on?" required autofocus maxlength="200"></label><div class="field instructions-field"><label for="task-instructions">Instructions</label><textarea id="task-instructions" name="text" placeholder="Describe the outcome, relevant details, and how to check the result..." required></textarea></div><div class="new-task-footer"><label class="field"><span>Project</span><select name="project_id">${data.projects.map(p=>`<option value="${p.id}" ${p.id===projectId?'selected':''}>${escape(p.name)}</option>`).join('')}</select></label><label class="field"><span>Model</span><select name="model">${modelOptions()}</select></label><button type="submit" class="button primary">Create task</button></div><p class="form-error" id="form-error" role="alert"></p></form>`, false, true);
  const attachments = attachFiles($('#task-instructions'));
  formSubmit(async f => { await api({action:'create_task',...Object.fromEntries(f),status,attachments:await attachments.upload()}); });
  $('#modal-form').onkeydown=e=>{if(e.key==='Enter'&&(e.ctrlKey||e.metaKey)){e.preventDefault();e.currentTarget.requestSubmit();}};
}
function openTask(key) {
  $('#task-panel').clearAttachments?.();
  selectedTask=key; lastMessages=''; const t=task(key); if(!t) return closeTask();
  $('#overlay').hidden=false; $('#task-panel').hidden=false;
  $('main').inert=true;
  $('#task-panel').innerHTML=`<div class="panel-header"><h2 id="panel-title"></h2><details class="app-menu task-menu"><summary aria-label="Task menu" title="Task menu"><span class="burger"></span></summary><div class="menu-items"><label class="field"><span>Model</span><select id="task-model" aria-label="Task model">${modelOptions(t.model)}</select></label><button id="task-agents">AGENTS.md</button><button id="edit-task">Edit title</button><button id="delete-task">Delete task</button><button id="stop-task" hidden>Stop</button></div></details><button id="approve-task" class="button primary small" title="Approve and clean up this task and its Codex session in 30 minutes. Project files stay." hidden>Approve</button><button id="revive-task" class="button small" title="Cancel cleanup" hidden>Revive</button><button id="close-task" class="icon-button" aria-label="Close task">×</button></div><div class="messages" id="messages"></div><form class="composer" id="message-form"><textarea id="message-input" placeholder="Add details or ask Codex to continue…" aria-label="Message to Codex" required></textarea></form>`;
  $('#close-task').onclick=closeTask;
  $('#task-model').onchange=async e=>{await act({action:'edit_task',id:key,title:task(key).title,model:e.target.value});refreshPanel();};
  $('#task-agents').onclick=()=>openAgents(t.project_id);
  $('#edit-task').onclick=()=>{
    modal('Edit task',`<form id="modal-form"><label class="field"><span>Title</span><input name="title" value="${escape(task(key).title)}" required></label>${formEnd()}`);
    formSubmit(f=>api({action:'edit_task',id:key,title:f.get('title'),model:task(key).model}));
  };
  $('#delete-task').onclick=()=>{
    modal('Delete this task?',`<form id="modal-form"><p class="help">This removes “${escape(task(key).title)}” and its board conversation. Project files and Codex session files are kept.</p>${formEnd('Delete task')}`);
    formSubmit(async()=>{await api({action:'delete_task',id:key});closeTask();});
  };
  $('#approve-task').onclick=async()=>{
    try{await api({action:'approve_task',id:key});if(selectedTask===key)closeTask();}catch(err){toast(err.message);}
  };
  $('#revive-task').onclick=()=>act({action:'revive_task',id:key});
  $('#stop-task').onclick=()=>act({action:'stop_task',id:key});
  const attachments = attachFiles($('#message-input'));
  $('#message-form').insertAdjacentHTML('beforeend','<button type="submit" id="send-message" class="button primary small">Send message</button>');
  $('#task-panel').clearAttachments = attachments.clear;
  let sending = false;
  $('#message-form').onsubmit=async e=>{
    e.preventDefault();if(sending)return;const input=$('#message-input');const text=input.value;
    sending=true;input.readOnly=true;$('#send-message').disabled=true;
    try{await api({action:'send_message',id:key,text,attachments:await attachments.upload()});input.value='';attachments.clear();$('#messages').scrollTop=$('#messages').scrollHeight;}catch(err){toast(err.message);}finally{sending=false;input.readOnly=false;const button=$('#send-message');if(button)button.disabled=false;}
  };
  $('#message-input').onkeydown=e=>{if(e.key==='Enter'&&(e.ctrlKey||e.metaKey)){e.preventDefault();$('#message-form').requestSubmit();}};
  refreshPanel();
  $('#close-task').focus();
}
function refreshPanel() {
  const t=task(selectedTask);if(!t)return closeTask();
  $('#panel-title').textContent=t.title;$('#panel-title').title=t.title;
  $('#task-model').disabled=t.status==='now';$('#task-model').value=t.model;
  $('#edit-task').disabled=t.status==='now';$('#delete-task').disabled=t.status==='now';
  $('#stop-task').hidden=t.status!=='now';$('#stop-task').disabled=t.cancel;$('#stop-task').textContent=t.cancel?'Stopping…':'Stop';
  $('#approve-task').hidden=t.status!=='done'||!!t.cleanup_at||!!t.attention;
  $('#revive-task').hidden=t.status!=='done'||!t.cleanup_at;
  $('#send-message').textContent=needsAttention(t)?'Send answer & continue':'Send message';
  $('#message-input').placeholder=needsAttention(t)?'Type your answer or instructions to continue…':'Add details or ask Codex to continue…';
  $('#panel-title').classList.toggle('card-error',needsAttention(t));
  const signature=JSON.stringify([t.messages,t.error,t.attention,t.status]);if(signature===lastMessages)return;lastMessages=signature;
  const messages=$('#messages');const atBottom=messages.scrollHeight-messages.scrollTop-messages.clientHeight<90||!messages.innerHTML;
  const opened=[...messages.querySelectorAll('details[open]')].map(el=>el.dataset.index);
  messages.innerHTML=(needsAttention(t)&&!t.error?`<div class="run-error" role="status">${escape(attentionLabel(t))}. This task is unfinished. Reply below to continue.</div>`:'')+(t.error?`<div class="run-error">${escape(t.error)}${t.status!=='now'?'<div><button id="retry-task" class="button small">Retry in queue</button></div>':''}</div>`:'')+t.messages.map((m,i)=>{
    if(m.role==='system')return `<div class="message system">${escape(m.text)}</div>`;
    if(m.role==='activity')return `<div class="message activity"><details data-index="${i}" ${opened.includes(String(i))?'open':''}><summary>${escape(m.text.split('\n')[0].slice(0,140))}</summary><pre>${escape(m.text)}</pre></details></div>`;
    return `<article class="message ${m.role}"><div class="message-label">${m.role==='user'?'You':'◈ Codex'}<time>${new Date(m.at).toLocaleTimeString(undefined,{hour:'2-digit',minute:'2-digit'})}</time></div><div class="message-body">${escape(m.text)}</div><div class="attachment-list">${(m.attachments||[]).map(f=>`<a class="attachment-chip" href="/api/attachments/${encodeURIComponent(f.id)}" download="${escape(f.name)}">${attachmentMarkup(f, `/api/attachments/${encodeURIComponent(f.id)}`)}</a>`).join('')}</div></article>`;
  }).join('');
  if($('#retry-task'))$('#retry-task').onclick=()=>act({action:'move_task',id:t.id,status:'todo'});
  if(atBottom)messages.scrollTop=messages.scrollHeight;
}
function closeTask(){$('#task-panel').clearAttachments?.();selectedTask='';$('#task-panel').hidden=true;$('#overlay').hidden=true;$('main').inert=false;}
async function openAgents(key){
  try{
    const response=await fetch(`/api/agents/${key}`);if(!response.ok)throw Error(await response.text());const info=await response.json();
    modal('AGENTS.md',`<form id="modal-form"><p class="path">${escape(info.path)}</p><p class="help">Shared by every task in this project. Codex reads these instructions when it works. Save changes before starting the next run.</p>${info.overridden?'<p class="warning">AGENTS.override.md exists in this folder and takes priority over AGENTS.md. Its content is shown below.</p>':''}${!info.exists?'<p class="help">This project has no AGENTS.md yet. Write instructions below or start from a template, then create the file.</p>'+`<div class="template-toolbar"><select id="apply-template" aria-label="Instruction template"><option value="">Start from a template…</option>${data.templates.map(t=>`<option value="${t.id}">${escape(t.name)}</option>`).join('')}</select><button id="load-template" type="button" class="button small">Load into editor</button></div>`:''}<label class="field"><span>AGENTS.md</span><textarea class="code-editor" name="content" id="agents-content" spellcheck="false">${escape(info.content)}</textarea></label>${info.inherited.length?`<details class="inherited"><summary>Other instruction files (${info.inherited.length})</summary>${info.inherited.map(f=>`<p class="path">${escape(f.path)}</p><pre>${escape(f.content)}</pre>`).join('')}</details>`:''}<p class="help">Nested folders may contain additional instructions. Changes are written to this project's AGENTS.md only when you save.</p>${formEnd(info.exists?'Save AGENTS.md':'Create AGENTS.md')}`,true);
    if($('#load-template'))$('#load-template').onclick=()=>{const t=data.templates.find(t=>t.id===$('#apply-template').value);if(t)$('#agents-content').value=t.content;};
    formSubmit(async f=>{await api({action:'save_agents',project_id:key,content:f.get('content'),original:info.content});toast('AGENTS.md saved');});
  }catch(e){toast(e.message);}
}
function templates(){
  modal('AGENTS.md templates',`<p class="help">Shared templates for new project instruction files. Editing or deleting a template does not change existing AGENTS.md files.</p><div class="template-toolbar"><button id="new-template" class="button primary small">New template</button><button id="templates-back" class="button small">Back to Settings</button></div>${data.templates.length?`<label class="field"><span>Template</span><select id="template-select">${data.templates.map(t=>`<option value="${t.id}">${escape(t.name)}</option>`).join('')}</select></label><div class="template-toolbar"><button id="edit-template" class="button small">Edit</button><button id="duplicate-template" class="button small">Duplicate</button><button id="delete-template" class="button danger small">Delete</button></div><pre id="template-preview" class="template-preview"></pre>`:'<p class="help">No templates yet. Create one to reuse instructions across projects.</p>'}`,true);
  $('#new-template').onclick=()=>editTemplate();
  $('#templates-back').onclick=settings;
  if(!data.templates.length)return;
  const selected=()=>data.templates.find(t=>t.id===$('#template-select').value);
  const preview=()=>$('#template-preview').textContent=selected()?.content??'';
  $('#template-select').onchange=preview;preview();
  $('#edit-template').onclick=()=>{const t=selected();if(t)editTemplate(t);};
  $('#duplicate-template').onclick=()=>{const t=selected();if(t)editTemplate({name:`${t.name} (copy)`,content:t.content});};
  $('#delete-template').onclick=()=>{
    const t=selected();if(!t)return;
    modal('Delete template?',`<form id="modal-form"><p class="help">Delete "${escape(t.name)}"? Existing project AGENTS.md files will be kept.</p>${formEnd('Delete template','<button id="cancel-delete-template" type="button" class="button">Cancel</button>')}`);
    $('#cancel-delete-template').onclick=templates;
    formSubmit(async()=>{await api({action:'delete_template',id:t.id});toast('Template deleted');},templates);
  };
}
function editTemplate(t = {}){
  modal(t.id?'Edit template':'New template',`<form id="modal-form"><label class="field"><span>Template name</span><input name="name" value="${escape(t.name)}" required autofocus></label><label class="field"><span>Instructions</span><textarea name="content" class="code-editor" spellcheck="false">${escape(t.content)}</textarea></label>${formEnd('Save template','<button id="cancel-template" type="button" class="button">Cancel</button>')}`,true);
  $('#cancel-template').onclick=templates;
  formSubmit(async f=>{await api({action:'save_template',id:t.id??null,name:f.get('name'),content:f.get('content')});toast('Template saved');},templates);
}
function settings(){
  modal('Settings',`<label class="field"><span>Project instructions</span><select id="agents-project" aria-label="Project for AGENTS.md">${data.projects.map(p=>`<option value="${p.id}" ${p.id===projectId?'selected':''}>${escape(p.name)}</option>`).join('')}</select></label><button id="settings-agents" class="button small" ${data.projects.length?'':'disabled'}>Edit or create AGENTS.md</button><p class="help">Edit the selected project's file. If it does not exist, create it from a template or write your own instructions.${data.projects.length?'':' Add a project first to create its AGENTS.md.'}</p><div class="settings-actions"><button id="pause-button" class="button small">${data.settings.paused?'Resume queue':'Pause queue'}</button></div><form id="modal-form"><label class="field"><span>Default model</span><input name="default_model" list="models-list" value="${escape(data.settings.default_model)}" placeholder="Use Codex configuration"><datalist id="models-list">${models.map(m=>`<option value="${escape(m)}"></option>`).join('')}</datalist></label><p class="help">Used by tasks set to Default. Model suggestions come from your local Codex model cache. You can also enter a model ID.</p><label class="field"><span>Codex executable</span><input name="codex_path" value="${escape(data.settings.codex_path)}" required></label><p class="help">Uses your existing Codex login. On Windows, choose the native codex.exe, not a .cmd or .ps1 launcher.</p><p class="help">Runs use workspace-write permissions with no interactive approval prompts. Codex can edit the project folder. Operations requiring extra permissions report a blocker in the conversation.</p><p class="help">Execution is sequential across all projects. Pause queue stops new runs; Stop ends the current task.</p>${formEnd()}`);
  $('#settings-agents').onclick=()=>openAgents($('#agents-project').value);
  $('#settings-agents').insertAdjacentHTML('afterend',' <button id="settings-templates" class="button small">Manage templates</button>');
  $('#settings-templates').onclick=templates;
  $('#pause-button').onclick=async e=>{await act({action:'settings',...data.settings,paused:!data.settings.paused});e.target.textContent=data.settings.paused?'Resume queue':'Pause queue';};
  formSubmit(f=>api({action:'settings',...Object.fromEntries(f),paused:data.settings.paused}));
}
$('#project-tabs').onclick=e=>{const tab=e.target.closest('[data-project]');if(tab){projectId=tab.dataset.project;renderBoard();}};
$('#add-project-tab').onclick=()=>{if(data)addProject();};
$('#add-project-button').onclick=()=>{if(data)addProject();};
$('#new-task-button').onclick=()=>{if(data)newTask();};
document.addEventListener('click',e=>{document.querySelectorAll('.app-menu[open]').forEach(menu=>{if(!menu.contains(e.target)||e.target.closest('button'))menu.open=false;});});
$('#settings-button').onclick=()=>{if(data)settings();};
$('#board').onclick=e=>{const card=e.target.closest('[data-task]');const add=e.target.closest('[data-new]');if(card)openTask(card.dataset.task);else if(add)newTask(add.dataset.new);};
$('#board').ondragstart=e=>{const card=e.target.closest('[data-task]');if(card)e.dataTransfer.setData('text/plain',card.dataset.task);};
$('#board').ondragover=e=>{const column=e.target.closest('.column');if(column&&column.dataset.status!=='now'){e.preventDefault();column.classList.add('dragover');}};
$('#board').ondragleave=e=>{const column=e.target.closest('.column');if(column&&!column.contains(e.relatedTarget))column.classList.remove('dragover');};
$('#board').ondragend=()=>document.querySelectorAll('.dragover').forEach(c=>c.classList.remove('dragover'));
$('#board').ondrop=e=>{e.preventDefault();document.querySelectorAll('.dragover').forEach(c=>c.classList.remove('dragover'));const column=e.target.closest('.column');const id=e.dataTransfer.getData('text/plain');if(column&&task(id)&&column.dataset.status!=='now')act({action:'move_task',id,status:column.dataset.status});};
$('#overlay').onclick=closeTask;
document.onkeydown=e=>{
  if(e.key==='Escape')document.querySelectorAll('.app-menu[open]').forEach(menu=>menu.open=false);
  if(e.key==='Escape'&&!$('#dialog').open)closeTask();
  if(e.key==='Tab'&&selectedTask&&!$('#dialog').open){
    const controls=[...$('#task-panel').querySelectorAll('button,select,textarea,summary')].filter(el=>!el.disabled&&el.getClientRects().length);
    const first=controls[0],last=controls.at(-1);
    if(e.shiftKey&&document.activeElement===first){e.preventDefault();last.focus();}
    else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first.focus();}
  }
};
setInterval(()=>{if(!data)return;document.querySelectorAll('[data-cleanup]').forEach(el=>{const t=task(el.dataset.cleanup);if(t?.cleanup_at)el.textContent=cleanupLabel(t);});document.querySelectorAll('[data-timer]').forEach(el=>{const t=task(el.dataset.timer);if(t)el.textContent=`${t.status==='now'?'◷ ':''}${duration(elapsed(t))}`;});},1000);
let disconnected = false;
pollLimits();
async function poll(){try{await refresh();}catch(e){if(!disconnected)toast('Server disconnected');disconnected=true;}setTimeout(poll,1200);}poll();
