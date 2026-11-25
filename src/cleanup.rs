use super::*;

fn path_key(path: &str) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let text = path.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
    let text = if cfg!(windows) { text.to_lowercase() } else { text };
    let mut result = PathBuf::new();
    for component in std::path::Path::new(&text).components() {
        match component {
            std::path::Component::ParentDir => { result.pop(); }
            std::path::Component::CurDir => (),
            _ => result.push(component),
        }
    }
    result
}

fn assigned_session(task: &Task, id: &str) -> bool {
    task.thread_id.as_deref() == Some(id) || task.session_ids.iter().any(|s| s == id)
}

async fn remove_session(executable: &str, id: &str) -> Result<(), String> {
    match delete_session(executable, id).await {
        // A previous cleanup may have deleted the session before a later step failed.
        Err(e) if e.to_lowercase().contains("not found") && (e.to_lowercase().contains("thread") || e.to_lowercase().contains("session")) => Ok(()),
        result => result,
    }
}

struct Worktree { path: String, branch: Option<String> }

async fn worktrees(project: &Project) -> Result<Vec<Worktree>, String> {
    let output = git(&project.path, &["worktree", "list", "--porcelain", "-z"]).await?;
    Ok(output.split("\0\0").filter_map(|record| {
        let mut fields = record.split('\0');
        let path = fields.next()?.strip_prefix("worktree ")?.to_string();
        let branch = fields.find_map(|f| f.strip_prefix("branch refs/heads/").map(String::from));
        Some(Worktree { path, branch })
    }).collect())
}

async fn remove_worktree(project: &Project, tree: &Worktree) -> Result<(), String> {
    if path_key(&tree.path) == path_key(&project.path) { return Err("Cannot remove the project folder".into()); }
    git(&project.path, &["worktree", "remove", "--force", &tree.path]).await?;
    if let Some(branch) = &tree.branch {
        if branch.starts_with("67ai/") || branch.starts_with("taskdesk/") {
            git(&project.path, &["branch", "-D", branch]).await?;
        }
    }
    Ok(())
}

pub(super) async fn task_resources(store: &Store, task: &Task) -> Result<(), String> {
    #[cfg(windows)]
    shells::remove_task(&task.id);
    let mut sessions = task.session_ids.clone();
    if let Some(thread) = &task.thread_id { sessions.push(thread.clone()); }
    sessions.sort(); sessions.dedup();
    for session in sessions {
        if store.data.tasks.iter().any(|t| t.id != task.id && assigned_session(t, &session)) { continue; }
        remove_session(&store.data.settings.codex_path, &session).await?;
    }
    if let Some(path) = &task.worktree {
        if store.data.tasks.iter().any(|t| t.id != task.id && t.worktree.as_ref().is_some_and(|p| path_key(p) == path_key(path))) {
            return Err("Worktree is assigned to another task".into());
        }
        let project = store.data.projects.iter().find(|p| p.id == task.project_id).ok_or("Project not found")?;
        let trees = worktrees(project).await?;
        if let Some(tree) = trees.iter().find(|t| path_key(&t.path) == path_key(path)) {
            remove_worktree(project, tree).await?;
        } else if std::path::Path::new(path).exists() {
            return Err("Task folder is not a registered project worktree; cleanup stopped".into());
        }
        for branch in [format!("67ai/{}", task.id), format!("taskdesk/{}", task.id)] {
            if !git(&project.path, &["branch", "--list", &branch]).await?.is_empty() {
                git(&project.path, &["branch", "-D", &branch]).await?;
            }
        }
    }
    Ok(())
}

pub(super) async fn clear_project(store: &Store, id: &str) -> Result<Value, String> {
    let project = store.data.projects.iter().find(|p| p.id == id).ok_or("Project not found")?;
    if store.data.tasks.iter().any(|t| t.project_id == id && (t.status == "now" || t.merging)) {
        return Err("Stop running tasks and wait for merges before clearing this project".into());
    }
    let trees = if std::path::Path::new(&project.path).join(".git").exists() { worktrees(project).await? } else { vec![] };
    let project_root = path_key(&project.path);
    let belongs = |cwd: &str| {
        let cwd = path_key(cwd);
        (cwd.starts_with(&project_root) || trees.iter().any(|t| cwd.starts_with(path_key(&t.path))))
            && !store.data.projects.iter().any(|p| p.id != id && cwd.starts_with(path_key(&p.path)))
    };
    let mut sessions = vec![];
    for archived in [false, true] {
        let mut cursor = Value::Null;
        loop {
            let page = codex_request(&store.data.settings.codex_path, "thread/list", json!({
                "archived": archived, "cursor": cursor, "limit": 100, "modelProviders": [],
                "sourceKinds": ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther", "unknown"]
            })).await?;
            for thread in page["data"].as_array().ok_or("Invalid Codex session list")? {
                let id = thread["id"].as_str().ok_or("Missing Codex session ID")?;
                if thread["cwd"].as_str().is_some_and(&belongs) && !store.data.tasks.iter().any(|t| assigned_session(t, id)) {
                    sessions.push(id.to_string());
                }
            }
            cursor = page["nextCursor"].clone();
            if cursor.is_null() { break; }
        }
    }
    sessions.sort(); sessions.dedup();
    // Collect all pages before deleting: deletion can change pagination cursors.
    for session in &sessions { remove_session(&store.data.settings.codex_path, session).await?; }
    let mut removed = 0;
    // Git lists the repository's primary worktree first; it is never disposable.
    for tree in trees.iter().skip(1) {
        if path_key(&tree.path) == project_root || store.data.projects.iter().any(|p| path_key(&p.path) == path_key(&tree.path)) { continue; }
        if store.data.tasks.iter().any(|t| t.worktree.as_ref().is_some_and(|p| path_key(p) == path_key(&tree.path))) { continue; }
        remove_worktree(project, tree).await?;
        removed += 1;
    }
    Ok(json!({"ok": true, "sessions": sessions.len(), "worktrees": removed}))
}
