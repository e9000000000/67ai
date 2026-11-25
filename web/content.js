const briefFields = [
  ['direction', 'Creative direction', 'What should this content feel like?'],
  ['sample_scope', 'One playable sample', 'Define exactly one small unit; use a level only if this game has levels.'],
  ['systems', 'Confirmed systems and evidence', 'Which existing systems and files support this direction?'],
  ['deliverables', 'Concrete deliverables', 'Which assets or content should the sample contain?'],
  ['integration', 'Integration paths', 'Existing content files, scene, registry, or entry point.'],
  ['checks', 'Technical checks', 'Relevant checks and project constraints, including any no-build rules.'],
  ['try_steps', 'How to find and try it', 'Where should the user find the sample and what should they try?'],
];

function contentPlans() {
  if (!data.projects.length) return addProject();
  const plans = (data.content_plans || []).filter(p => !projectId || p.project_id === projectId);
  modal('Content plans', `<p class="help">Save a brief, make one playable sample, and try it before expanding. Your direction and feedback stay here after task cleanup.</p><button id="explore-content" class="button primary">Explore content</button><button id="new-content-brief" class="button primary">New content brief</button><div class="content-plan-list">${plans.map(p=>`<button class="button" data-plan="${p.id}">${escape(p.sample_brief?.direction||p.brief?.title||'Choose a content direction')} · ${escape(project(p.project_id)?.name)} · ${escape(p.decision)}</button>`).join('') || '<p class="help">No saved briefs yet.</p>'}</div>`, true);
  $('#explore-content').onclick = startContent;
  $('#new-content-brief').onclick = ()=>newContentBrief();
  document.querySelectorAll('[data-plan]').forEach(b=>b.onclick=()=>openContent(b.dataset.plan));
}

function newContentBrief(planId = null) {
  const plan=data.content_plans.find(p=>p.id===planId), direction=plan?.brief;
  const values=direction?{direction:direction.title+' ? '+direction.experience,systems:direction.reuse+'\n'+JSON.stringify(plan.research.inventory),checks:direction.quality_checks+'\n'+direction.constraints}:{};
  const selectedProject=plan?.project_id||projectId;
  modal('Content brief', `<form id="modal-form"><p class="help">Use evidence from your project. These details bound the sample; the implementation agent must verify them before making changes.</p><label class="field"><span>Project</span><select name="project_id" required><option value="" disabled ${!selectedProject?'selected':''}>Select a project</option>${data.projects.map(p=>`<option value="${p.id}" ${p.id===selectedProject?'selected':''}>${escape(p.name)}</option>`).join('')}</select></label>${briefFields.map(([key,label,hint])=>`<label class="field"><span>${label}</span><textarea name="${key}" placeholder="${hint}" required>${escape(values[key]||'')}</textarea></label>`).join('')}${formEnd('Save brief')}`, true);
  let key;
  formSubmit(async f=>{
    const brief=Object.fromEntries(briefFields.map(([key])=>[key,f.get(key)]));
    key=(await api({action:'save_content_brief',project_id:f.get('project_id'),brief,id:planId})).id;
  }, ()=>openContentPlan(key));
}

function openContentPlan(key) {
  const p=data.content_plans.find(p=>p.id===key); if(!p)return;
  modal(p.sample_brief.direction, `<div id="content-plan" data-id="${p.id}"><button id="content-back" class="button small">All content plans</button>${p.research?'<button id="guided-research" class="button small">Guided research &amp; direction</button>':''}<details><summary>Saved brief</summary><dl class="content-brief">${briefFields.map(([key,label])=>`<dt>${label}</dt><dd>${escape(p.sample_brief[key])}</dd>`).join('')}</dl></details><div id="sample-status" aria-live="polite"></div><div class="content-actions"><button id="make-sample" class="button primary">Make a sample</button><button id="open-sample" class="button">Open sample task</button></div><h3>Creative feedback</h3><p class="help">Try the sample yourself. Approve and AI approve only review task completion and schedule cleanup; they never select this direction.</p><label class="field"><span>Short feedback</span><textarea id="sample-feedback" placeholder="What worked? What should change?" maxlength="2000"></textarea></label><div class="content-actions"><button data-decision="revise" class="button">Revise sample</button><button data-decision="accepted" class="button primary">Use this direction</button><button data-decision="set_aside" class="button">Set aside</button></div><div id="sample-history"></div><div class="content-actions"><button id="expand-content" class="button primary">Research next bounded batch</button><button id="open-content-research" class="button">Open batch research</button></div><p class="help">After explicit acceptance, research can propose up to three Future tasks, each limited to one unit comparable to the sample. You choose when to start them.</p></div>`, true);
  $('#content-back').onclick=contentPlans;
  if($('#guided-research'))$('#guided-research').onclick=()=>openContent(key,true);
  const run=async (button, value)=>{
    $('#content-plan').dataset.busy='true'; refreshContentPlan();
    try { await api(value); if(value.action==='sample_decision' && $('#sample-feedback').value===value.feedback)$('#sample-feedback').value=''; }
    catch(e){toast(e.message);}
    finally { if($('#content-plan'))delete $('#content-plan').dataset.busy; refreshContentPlan(); }
  };
  $('#make-sample').onclick=e=>run(e.target,{action:'make_sample',id:key});
  $('#expand-content').onclick=e=>run(e.target,{action:'expand_content',id:key});
  document.querySelectorAll('[data-decision]').forEach(b=>b.onclick=()=>run(b,{action:'sample_decision',id:key,decision:b.dataset.decision,feedback:$('#sample-feedback').value}));
  const openLinked=field=>{const p=data.content_plans.find(p=>p.id===key);$('#dialog').close();openTask(p[field]);};
  $('#open-sample').onclick=()=>openLinked('sample_id');
  $('#open-content-research').onclick=()=>openLinked('research_id');
  refreshContentPlan();
}

function refreshContentPlan() {
  const el=$('#content-plan'); if(!el || !$('#dialog').open)return;
  const p=data.content_plans.find(p=>p.id===el.dataset.id); if(!p)return;
  const sample=task(p.sample_id), research=task(p.research_id);
  $('#sample-status').innerHTML=`<p><strong>${escape(p.sample_status)}</strong>${p.sample_id&&!sample?' · Task cleaned up; report saved':''}</p><p>Creative decision: <strong>${escape({pending:'Awaiting your judgment',accepted:'Use this direction',set_aside:'Set aside'}[p.decision])}</strong></p>${p.sample_report?`<h3>Sample report</h3><pre class="sample-report">${escape(p.sample_report)}</pre>`:''}`;
  $('#make-sample').hidden=!!p.sample_id;
  $('#make-sample').disabled=!!p.sample_id;
  $('#open-sample').hidden=!sample;
  $('[data-decision="accepted"]').disabled=!p.sample_ready;
  $('[data-decision="revise"]').disabled=!p.sample_id || !!(sample && (['now','todo'].includes(sample.status)||sample.awaiting_merge));
  $('[data-decision="set_aside"]').disabled=!p.sample_id;
  $('#expand-content').disabled=p.decision!=='accepted'||!p.sample_ready||!!p.research_id;
  $('#open-content-research').hidden=!research;
  $('#sample-history').innerHTML=p.feedback.map(f=>`<p><strong>${escape({revise:'Revise sample',accepted:'Use this direction',set_aside:'Set aside'}[f.decision])}</strong>: ${escape(f.text)}</p>`).join('');
  if(el.dataset.busy)el.querySelectorAll('.content-actions button').forEach(b=>b.disabled=true);
  else { $('#open-sample').disabled=false; $('#open-content-research').disabled=false; }
}
