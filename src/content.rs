use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct ContentBrief {
    pub direction: String,
    pub sample_scope: String,
    pub systems: String,
    pub deliverables: String,
    pub integration: String,
    pub checks: String,
    pub try_steps: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ContentFeedback { decision: String, text: String, at: u64 }

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentPlan {
    pub id: String,
    pub(super) project_id: String,
    pub(super) source_id: String,
    pub(super) research: Option<ContentResearch>,
    pub(super) messages: Vec<Message>,
    pub(super) brief: Option<ContentDirection>,
    pub(super) sample_brief: Option<ContentBrief>,
    pub(super) sample_id: Option<String>,
    pub(super) research_id: Option<String>,
    pub(super) revision: u64,
    pub(super) decision: String,
    pub(super) feedback: Vec<ContentFeedback>,
    pub(super) sample_status: String,
    pub(super) sample_report: String,
    pub(super) sample_ready: bool,
    #[serde(default)] pub batch_task_ids: Vec<String>,
}

fn sample_ready(t: &Task) -> bool {
    t.implementation_completed && t.status == "done" && t.attention.is_none() && t.error.is_none()
        && !t.awaiting_merge && !t.merging && !t.review_pending && !t.cancel
}

pub fn sync_content_plans(d: &mut Data, previous: &Data) {
    for p in &mut d.content_plans {
        // Capture the final report even when this commit deletes the task/session.
        let sample = d.tasks.iter().find(|t| Some(&t.id) == p.sample_id.as_ref())
            .or_else(|| previous.tasks.iter().find(|t| Some(&t.id) == p.sample_id.as_ref()));
        if let Some(t) = sample {
            p.sample_ready = sample_ready(t);
            p.sample_status = if t.awaiting_merge || t.merging {
                if t.error.is_some() { "Merge blocked" } else { "Waiting for merge" }
            } else if t.error.is_some() { "Failed / needs attention" }
            else if t.review_pending { "AI review pending" }
            else if p.sample_ready { "Ready to try" }
            else if t.status == "now" { "Running" }
            else if t.status == "todo" { "Queued" }
            else if t.status == "future" { "Not started / stopped" }
            else if let Some(attention) = &t.attention { attention }
            else { "Completion not confirmed" }.into();
            p.sample_report = t.messages.iter().rev().find(|m| m.role == "assistant").map(|m| m.text.clone()).unwrap_or_default();
            if let Some(error) = &t.error { p.sample_status.push_str(&format!(": {error}")); }
            // Technical reviews never accept a direction. Further implementation requires another decision.
            if p.decision == "accepted" && !t.implementation_completed { p.decision = "pending".into(); }
        }
    }
}

pub fn content_context(p: &ContentPlan) -> String {
    format!("Saved content plan {} (revision {}). Creative decision: {}.\nChosen brief:\n{}\nGuided direction and research evidence:\n{}\nUser feedback history:\n{}\nSample task: {}\nSample status: {}\nSample completion report and try instructions:\n{}",
        p.id, p.revision, p.decision, serde_json::to_string_pretty(&p.sample_brief).unwrap(), json!({"direction":p.brief,"research":p.research}),
        serde_json::to_string_pretty(&p.feedback).unwrap(), p.sample_id.as_deref().unwrap_or("none"), p.sample_status, p.sample_report)
}

pub fn content_task_allowed(d: &Data, t: &Task) -> bool {
    let Some(key) = &t.content_plan_id else { return true; };
    let Some(p) = d.content_plans.iter().find(|p| &p.id == key) else { return false; };
    if p.sample_id.as_ref() == Some(&t.id) { return true; }
    p.decision == "accepted" && current_sample_ready(d, p) && p.revision == t.content_revision
}

fn current_sample_ready(d: &Data, p: &ContentPlan) -> bool {
    d.tasks.iter().find(|t| Some(&t.id) == p.sample_id.as_ref()).map(sample_ready).unwrap_or(p.sample_ready)
}

fn plan_index(d: &Data, key: &str) -> Result<usize, (StatusCode, String)> {
    d.content_plans.iter().position(|p| p.id == key).ok_or_else(|| bad("Content plan not found"))
}

pub fn save_brief(d: &mut Data, project_id: String, brief: ContentBrief, key: Option<&str>) -> Result<Value, (StatusCode, String)> {
    if !d.projects.iter().any(|p| p.id == project_id) { return Err(bad("Project not found")); }
    for (name, value) in [("Direction", &brief.direction), ("One sample scope", &brief.sample_scope), ("Confirmed systems and evidence", &brief.systems),
        ("Deliverables", &brief.deliverables), ("Integration paths", &brief.integration), ("Technical checks", &brief.checks), ("How to try", &brief.try_steps)] {
        nonempty(value, name)?;
    }
    let index = if let Some(key) = key {
        let i = plan_index(d, key)?;
        if d.content_plans[i].project_id != project_id { return Err(bad("Wrong project")); }
        if d.content_plans[i].sample_brief.is_some() { return Err(bad("Sample brief already saved")); }
        i
    } else {
        d.content_plans.push(ContentPlan { id: id(), project_id, ..ContentPlan::default() });
        d.content_plans.len() - 1
    };
    let p = &mut d.content_plans[index];
    p.sample_brief = Some(brief); p.revision = 1; p.decision = "pending".into(); p.sample_status = "No sample yet".into();
    let key = p.id.clone();
    Ok(json!({"id": key}))
}

fn content_task(d: &Data, p: &ContentPlan, title: String, text: String, research: bool) -> Task {
    let mut t = Task { blocker_id: None, content_planning: false, id: id(), project_id: p.project_id.clone(), title,
        model: if research { d.settings.gen_model.clone() } else { d.settings.hard_model.clone() },
        difficulty: Some(if research { Difficulty::Gen } else { Difficulty::Hard }), status: "todo".into(), messages: vec![],
        created_at: now(), queued_at: now(), elapsed_ms: 0, started_at: None, thread_id: None, session_ids: vec![], error: None, sent_count: 0,
        cancel: false, cleanup_at: None, attention: None, worktree: None, awaiting_merge: false, merging: false,
        review_pending: false, review_attempted: false, content_plan_id: Some(p.id.clone()), content_revision: p.revision, implementation_completed: false };
    t.message("user", text);
    t
}

const SAMPLE_INSTRUCTIONS: &str = "Implement exactly ONE playable sample of the saved brief's sample_scope using confirmed existing game systems. Do not expand the content batch or add new mechanics. A level is appropriate only if levels are confirmed in project files. First inspect the cited systems, formats and integration paths; if the brief is unsupported or not bounded to one sample, ask for clarification instead of inventing systems. Deliver the specified sample assets/content and integrate them at the listed paths. Run the listed technical checks subject to all project-specific validation constraints (including Unity Editor/background/no-build rules). Report concrete changed paths, integration entry points, checks passed/failed or unavailable, and exact instructions for finding, launching and trying the sample. If required work or verification is blocked, report blocked, not completed. Technical success does not mean the user likes or accepts this direction.";

pub fn make_sample(d: &mut Data, key: &str) -> Result<Value, (StatusCode, String)> {
    let i = plan_index(d, key)?;
    if let Some(key) = &d.content_plans[i].sample_id { return Ok(json!({"id": key})); }
    let p = &d.content_plans[i];
    if p.sample_brief.is_none() { return Err(bad("Save a bounded sample brief first")); }
    let t = content_task(d, p, format!("Sample: {}", p.sample_brief.as_ref().unwrap().direction), format!("{SAMPLE_INSTRUCTIONS}\n\n{}", content_context(p)), false);
    let result = json!({"id": t.id});
    d.content_plans[i].sample_id = Some(t.id.clone());
    d.tasks.push(t);
    Ok(result)
}

pub fn decide_sample(d: &mut Data, key: &str, decision: &str, feedback: &str) -> Result<(), (StatusCode, String)> {
    nonempty(feedback, "Short feedback")?;
    if !matches!(decision, "revise" | "accepted" | "set_aside") { return Err(bad("Unknown sample decision")); }
    let i = plan_index(d, key)?;
    let p = &d.content_plans[i];
    if p.sample_id.is_none() { return Err(bad("Make a sample first")); }
    if decision == "accepted" && !current_sample_ready(d, p) { return Err(bad("Only a completed, merged sample is ready to try and accept")); }
    if decision == "revise" {
        if let Some(t) = d.tasks.iter().find(|t| Some(&t.id) == p.sample_id.as_ref()) {
            idle(t)?;
            if t.awaiting_merge { return Err(bad("Resolve the sample merge before revising")); }
            if t.status == "todo" { return Err(bad("The sample is already queued")); }
        }
    }
    let p = &mut d.content_plans[i];
    p.feedback.push(ContentFeedback { decision: decision.into(), text: feedback.trim().into(), at: now() });
    p.decision = if decision == "revise" { "pending" } else { decision }.into();
    if decision == "revise" {
        p.revision += 1;
        p.research_id = None;
        p.batch_task_ids.clear();
        p.sample_ready = false;
        let p = p.clone();
        let instructions = format!("Revise the same single sample, retaining its brief. Apply the saved user feedback.\n{SAMPLE_INSTRUCTIONS}\n\n{}", content_context(&p));
        if let Some(t) = d.tasks.iter_mut().find(|t| Some(&t.id) == p.sample_id.as_ref()) {
            t.message("user", instructions);
            t.status = "todo".into(); t.queued_at = now(); t.cleanup_at = None; t.error = None; t.attention = None;
            t.review_pending = false; t.review_attempted = false; t.implementation_completed = false; t.content_revision = p.revision;
        } else {
            let t = content_task(d, &p, format!("Sample: {}", p.sample_brief.as_ref().unwrap().direction), instructions, false);
            d.content_plans[i].sample_id = Some(t.id.clone());
            d.tasks.push(t);
        }
    }
    Ok(())
}

pub const BATCH_INSTRUCTIONS: &str = "Research the next bounded batch from the accepted implemented sample. Inspect current project files, the actual sample integration and assets, saved feedback, and existing tasks. Propose at most THREE Future tasks, each implementing at most ONE content unit comparable to the accepted sample. Include concrete deliverables, integration paths, technical checks, and how to try each result. Reuse confirmed existing systems; no new features, speculative levels, broad production chains, or further batches. If evidence is missing, ask or report blocked. Research only; do not implement.";

pub fn expand_content(d: &mut Data, key: &str) -> Result<Value, (StatusCode, String)> {
    let i = plan_index(d, key)?;
    let p = &d.content_plans[i];
    if p.decision != "accepted" || !current_sample_ready(d, p) { return Err(bad("Try and explicitly accept a ready sample before expanding")); }
    if let Some(key) = &p.research_id { return Ok(json!({"id": key})); }
    let text = format!("{BATCH_INSTRUCTIONS}\n\n{}", content_context(p));
    let t = content_task(d, p, format!("Next content batch: {}", p.sample_brief.as_ref().unwrap().direction), text, true);
    let result = json!({"id": t.id});
    d.content_plans[i].research_id = Some(t.id.clone());
    d.tasks.push(t);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_guided_plans_keep_their_brief_and_evidence() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/content/existing.json")).unwrap();
        let old = json!({"id":"plan", "project_id":"p", "source_id":"research", "research":fixture["content"],
            "brief":fixture["content"]["directions"][0], "messages":[]});
        let p: ContentPlan = serde_json::from_value(old.clone()).unwrap();
        assert!(p.sample_brief.is_none());
        assert!(!p.sample_ready);
        let saved = serde_json::to_value(p).unwrap();
        assert_eq!(saved["brief"], old["brief"]);
        assert_eq!(saved["research"], old["research"]);
    }

    #[test]
    fn ready_requires_completed_merged_work_and_queue_checks_live_state() {
        let mut d = initial_data();
        d.projects.push(Project { id: "p".into(), name: "Game".into(), path: "".into(), auto_approve_easy: false, auto_approve_hard: false });
        let brief = ContentBrief { direction: "Quiet".into(), sample_scope: "One encounter".into(), systems: "encounters.txt".into(),
            deliverables: "sample.txt".into(), integration: "encounters.txt".into(), checks: "Loader".into(), try_steps: "Encounter menu".into() };
        let key = save_brief(&mut d, "p".into(), brief, None).unwrap()["id"].as_str().unwrap().to_string();
        make_sample(&mut d, &key).unwrap();
        let mut complete = d.tasks[0].clone();
        complete.status = "done".into(); complete.implementation_completed = true;
        d.tasks[0] = complete.clone();
        sync_content_plans(&mut d, &initial_data());
        decide_sample(&mut d, &key, "accepted", "Enjoyed this").unwrap();
        expand_content(&mut d, &key).unwrap();
        assert_eq!(next_task(&d), Some(1));
        for mode in ["failed", "blocked", "unmerged", "merging", "queued", "running", "stopped", "review", "unconfirmed"] {
            d.tasks[0] = complete.clone();
            let t = &mut d.tasks[0];
            match mode {
                "failed" => t.error = Some("failure".into()),
                "blocked" => t.attention = Some("blocked".into()),
                "unmerged" => t.awaiting_merge = true,
                "merging" => t.merging = true,
                "queued" => t.status = "todo".into(),
                "running" => t.status = "now".into(),
                "stopped" => t.status = "future".into(),
                "review" => t.review_pending = true,
                _ => t.implementation_completed = false,
            }
            // A previously saved ready snapshot cannot override current task state.
            assert!(!content_task_allowed(&d, &d.tasks[1]), "{mode}");
            assert!(decide_sample(&mut d, &key, "accepted", "Try accepting").is_err(), "{mode}");
            assert!(expand_content(&mut d, &key).is_err(), "{mode}");
        }
        d.tasks[0] = complete;
        decide_sample(&mut d, &key, "revise", "Shorter").unwrap();
        assert!(!content_task_allowed(&d, &d.tasks[1]));
        let saved = serde_json::to_vec(&d).unwrap();
        let restored: Data = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.content_plans[0].feedback.len(), 2);
        assert_eq!(restored.content_plans[0].revision, 2);
    }
}
