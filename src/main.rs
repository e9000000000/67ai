use axum::{Json, Router, extract::{Path, State}, http::{HeaderMap, StatusCode}, response::{Html, IntoResponse}, routing::{get, post}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::{SystemTime, UNIX_EPOCH}, process::Stdio};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::Command, sync::{Mutex, watch}};

type App = Arc<Mutex<Store>>;
// Worktree setup, recovery, and merging must not edit main concurrently.
static MAIN_GIT: Mutex<()> = Mutex::const_new(());
const DIRTY_MAIN: &str = "Commit or stash changes in the main worktree before starting or merging parallel tasks.";
type ApiResult = Result<Json<Value>, (StatusCode, String)>;
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64 }
fn id() -> String { uuid::Uuid::new_v4().to_string() }
fn bad(s: impl ToString) -> (StatusCode, String) { (StatusCode::BAD_REQUEST, s.to_string()) }

#[derive(Clone, Serialize, Deserialize)]
struct Project { id: String, name: String, path: String }
#[derive(Clone, Serialize, Deserialize)]
struct Message { role: String, text: String, at: u64, #[serde(default)] attachments: Vec<Attachment> }
#[derive(Clone, Serialize, Deserialize)]
struct Attachment { id: String, name: String, media_type: String }
fn image_type(kind: &str) -> bool { matches!(kind, "image/png" | "image/jpeg" | "image/webp" | "image/gif") }
fn attachment_dir(store: &Store, key: &str) -> Result<PathBuf, (StatusCode, String)> {
    uuid::Uuid::parse_str(key).map_err(bad)?;
    Ok(store.file.parent().unwrap().join("attachments").join(key))
}
fn attachment_file(dir: PathBuf, file: &Attachment) -> PathBuf {
    let extension = std::path::Path::new(&file.name).extension().and_then(|s| s.to_str()).unwrap_or("");
    if !extension.is_empty() && extension.len() <= 20 && extension.chars().all(|c| c.is_ascii_alphanumeric()) {
        dir.join(format!("file.{extension}"))
    } else { dir.join("file") }
}
fn load_attachments(store: &Store, keys: Vec<String>) -> Result<Vec<Attachment>, (StatusCode, String)> {
    keys.iter().map(|key| {
        let bytes = std::fs::read(attachment_dir(store, key)?.join("metadata.json")).map_err(bad)?;
        serde_json::from_slice(&bytes).map_err(bad)
    }).collect()
}
#[derive(Deserialize)]
struct Upload { name: String, media_type: String }
async fn upload(State(app): State<App>, axum::extract::Query(info): axum::extract::Query<Upload>, body: axum::body::Bytes) -> ApiResult {
    nonempty(&info.name, "File name")?;
    let store = app.lock().await;
    let attachment = Attachment { id: id(), name: info.name, media_type: info.media_type };
    let dir = attachment_dir(&store, &attachment.id)?;
    std::fs::create_dir_all(&dir).map_err(bad)?;
    std::fs::write(attachment_file(dir.clone(), &attachment), body).map_err(bad)?;
    std::fs::write(dir.join("metadata.json"), serde_json::to_vec(&attachment).map_err(bad)?).map_err(bad)?;
    Ok(Json(json!(attachment)))
}
async fn attachment(State(app): State<App>, Path(key): Path<String>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let store = app.lock().await;
    let files = load_attachments(&store, vec![key.clone()])?;
    let kind = if image_type(&files[0].media_type) { files[0].media_type.as_str() } else { "application/octet-stream" };
    let body = std::fs::read(attachment_file(attachment_dir(&store, &key)?, &files[0])).map_err(bad)?;
    Ok(([("content-type", kind.to_string()), ("x-content-type-options", "nosniff".into())], body))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Difficulty { Easy, Hard }
#[derive(Clone, Serialize, Deserialize)]
struct Task {
    id: String, project_id: String, title: String, model: String, status: String,
    messages: Vec<Message>, created_at: u64, queued_at: u64, elapsed_ms: u64,
    started_at: Option<u64>, thread_id: Option<String>, error: Option<String>,
    sent_count: usize, cancel: bool,
    #[serde(default)] difficulty: Option<Difficulty>,
    #[serde(default)] cleanup_at: Option<u64>,
    #[serde(default)] attention: Option<String>,
    #[serde(default)] worktree: Option<String>,
    #[serde(default)] awaiting_merge: bool,
    #[serde(default)] merging: bool,
}
impl Task {
    fn message(&mut self, role: &str, text: impl Into<String>) { self.messages.push(Message { role: role.into(), text: text.into(), at: now(), attachments: vec![] }); }
}
#[derive(Clone, Serialize, Deserialize)]
struct Template { id: String, name: String, content: String }
fn default_workers() -> usize { 4 }
#[derive(Clone, Serialize, Deserialize)]
struct Settings { #[serde(default)] hard_model: String, #[serde(default)] easy_model: String, codex_path: String, paused: bool, #[serde(default = "default_workers")] max_workers: usize, #[serde(default)] parallelism: bool }
#[derive(Clone, Serialize, Deserialize)]
struct Data { projects: Vec<Project>, tasks: Vec<Task>, templates: Vec<Template>, settings: Settings, saved_at: u64 }
struct Store { data: Data, file: PathBuf }
impl Store {
    fn commit(&mut self, mut data: Data) -> Result<(), String> {
        data.saved_at = now();
        let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
        let temp = self.file.with_extension("tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &self.file).map_err(|e| e.to_string())?;
        self.data = data;
        Ok(())
    }
}
fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default()).join(".codex")
    })
}
fn detect_codex() -> String {
    if let Ok(p) = std::env::var("67AI_CODEX") { return p; }
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            let base = PathBuf::from(roaming).join("npm/node_modules/@openai/codex/node_modules/@openai");
            for (package, target) in [("codex-win32-x64", "x86_64-pc-windows-msvc"), ("codex-win32-arm64", "aarch64-pc-windows-msvc")] {
                let path = base.join(package).join("vendor").join(target).join("bin/codex.exe");
                if path.is_file() { return path.to_string_lossy().into(); }
            }
        }
    }
    "codex".into()
}
fn models() -> Vec<String> {
    std::fs::read(codex_home().join("models_cache.json")).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["models"].as_array().cloned()).unwrap_or_default().iter()
        .filter(|m| m["visibility"] == "list").filter_map(|m| m["slug"].as_str().map(String::from)).collect()
}
fn initial_data() -> Data {
    let config = std::fs::read_to_string(codex_home().join("config.toml")).unwrap_or_default();
    let model = config.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "model").then(|| value.trim().trim_matches('"').to_string())
    }).unwrap_or_default();
    Data {
        projects: vec![], tasks: vec![], saved_at: now(),
        settings: Settings { hard_model: model.clone(), easy_model: model, codex_path: detect_codex(), paused: false, max_workers: 4, parallelism: false },
        templates: vec![
            Template { id: id(), name: "Simple & direct".into(), content: "# Project instructions\n\nUse simple, straightforward code and existing tools. Match the existing style.\nAvoid speculative abstractions and unrelated changes.\nRun relevant checks and report what changed, what passed, and any remaining issues.\nWork sequentially. Do not spawn subagents or delegate work.\n".into() },
            Template { id: id(), name: "Careful maintenance".into(), content: "# Project instructions\n\nRead the surrounding code before editing. Keep changes focused on the task.\nPreserve public interfaces unless the task requires a change.\nAdd a regression test for bug fixes where practical. Run relevant existing tests.\nReport the cause, fix, and validation. Work alone, without subagents.\n".into() },
            Template { id: id(), name: "New project".into(), content: "# Project instructions\n\nBuild the smallest complete implementation that meets the request.\nKeep dependencies minimal, use clear names, and handle errors concisely.\nDocument how to run the project. Verify the main user flow before finishing.\nWork on one task at a time. Do not use subagents.\n".into() },
        ],
    }
}

async fn state(State(app): State<App>) -> Json<Value> {
    let store = app.lock().await;
    Json(json!({"data": store.data, "models": models(), "data_file": store.file, "server_time": now()}))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    AddProject { name: String, path: String },
    CreateTask { project_id: String, title: String, text: String, model: String, status: String, #[serde(default)] difficulty: Option<Difficulty>, #[serde(default)] attachments: Vec<String> },
    EditTask { id: String, title: String, model: String },
    MoveTask { id: String, status: String },
    SendMessage { id: String, text: String, #[serde(default)] attachments: Vec<String> },
    StopTask { id: String },
    DeleteTask { id: String },
    ApproveTask { id: String },
    ReviveTask { id: String },
    RetryMerge { id: String },
    Settings { #[serde(default)] hard_model: String, #[serde(default)] easy_model: String, codex_path: String, paused: bool, #[serde(default = "default_workers")] max_workers: usize, #[serde(default)] parallelism: bool },
    SaveTemplate { id: Option<String>, name: String, content: String },
    DeleteTemplate { id: String },
    SaveAgents { project_id: String, content: String, original: String },
}
fn nonempty(s: &str, name: &str) -> Result<(), (StatusCode, String)> {
    if s.trim().is_empty() { Err(bad(format!("{name} is required"))) } else { Ok(()) }
}
fn task_mut<'a>(d: &'a mut Data, key: &str) -> Result<&'a mut Task, (StatusCode, String)> {
    d.tasks.iter_mut().find(|t| t.id == key).ok_or_else(|| bad("Task not found"))
}
fn idle(t: &Task) -> Result<(), (StatusCode, String)> {
    if t.merging { Err(bad("Wait for the merge to finish")) } else if t.status == "now" { Err(bad("Stop the running task first")) } else { Ok(()) }
}
fn valid_status(s: &str) -> Result<(), (StatusCode, String)> {
    if ["future", "todo", "done"].contains(&s) { Ok(()) } else { Err(bad("Now is controlled by the worker. Move a task to todo to run it.")) }
}
async fn action(State(app): State<App>, Json(action): Json<Action>) -> ApiResult {
    let mut store = app.lock().await;
    let mut d = store.data.clone();
    let mut result = json!({"ok": true});
    match action {
        Action::AddProject { name, path } => {
            nonempty(&name, "Project name")?;
            let path = std::fs::canonicalize(path.trim()).map_err(bad)?;
            if !path.is_dir() { return Err(bad("Choose an existing folder")); }
            let path = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            if d.projects.iter().any(|p| p.path == path) { return Err(bad("This folder is already a project")); }
            let project = Project { id: id(), name: name.trim().into(), path };
            result = json!({"id": project.id}); d.projects.push(project);
        }
        Action::CreateTask { project_id, title, text, model, status, difficulty, attachments } => {
            let attachments = load_attachments(&store, attachments)?;
            nonempty(&title, "Title")?; if attachments.is_empty() { nonempty(&text, "Task instructions")?; } valid_status(&status)?;
            if status == "done" { return Err(bad("New tasks must start in future or todo")); }
            if !d.projects.iter().any(|p| p.id == project_id) { return Err(bad("Project not found")); }
            let mut task = Task { id: id(), project_id, title: title.trim().into(), model, status, difficulty, messages: vec![], created_at: now(), queued_at: now(), elapsed_ms: 0, started_at: None, thread_id: None, error: None, sent_count: 0, cancel: false, cleanup_at: None, attention: None, worktree: None, awaiting_merge: false, merging: false };
            task.message("user", text.trim()); task.messages.last_mut().unwrap().attachments = attachments;
            result = json!({"id": task.id}); d.tasks.push(task);
        }
        Action::EditTask { id, title, model } => {
            nonempty(&title, "Title")?; let t = task_mut(&mut d, &id)?; idle(t)?;
            t.title = title.trim().into(); t.model = model;
        }
        Action::MoveTask { id, status } => {
            valid_status(&status)?; let t = task_mut(&mut d, &id)?; idle(t)?;
            if t.awaiting_merge { return Err(bad("Merge this task before moving it")); }
            if status == "done" && t.worktree.is_some() { return Err(bad("Worktree tasks must complete and merge before being marked done")); }
            t.cleanup_at = None;
            if status == "todo" {
                if t.sent_count >= t.messages.len() || !t.messages[t.sent_count..].iter().any(|m| m.role == "user") {
                    t.message("user", "Continue this task. Check the current work and complete any remaining steps, then report the result.");
                }
                t.queued_at = now(); t.error = None; t.cancel = false; t.attention = None;
            }
            t.status = status;
        }
        Action::SendMessage { id, text, attachments } => {
            let attachments = load_attachments(&store, attachments)?;
            if attachments.is_empty() { nonempty(&text, "Message")?; } let t = task_mut(&mut d, &id)?;
            if t.awaiting_merge { return Err(bad("Wait for this task to merge before sending a follow-up")); }
            t.message("user", text.trim());
            t.messages.last_mut().unwrap().attachments = attachments;
            t.cleanup_at = None;
            if t.status == "done" { t.status = "todo".into(); t.queued_at = now(); }
            t.error = None; if t.status != "now" { t.attention = None; }
        }
        Action::StopTask { id } => {
            let t = task_mut(&mut d, &id)?;
            if t.status != "now" { return Err(bad("Task is not running")); } t.cancel = true;
        }
        Action::DeleteTask { id } => { let t = task_mut(&mut d, &id)?; idle(t)?; if t.awaiting_merge { return Err(bad("Merge this task before deleting it")); } d.tasks.retain(|t| t.id != id); }
        Action::ApproveTask { id } => {
            let t = task_mut(&mut d, &id)?;
            if t.status != "done" { return Err(bad("Only done tasks can be approved")); }
            if t.awaiting_merge { return Err(bad("Merge this task before approving it")); }
            if t.attention.is_some() { return Err(bad("Answer or retry this unfinished task before approving it")); }
            if t.cleanup_at.is_none() { t.cleanup_at = Some(now() + 30 * 60 * 1000); }
            t.error = None;
        }
        Action::RetryMerge { id } => {
            let t = task_mut(&mut d, &id)?; idle(t)?;
            if !t.awaiting_merge { return Err(bad("Task is not waiting to merge")); }
            t.error = None;
        }
        Action::ReviveTask { id } => {
            let t = task_mut(&mut d, &id)?; idle(t)?;
            t.cleanup_at = None; t.error = None;
        }
        Action::Settings { hard_model, easy_model, codex_path, paused, max_workers, parallelism } => {
            nonempty(&codex_path, "Codex executable")?;
            if max_workers == 0 { return Err(bad("Worker count must be at least 1")); }
            d.settings = Settings { hard_model: hard_model.trim().into(), easy_model: easy_model.trim().into(), codex_path: codex_path.trim().into(), paused, max_workers, parallelism };
        }
        Action::SaveTemplate { id: key, name, content } => {
            nonempty(&name, "Template name")?;
            if let Some(key) = key {
                let t = d.templates.iter_mut().find(|t| t.id == key).ok_or_else(|| bad("Template not found"))?;
                t.name = name.trim().into(); t.content = content;
            } else { d.templates.push(Template { id: id(), name: name.trim().into(), content }); }
        }
        Action::DeleteTemplate { id } => { d.templates.retain(|t| t.id != id); }
        Action::SaveAgents { project_id, content, original } => {
            if d.tasks.iter().any(|t| t.project_id == project_id && (t.status == "now" || t.merging)) { return Err(bad("Wait for this project's running task to finish before editing instructions")); }
            let p = d.projects.iter().find(|p| p.id == project_id).ok_or_else(|| bad("Project not found"))?;
            let file = PathBuf::from(&p.path).join("AGENTS.md");
            let current = read_optional(&file).map_err(bad)?;
            if current != original { return Err((StatusCode::CONFLICT, "AGENTS.md changed on disk. Reopen it before saving.".into())); }
            std::fs::write(file, content).map_err(bad)?;
        }
    }
    store.commit(d).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}
fn read_optional(file: &std::path::Path) -> Result<String, std::io::Error> {
    match std::fs::read_to_string(file) { Ok(s) => Ok(s), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()), Err(e) => Err(e) }
}
async fn agents(State(app): State<App>, Path(key): Path<String>) -> ApiResult {
    let store = app.lock().await;
    let p = store.data.projects.iter().find(|p| p.id == key).ok_or_else(|| bad("Project not found"))?;
    let root = PathBuf::from(&p.path);
    let file = root.join("AGENTS.md");
    let mut inherited = vec![];
    let mut dirs: Vec<PathBuf> = root.ancestors().skip(1).map(PathBuf::from).collect();
    dirs.reverse(); dirs.insert(0, codex_home()); dirs.push(root.clone());
    for dir in dirs {
        for name in ["AGENTS.override.md", "AGENTS.md"] {
            let path = dir.join(name);
            if path.is_file() {
                if path != file { inherited.push(json!({"path": path, "content": read_optional(&path).map_err(bad)?})); }
                break;
            }
        }
    }
    Ok(Json(json!({"path": file, "exists": file.is_file(), "content": read_optional(&file).map_err(bad)?, "inherited": inherited, "overridden": root.join("AGENTS.override.md").is_file()})))
}

// Reject cross-origin access to a service that can edit local files and run Codex.
async fn local_only(headers: HeaderMap, req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = host.split(':').next().unwrap_or("");
    if !["127.0.0.1", "localhost"].contains(&name) { return (StatusCode::FORBIDDEN, "Local access only").into_response(); }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if origin != format!("http://{host}") { return (StatusCode::FORBIDDEN, "Cross-origin request rejected").into_response(); }
    }
    if req.uri().path().starts_with("/api/") && headers.get("sec-fetch-site").is_some_and(|v| v == "cross-site") { return (StatusCode::FORBIDDEN, "Cross-site request rejected").into_response(); }
    let mut response = next.run(req).await;
    response.headers_mut().insert("cache-control", "no-store".parse().unwrap());
    response.headers_mut().insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data: blob:; frame-ancestors 'none'".parse().unwrap());
    response
}

async fn update_task(app: &App, key: &str, f: impl FnOnce(&mut Task)) -> Result<(), String> {
    let mut store = app.lock().await;
    let mut d = store.data.clone();
    if let Some(task) = d.tasks.iter_mut().find(|t| t.id == key) { f(task); }
    store.commit(d)
}

fn event(task: &mut Task, value: &Value) -> bool {
    match value["type"].as_str().unwrap_or("") {
        "thread.started" => { task.thread_id = value["thread_id"].as_str().map(String::from); }
        "item.completed" => {
            let item = &value["item"];
            match item["type"].as_str().unwrap_or("") {
                "agent_message" => if let Some(text) = item["text"].as_str() {
                    if let Ok(reply) = serde_json::from_str::<Value>(text) {
                        if let (Some(outcome), Some(message)) = (reply["outcome"].as_str(), reply["message"].as_str()) {
                            if matches!(outcome, "completed" | "question" | "blocked" | "refused") && !message.trim().is_empty() {
                                task.attention = if outcome == "completed" { None } else { Some(outcome.into()) };
                                task.message("assistant", message);
                                return true;
                            }
                        }
                    }
                    task.attention = Some("unconfirmed".into());
                    task.message("assistant", text);
                },
                "command_execution" => {
                    let command = item["command"].as_str().unwrap_or("Command");
                    let output: String = item["aggregated_output"].as_str().unwrap_or("").chars().take(12000).collect();
                    task.message("activity", format!("{command}\nExit: {}\n{output}", item["exit_code"]));
                }
                "file_change" => task.message("activity", format!("Files changed\n{}", item["changes"])),
                "mcp_tool_call" => task.message("activity", format!("Tool: {} / {} — {}", item["server"], item["tool"], item["status"])),
                _ => {}
            }
        }
        "turn.failed" | "error" => {
            let text = value["error"]["message"].as_str().or(value["message"].as_str()).unwrap_or("Codex reported an error");
            task.message("system", text); return false;
        }
        _ => {}
    }
    true
}

// A Windows job closes the entire process tree when a run stops or this server exits.
#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::{System::JobObjects::*, Foundation::*};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
            let job = Self(handle);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(handle, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if AssignProcessToJobObject(handle, child.raw_handle().unwrap() as HANDLE) == 0 { return Err(std::io::Error::last_os_error().to_string()); }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob { fn drop(&mut self) { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); } } }

fn recovery_event(task: &mut Task, recovery: &mut Task, value: &Value) -> bool {
    let ok = event(recovery, value);
    for message in recovery.messages.drain(..) { task.message("activity", format!("Main worktree recovery: {}", message.text)); }
    ok
}
async fn run_codex(app: &App, task: &Task, project: &Project, settings: &Settings, shutdown: &mut watch::Receiver<bool>, recovery: bool) -> Result<(), String> {
    if *shutdown.borrow() || app.lock().await.data.tasks.iter().any(|t| t.id == task.id && t.cancel) { return Err("Run stopped.".into()); }
    let mut recovery_state = task.clone();
    recovery_state.messages.clear();
    let mut images = vec![];
    let store = app.lock().await;
    let schema = store.file.parent().unwrap().join("response-schema.json");
    let mut parts = vec![];
    for message in task.messages[task.sent_count..].iter().filter(|m| m.role == "user") {
        parts.push(message.text.clone());
        for file in &message.attachments {
            let path = std::fs::canonicalize(attachment_file(attachment_dir(&store, &file.id).map_err(|e| e.1)?, file)).map_err(|e| e.to_string())?;
            let path = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            parts.push(format!("Attached file: {}\nPath: {}", file.name, path));
            if image_type(&file.media_type) { images.push(path); }
        }
    }
    drop(store);
    let prompt = parts.join("\n\n");
    let mut prompt = format!("Task: {}\n\n{}", task.title, prompt);
    if task.worktree.is_some() {
        prompt.push_str("\n\nWork only in this task's Git worktree and on its existing 67ai branch. Do not switch branches, merge into main, or modify other worktrees. Commit completed changes here; 67ai will merge them into main separately.");
    }
    let mut command = Command::new(&settings.codex_path);
    command.arg("exec");
    if let Some(thread) = &task.thread_id { command.args(["resume", thread]); }
    command.args(["--dangerously-bypass-approvals-and-sandbox", "--json", "--skip-git-repo-check"]);
    command.arg("--output-schema").arg(&schema);
    if !task.model.is_empty() { command.args(["--model", &task.model]); }
    for path in images { command.arg("--image").arg(path); }
    command.arg("--");
    command.arg("-").current_dir(&project.path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| format!("Could not start Codex: {e}. Check Settings → Codex executable."))?;
    #[cfg(windows)]
    let _job = match ProcessJob::attach(&child) {
        Ok(job) => job, Err(e) => { let _ = child.kill().await; return Err(format!("Could not contain Codex process: {e}")); }
    };
    let mut input = child.stdin.take().unwrap();
    input.write_all(prompt.as_bytes()).await.map_err(|e| e.to_string())?;
    drop(input);
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut err = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut stderr = String::new();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut completed = false;
    let mut failed = false;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    let mut last_checkpoint = now();
    loop {
        tokio::select! {
            line = out.next_line(), if !stdout_done => match line.map_err(|e| e.to_string())? {
                Some(line) => if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if value["type"] == "turn.completed" { completed = true; }
                    update_task(app, &task.id, |t| {
                        if recovery {
                            if !recovery_event(t, &mut recovery_state, &value) { failed = true; }
                        } else if !event(t, &value) { failed = true; }
                    }).await?;
                },
                None => stdout_done = true,
            },
            line = err.next_line(), if !stderr_done => match line.map_err(|e| e.to_string())? {
                Some(line) => { stderr.push_str(&line); stderr.push('\n'); if stderr.len() > 16000 { stderr = stderr.chars().rev().take(8000).collect::<String>().chars().rev().collect(); } },
                None => stderr_done = true,
            },
            _ = shutdown.changed() => { let _ = child.kill().await; return Err("Server stopped. Move the task to todo to resume.".into()); },
            _ = tick.tick() => {
                let mut store = app.lock().await;
                if store.data.tasks.iter().any(|t| t.id == task.id && t.cancel) {
                    drop(store); let _ = child.kill().await; return Err("Stopped by you. Move the task to todo to resume.".into());
                }
                if now().saturating_sub(last_checkpoint) >= 5000 {
                    let d = store.data.clone(); store.commit(d)?; last_checkpoint = now();
                }
            }
        }
        if stdout_done && stderr_done { break; }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() || !completed || failed {
        return Err(format!("Codex did not finish successfully ({status}). {}", stderr.trim()));
    }
    if recovery && recovery_state.attention.is_some() { return Err("Hard-model recovery is unfinished. See the recovery messages and retry when resolved.".into()); }
    Ok(())
}

fn next_task(d: &Data) -> Option<usize> {
    let limit = if d.settings.parallelism { d.settings.max_workers } else { 1 };
    if d.settings.paused || d.tasks.iter().filter(|t| t.status == "now").count() >= limit { return None; }
    d.tasks.iter().enumerate().filter(|(_, t)| t.status == "todo" && t.error.is_none()
        && !d.tasks.iter().any(|other| other.project_id == t.project_id
            && (((other.merging || (other.awaiting_merge && !d.settings.parallelism)) && t.worktree.is_none())
                || (other.status == "now" && (!d.settings.parallelism || other.worktree.is_none())))))
        .min_by_key(|(_, t)| t.queued_at).map(|(i, _)| i)
}

async fn git(path: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.current_dir(path).args(args).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command.output().await.map_err(|e| e.to_string())?;
    if !output.status.success() { return Err(format!("git {}: {}{}", args[0], String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))); }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
async fn require_main(project: &Project) -> Result<(), String> {
    if !git(&project.path, &["rev-parse", "--show-prefix"]).await?.is_empty() {
        return Err("Parallel tasks require the project folder to be the Git repository root.".into());
    }
    if git(&project.path, &["branch", "--show-current"]).await? != "main" {
        return Err("Parallel tasks require the project's main branch to be checked out.".into());
    }
    if !git(&project.path, &["status", "--porcelain"]).await?.is_empty() {
        return Err(DIRTY_MAIN.into());
    }
    Ok(())
}
async fn recover_main(app: &App, task: &Task, project: &Project, settings: &Settings, shutdown: &mut watch::Receiver<bool>) -> Result<(), String> {
    match require_main(project).await {
        Ok(()) => return Ok(()),
        Err(e) if e == DIRTY_MAIN => {},
        Err(e) => return Err(e),
    }
    if settings.hard_model.trim().is_empty() { return Err("Main worktree is dirty. Select a hard-task model in Settings for automatic recovery.".into()); }
    update_task(app, &task.id, |t| t.message("system", "Main worktree has uncommitted changes. Running the hard-task model to fix it, then retrying automatically.")).await?;
    let mut repair = task.clone();
    repair.title = "Recover main worktree for parallel tasks".into();
    repair.model = settings.hard_model.clone();
    repair.thread_id = None;
    repair.worktree = None;
    repair.sent_count = 0;
    repair.attention = Some("unconfirmed".into());
    repair.messages.clear();
    repair.message("user", "Parallel task startup or merge was blocked by uncommitted changes in this project's main worktree. Inspect git status and the diffs, preserve all existing work, and commit the changes appropriately so main is clean. You are authorized to fix this Git blocker only. Stay on main in this project; do not modify other worktrees, merge task branches, push, deploy, discard changes, or use destructive reset/clean commands. Do not implement the original task. If you cannot safely preserve and commit the changes, report the blocker. 67ai will verify main is clean and retry the original operation automatically.");
    run_codex(app, &repair, project, settings, shutdown, true).await?;
    require_main(project).await.map_err(|e| format!("Main worktree recovery did not resolve the blocker: {e}"))?;
    update_task(app, &task.id, |t| t.message("system", "Main worktree recovered. Retrying automatically.")).await
}
async fn prepare_worktree(app: &App, task: &mut Task, project: &mut Project, settings: &Settings, shutdown: &mut watch::Receiver<bool>) -> Result<(), String> {
    let _main_git = MAIN_GIT.lock().await;
    if task.worktree.is_none() && settings.parallelism {
        recover_main(app, task, project, settings, shutdown).await?;
        let dir = {
            let store = app.lock().await;
            store.file.parent().unwrap().join("worktrees")
        };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dir = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
        let path = dir.join(&task.id).to_string_lossy().trim_start_matches(r"\\?\").to_string();
        // Persist the location first so interrupted setup can be retried safely.
        update_task(app, &task.id, |t| { t.worktree = Some(path.clone()); t.thread_id = None; t.sent_count = 0; }).await?;
        task.worktree = Some(path); task.thread_id = None; task.sent_count = 0;
    }
    if let Some(path) = &task.worktree {
        let branch = format!("67ai/{}", task.id);
        if !std::path::Path::new(path).exists() {
            recover_main(app, task, project, settings, shutdown).await?;
            git(&project.path, &["worktree", "add", "-b", &branch, path, "main"]).await?;
        }
        let current_branch = git(path, &["branch", "--show-current"]).await?;
        if current_branch != branch && current_branch != format!("taskdesk/{}", task.id) { return Err("Task worktree is on an unexpected branch.".into()); }
        project.path = path.clone();
    }
    Ok(())
}
async fn merge_task(task: &Task, project: &Project) -> Result<(), String> {
    let path = task.worktree.as_deref().ok_or("Missing task worktree")?;
    require_main(project).await?;
    let branch = git(path, &["branch", "--show-current"]).await?;
    if branch != format!("67ai/{}", task.id) && branch != format!("taskdesk/{}", task.id) { return Err("Task worktree is on an unexpected branch.".into()); }
    git(path, &["add", "--all"]).await?;
    if !git(path, &["diff", "--cached", "--name-only"]).await?.is_empty() {
        git(path, &["commit", "-m", &format!("Complete task: {}", task.title)]).await?;
    }
    if let Err(error) = git(&project.path, &["merge", "--no-edit", &branch]).await {
        // Abort only the merge we started; preserve both branches for a retry.
        if git(&project.path, &["rev-parse", "--verify", "MERGE_HEAD"]).await.is_ok() {
            git(&project.path, &["merge", "--abort"]).await?;
        }
        return Err(format!("Merge failed. Resolve the task branch against main, then retry. {error}"));
    }
    Ok(())
}
async fn merge_worker(app: App, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() { break; }
        let job = {
            let mut store = app.lock().await;
            let mut d = store.data.clone();
            let index = if d.settings.paused { None } else { d.tasks.iter().position(|t| t.awaiting_merge && !t.merging && t.error.is_none()
                && !d.tasks.iter().any(|other| other.project_id == t.project_id && other.status == "now" && other.worktree.is_none())) };
            if let Some(index) = index {
                d.tasks[index].merging = true;
                let task = d.tasks[index].clone();
                let project = d.projects.iter().find(|p| p.id == task.project_id).unwrap().clone();
                let settings = d.settings.clone();
                match store.commit(d) { Ok(()) => Some((task, project, settings)), Err(e) => { eprintln!("Cannot persist merge: {e}"); None } }
            } else { None }
        };
        if let Some((task, project, settings)) = job {
            let result = {
                let _main_git = MAIN_GIT.lock().await;
                match recover_main(&app, &task, &project, &settings, &mut shutdown).await {
                    Ok(()) => merge_task(&task, &project).await,
                    Err(e) => Err(e),
                }
            };
            if let Err(e) = update_task(&app, &task.id, |t| {
                t.merging = false;
                match result {
                    Ok(()) => { t.awaiting_merge = false; t.error = None; t.message("system", "Merged into main."); }
                    Err(e) => { t.error = Some(e.clone()); t.message("system", e); }
                }
            }).await { eprintln!("Cannot persist merge result: {e}"); }
        } else {
            tokio::select! { _ = shutdown.changed() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {} }
        }
    }
}
async fn worker(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut runs = tokio::task::JoinSet::new();
    loop {
        if *shutdown.borrow() { break; }
        let job = {
            let mut store = app.lock().await;
            let mut d = store.data.clone();
            if let Some(index) = next_task(&d) {
                let t = &mut d.tasks[index]; t.status = "now".into(); t.started_at = Some(now()); t.cancel = false;
                t.attention = Some("unconfirmed".into());
                t.message("system", "Codex started working.");
                let task = t.clone();
                let project = d.projects.iter().find(|p| p.id == task.project_id).unwrap().clone();
                let settings = d.settings.clone();
                if let Err(e) = store.commit(d) { eprintln!("Cannot persist queue: {e}"); None } else { Some((task, project, settings)) }
            } else { None }
        };
        if let Some((mut task, mut project, settings)) = job {
            let app = app.clone();
            let mut shutdown = shutdown.clone();
            runs.spawn(async move {
                let result = match prepare_worktree(&app, &mut task, &mut project, &settings, &mut shutdown).await {
                    Ok(()) => run_codex(&app, &task, &project, &settings, &mut shutdown, false).await,
                    Err(e) => Err(e),
                };
                let stopped = *shutdown.borrow();
                if let Err(e) = update_task(&app, &task.id, |t| {
                    t.elapsed_ms += now().saturating_sub(t.started_at.unwrap_or(now())); t.started_at = None;
                    match result {
                        Ok(()) => {
                            t.sent_count = task.messages.len();
                            let pending = t.messages[t.sent_count..].iter().any(|m| m.role == "user");
                            t.status = if pending { "todo" } else { "done" }.into();
                            t.queued_at = now(); t.error = None;
                            if pending { t.attention = None; }
                            t.awaiting_merge = !pending && t.attention.is_none() && t.worktree.is_some();
                            t.message("system", if pending { "Run ended. Your follow-up is queued." } else if t.attention.is_some() { "Task unfinished. Reply below to continue." } else { "Run completed." });
                        }
                        Err(e) => { t.status = if t.cancel || stopped { "future" } else { "done" }.into(); t.attention = if t.cancel || stopped { None } else { Some("failed".into()) }; t.error = Some(e.clone()); t.message("system", e); }
                    }
                    t.cancel = false;
                }).await { eprintln!("Cannot persist result: {e}"); }
            });
        } else {
            tokio::select! {
                _ = shutdown.changed() => break,
                result = runs.join_next(), if !runs.is_empty() => {
                    if let Some(Err(e)) = result { eprintln!("Task worker failed: {e}"); }
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {}
            }
        }
    }
    while runs.join_next().await.is_some() {}
}

async fn delete_session(executable: &str, thread: &str) -> Result<(), String> {
    codex_request(executable, "thread/delete", json!({"threadId":thread})).await.map(|_| ())
}

async fn rate_limits(State(app): State<App>) -> Json<Value> {
    static CACHE: std::sync::OnceLock<Mutex<(String, u64, Value)>> = std::sync::OnceLock::new();
    let executable = app.lock().await.data.settings.codex_path.clone();
    let mut cache = CACHE.get_or_init(|| Mutex::new((String::new(), 0, Value::Null))).lock().await;
    if cache.0 != executable || now().saturating_sub(cache.1) >= 60_000 {
        let result = match codex_request(&executable, "account/rateLimits/read", Value::Null).await {
            Ok(value) => value,
            Err(_) => json!({"error":"Codex usage unavailable. Check your Codex login and executable in Settings."}),
        };
        *cache = (executable, now(), result);
    }
    Json(cache.2.clone())
}

async fn codex_request(executable: &str, method: &str, params: Value) -> Result<Value, String> {
    let mut command = Command::new(executable);
    command.args(["app-server", "--stdio"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for request in [
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"67ai","version":"0.1.0"},"capabilities":null}}),
            json!({"id":2,"method":method,"params":params}),
        ] {
            input.write_all(format!("{request}\n").as_bytes()).await.map_err(|e| e.to_string())?;
            loop {
                let line = output.next_line().await.map_err(|e| e.to_string())?.ok_or("Codex closed before responding")?;
                let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                if value["id"] != request["id"] { continue; }
                if let Some(error) = value.get("error") { return Err(error.to_string()); }
                if value.get("result").is_none() { return Err("Invalid Codex response".into()); }
                if request["id"] == 2 { return Ok(value["result"].clone()); }
                break;
            }
            if request["id"] == 1 {
                input.write_all(b"{\"method\":\"initialized\"}\n").await.map_err(|e| e.to_string())?;
            }
        }
        Err("Missing Codex response".into())
    }).await.unwrap_or_else(|_| Err("Codex request timed out".into()));
    let _ = child.kill().await;
    result
}

async fn cleanup_approved(app: &App) {
    // Serialize expiry with revive/send actions so a cancelled timer cannot delete a session.
    let mut store = app.lock().await;
    let expired: Vec<Task> = store.data.tasks.iter().filter(|t| t.status == "done" && t.cleanup_at.is_some_and(|at| at <= now())).cloned().collect();
    for task in expired {
        let result = if let Some(thread) = &task.thread_id {
            delete_session(&store.data.settings.codex_path, thread).await
        } else { Ok(()) };
        let mut d = store.data.clone();
        if let Err(e) = result {
            let t = d.tasks.iter_mut().find(|t| t.id == task.id).unwrap();
            t.cleanup_at = None;
            t.error = Some(format!("Cleanup failed; task kept. Approve to retry: {e}"));
        } else {
            d.tasks.retain(|t| t.id != task.id);
        }
        if let Err(e) = store.commit(d) { eprintln!("Cannot persist cleanup: {e}"); break; }
        if store.data.tasks.iter().any(|t| t.id == task.id) { continue; }
        for file in task.messages.iter().flat_map(|m| &m.attachments) {
            if store.data.tasks.iter().flat_map(|t| &t.messages).flat_map(|m| &m.attachments).any(|f| f.id == file.id) { continue; }
            if let Ok(dir) = attachment_dir(&store, &file.id) {
                let _ = std::fs::remove_file(attachment_file(dir.clone(), file));
                let _ = std::fs::remove_file(dir.join("metadata.json"));
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
}

async fn cleanup_worker(app: App, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() { break; }
        cleanup_approved(&app).await;
        tokio::select! { _ = shutdown.changed() => break, _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {} }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("67AI_PORT").unwrap_or_else(|_| "3030".into()).parse::<u16>()?;
    // Bind before loading state: a second launch must not recover another server's running tasks.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let dir = match std::env::var_os("67AI_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir()?.join("data"),
    };
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("67ai.lock"))?;
    lock.try_lock().map_err(|_| "This 67ai data directory is already open in another server")?;
    std::fs::write(dir.join("response-schema.json"), include_str!("response-schema.json"))?;
    let file = dir.join("67ai.json");
    // Keep the previous installation locked while migrating its saved board.
    let legacy_lock = if dir.join("taskdesk.lock").exists() {
        let lock = std::fs::OpenOptions::new().write(true).open(dir.join("taskdesk.lock"))?;
        lock.try_lock().map_err(|_| "Stop the previous server before starting 67ai")?;
        Some(lock)
    } else { None };
    if !file.exists() && dir.join("taskdesk.json").exists() {
        std::fs::rename(dir.join("taskdesk.json"), &file)?;
    }
    let _legacy_lock = legacy_lock;
    let mut data: Data = if file.exists() { serde_json::from_slice(&std::fs::read(&file)?)? } else { initial_data() };
    for t in &mut data.tasks {
        t.merging = false;
        if t.status == "now" {
            t.elapsed_ms += data.saved_at.saturating_sub(t.started_at.unwrap_or(data.saved_at));
            t.status = "future".into(); t.started_at = None; t.cancel = false;
            t.error = Some("Server was interrupted. Move to todo to resume.".into());
            t.message("system", "Previous run was interrupted. Review and move to todo to resume.");
        }
    }
    let mut store = Store { data: data.clone(), file }; store.commit(data)?;
    let app: App = Arc::new(Mutex::new(store));
    let (stop, shutdown) = watch::channel(false);
    let cleanup = tokio::spawn(cleanup_worker(app.clone(), shutdown.clone()));
    let merger = tokio::spawn(merge_worker(app.clone(), shutdown.clone()));
    let worker = tokio::spawn(worker(app.clone(), shutdown));
    let router = Router::new()
        .route("/", get(|| async { Html(include_str!("../web/index.html")) }))
        .route("/app.js", get(|| async { ([("content-type", "text/javascript")], include_str!("../web/app.js")) }))
        .route("/style.css", get(|| async { ([("content-type", "text/css")], include_str!("../web/style.css")) }))
        .route("/api/state", get(state)).route("/api/action", post(action)).route("/api/agents/{id}", get(agents))
        .route("/api/rate-limits", get(rate_limits))
        .route("/api/attachments", post(upload).layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024)))
        .route("/api/attachments/{id}", get(attachment))
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(axum::middleware::from_fn(local_only)).with_state(app);
    println!("67ai running at http://127.0.0.1:{port}");
    axum::serve(listener, router).with_graceful_shutdown(async move { let _ = tokio::signal::ctrl_c().await; let _ = stop.send(true); }).await?;
    let _ = worker.await;
    let _ = merger.await;
    let _ = cleanup.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn difficulty_survives_reload_and_accepts_older_tasks() {
        for (name, difficulty) in [("easy", Difficulty::Easy), ("hard", Difficulty::Hard)] {
            let mut t = task("future", 0);
            t.difficulty = Some(difficulty);
            let mut saved = serde_json::to_value(&t).unwrap();
            assert_eq!(saved["difficulty"], name);
            let loaded: Task = serde_json::from_value(saved.clone()).unwrap();
            assert_eq!(serde_json::to_value(loaded).unwrap()["difficulty"], name);
            saved.as_object_mut().unwrap().remove("difficulty");
            assert!(serde_json::from_value::<Task>(saved).unwrap().difficulty.is_none());
        }
        assert!(serde_json::from_value::<Difficulty>(json!("other")).is_err());
    }
    #[tokio::test]
    async fn approval_revive_feedback_restart_and_expiry() {
        let dir = std::env::current_dir().unwrap().join("test-results").join(format!("approval-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = initial_data();
        d.settings.paused = true;
        d.settings.codex_path = dir.join("missing-codex.exe").to_string_lossy().into();
        let t = task("done", 0); let key = t.id.clone(); d.tasks.push(t);
        let app = Arc::new(Mutex::new(Store { data: d, file: dir.join("67ai.json") }));
        let _ = action(State(app.clone()), Json(Action::ApproveTask { id: key.clone() })).await.unwrap();
        let deadline = app.lock().await.data.tasks[0].cleanup_at.unwrap();
        assert!(deadline > now() + 1_790_000);
        cleanup_approved(&app).await;
        assert_eq!(app.lock().await.data.tasks.len(), 1);
        let _ = action(State(app.clone()), Json(Action::ApproveTask { id: key.clone() })).await.unwrap();
        assert_eq!(app.lock().await.data.tasks[0].cleanup_at, Some(deadline));
        let _ = action(State(app.clone()), Json(Action::ReviveTask { id: key.clone() })).await.unwrap();
        assert_eq!(app.lock().await.data.tasks[0].cleanup_at, None);
        let _ = action(State(app.clone()), Json(Action::ApproveTask { id: key.clone() })).await.unwrap();
        let _ = action(State(app.clone()), Json(Action::SendMessage { id: key.clone(), text: "Fix the result".into(), attachments: vec![] })).await.unwrap();
        {
            let store = app.lock().await;
            assert_eq!(store.data.tasks[0].status, "todo");
            assert_eq!(store.data.tasks[0].cleanup_at, None);
            assert_eq!(next_task(&store.data), None);
        }
        assert!(action(State(app.clone()), Json(Action::ApproveTask { id: key.clone() })).await.is_err());
        let _ = action(State(app.clone()), Json(Action::MoveTask { id: key.clone(), status: "done".into() })).await.unwrap();
        let _ = action(State(app.clone()), Json(Action::ApproveTask { id: key.clone() })).await.unwrap();
        {
            let mut store = app.lock().await;
            let restored: Data = serde_json::from_slice(&std::fs::read(&store.file).unwrap()).unwrap();
            assert!(restored.tasks[0].cleanup_at.is_some());
            store.data = restored;
            store.data.tasks[0].cleanup_at = Some(0);
            store.data.tasks[0].thread_id = Some(id());
        }
        cleanup_approved(&app).await;
        {
            let mut store = app.lock().await;
            assert!(store.data.tasks[0].error.as_ref().unwrap().contains("Cleanup failed"));
            assert_eq!(store.data.tasks[0].cleanup_at, None);
            store.data.tasks[0].thread_id = None;
            store.data.tasks[0].cleanup_at = Some(0);
        }
        cleanup_approved(&app).await;
        assert!(app.lock().await.data.tasks.is_empty());
    }

    #[tokio::test]
    async fn attachments_save_with_messages_and_survive_reload() {
        let dir = std::env::current_dir().unwrap().join("test-results").join(format!("attachments-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut data = initial_data();
        data.projects.push(Project { id: "p".into(), name: "Test".into(), path: dir.to_string_lossy().into() });
        let app = Arc::new(Mutex::new(Store { data, file: dir.join("67ai.json") }));
        let Json(uploaded) = upload(State(app.clone()), axum::extract::Query(Upload { name: "notes.txt".into(), media_type: "text/plain".into() }), axum::body::Bytes::from_static(b"file contents")).await.unwrap();
        let key = uploaded["id"].as_str().unwrap().to_string();
        let Json(created) = action(State(app.clone()), Json(Action::CreateTask { project_id: "p".into(), title: "Files".into(), text: "".into(), model: "".into(), status: "future".into(), difficulty: None, attachments: vec![key.clone()] })).await.unwrap();
        let task_id = created["id"].as_str().unwrap().to_string();
        let _ = action(State(app.clone()), Json(Action::SendMessage { id: task_id.clone(), text: "".into(), attachments: vec![key.clone()] })).await.unwrap();
        assert!(action(State(app.clone()), Json(Action::SendMessage { id: task_id.clone(), text: "".into(), attachments: vec![] })).await.is_err());
        assert!(action(State(app.clone()), Json(Action::SendMessage { id: task_id, text: "bad".into(), attachments: vec!["../67ai.json".into()] })).await.is_err());
        let saved: Data = serde_json::from_slice(&std::fs::read(dir.join("67ai.json")).unwrap()).unwrap();
        assert_eq!(saved.tasks[0].messages.len(), 2);
        assert_eq!(saved.tasks[0].messages[0].attachments[0].name, "notes.txt");
        assert_eq!(saved.tasks[0].messages[1].attachments[0].id, key);
        let response = attachment(State(app), Path(key)).await.unwrap().into_response();
        assert_eq!(response.headers()["content-type"], "application/octet-stream");
        assert_eq!(axum::body::to_bytes(response.into_body(), 100).await.unwrap(), "file contents");
        let old: Message = serde_json::from_value(json!({"role":"user","text":"old","at":0})).unwrap();
        assert!(old.attachments.is_empty());
    }
    fn task(status: &str, queued_at: u64) -> Task {
        Task { id: id(), project_id: "p".into(), title: "Test".into(), model: "".into(), status: status.into(), difficulty: None, messages: vec![], created_at: 0, queued_at, elapsed_ms: 0, started_at: None, thread_id: None, error: None, sent_count: 0, cancel: false, cleanup_at: None, attention: None, worktree: None, awaiting_merge: false, merging: false }
    }
    #[test]
    fn queue_is_fifo_and_sequential_by_default() {
        let mut d = initial_data(); d.tasks = vec![task("future", 0), task("todo", 20), task("todo", 10)];
        assert_eq!(next_task(&d), Some(2));
        d.tasks[2].status = "now".into(); assert_eq!(next_task(&d), None);
        d.tasks.push(task("todo", 30)); d.tasks[3].project_id = "other".into();
        assert_eq!(next_task(&d), None);
        d.tasks[3].status = "now".into(); assert_eq!(next_task(&d), None);
        d.tasks[2].status = "done".into(); d.tasks[3].status = "done".into(); assert_eq!(next_task(&d), Some(1));
        d.settings.paused = true; assert_eq!(next_task(&d), None);
        d.settings.paused = false; d.tasks[1].error = Some("failed".into()); assert_eq!(next_task(&d), None);
    }
    #[test]
    fn parallel_queue_respects_global_limit_and_legacy_settings() {
        let mut d = initial_data();
        let mut old = serde_json::to_value(&d.settings).unwrap();
        old.as_object_mut().unwrap().remove("max_workers");
        old.as_object_mut().unwrap().remove("parallelism");
        let settings: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(settings.max_workers, 4); assert!(!settings.parallelism);
        d.settings.parallelism = true; d.settings.max_workers = 2;
        d.tasks = vec![task("now", 0), task("todo", 1), task("todo", 2)];
        assert_eq!(next_task(&d), None); // An existing main-worktree run must finish first.
        d.tasks[0].worktree = Some("isolated".into());
        assert_eq!(next_task(&d), Some(1));
        d.tasks[1].status = "now".into(); d.tasks[1].worktree = Some("second".into());
        d.tasks[2].project_id = "other".into();
        assert_eq!(next_task(&d), None); // The limit spans projects.
        d.tasks[0].status = "done".into(); d.tasks[0].awaiting_merge = true;
        assert_eq!(next_task(&d), Some(2)); // Waiting merges do not consume a task worker.
        d.settings.parallelism = false;
        assert_eq!(next_task(&d), None);
        d.tasks[1].status = "done".into();
        assert_eq!(next_task(&d), Some(2));
        d.tasks[2].project_id = "p".into();
        assert_eq!(next_task(&d), None); // Drain merges before starting a direct main run.
    }
    #[test]
    fn captures_thread_messages_and_failures() {
        let mut t = task("now", 0);
        assert!(event(&mut t, &json!({"type":"thread.started", "thread_id":"thread-123"})));
        assert_eq!(t.thread_id.as_deref(), Some("thread-123"));
        event(&mut t, &json!({"type":"item.completed", "item":{"type":"agent_message", "text":"Finished"}}));
        assert_eq!(t.messages[0].text, "Finished");
        assert!(!event(&mut t, &json!({"type":"turn.failed", "error":{"message":"No access"}})));
        assert_eq!(t.messages[1].text, "No access");
    }
    #[test]
    fn recovery_does_not_replace_task_conversation_or_outcome() {
        let mut original = task("done", 0);
        original.thread_id = Some("original-thread".into());
        original.awaiting_merge = true;
        original.message("user", "Original request");
        original.sent_count = 1;
        let mut repair = task("now", 0);
        repair.attention = Some("unconfirmed".into());
        assert!(recovery_event(&mut original, &mut repair, &json!({"type":"thread.started", "thread_id":"repair-thread"})));
        for outcome in ["question", "completed"] {
            assert!(recovery_event(&mut original, &mut repair, &json!({"type":"item.completed", "item":{"type":"agent_message", "text":json!({"outcome":outcome,"message":"Recovery reply"}).to_string()}})));
            assert_eq!(repair.attention.as_deref(), if outcome == "completed" { None } else { Some(outcome) });
            assert_eq!(original.attention, None);
        }
        assert_eq!(original.thread_id.as_deref(), Some("original-thread"));
        assert_eq!(original.sent_count, 1);
        assert!(original.awaiting_merge);
        assert_eq!(original.messages[0].text, "Original request");
        assert!(original.messages[1..].iter().all(|m| m.role == "activity"));
        assert!(!recovery_event(&mut original, &mut repair, &json!({"type":"turn.failed", "error":{"message":"Failed"}})));
    }
    #[tokio::test]
    async fn recovery_only_runs_for_dirty_main_and_uses_hard_model() {
        let dir = std::env::current_dir().unwrap().join("tmp").join(format!("recovery-{}", id()));
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let project = Project { id: "p".into(), name: "Test".into(), path: repo.to_string_lossy().into() };
        git(&project.path, &["init", "-b", "main"]).await.unwrap();
        let mut settings = initial_data().settings;
        settings.hard_model.clear();
        settings.codex_path = dir.join("missing-codex.exe").to_string_lossy().into();
        let original = task("now", 0);
        let mut data = initial_data();
        data.tasks.push(original.clone());
        let app = Arc::new(Mutex::new(Store { data, file: dir.join("state.json") }));
        let (_sender, mut shutdown) = watch::channel(false);
        recover_main(&app, &original, &project, &settings, &mut shutdown).await.unwrap();
        std::fs::write(repo.join("work.txt"), "preserve this work").unwrap();
        assert_eq!(require_main(&project).await.unwrap_err(), DIRTY_MAIN);
        assert!(recover_main(&app, &original, &project, &settings, &mut shutdown).await.unwrap_err().contains("Select a hard-task model"));
        settings.hard_model = "hard-model".into();
        assert!(recover_main(&app, &original, &project, &settings, &mut shutdown).await.unwrap_err().contains("Could not start Codex"));
        assert_eq!(std::fs::read_to_string(repo.join("work.txt")).unwrap(), "preserve this work");
        git(&project.path, &["symbolic-ref", "HEAD", "refs/heads/other"]).await.unwrap();
        assert!(recover_main(&app, &original, &project, &settings, &mut shutdown).await.unwrap_err().contains("main branch"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn structured_outcomes_keep_questions_separate_from_completion() {
        let mut t = task("now", 0);
        for outcome in ["question", "blocked", "refused", "completed"] {
            let reply = json!({"outcome":outcome,"message":"Reply with your choice?"}).to_string();
            assert!(event(&mut t, &json!({"type":"item.completed","item":{"type":"agent_message","text":reply}})));
            assert_eq!(t.attention.as_deref(), if outcome == "completed" { None } else { Some(outcome) });
            assert_eq!(t.messages.last().unwrap().text, "Reply with your choice?");
        }
        t.attention = Some("unconfirmed".into());
        for text in ["Plain reply", r#"{"outcome":"unknown","message":"hello"}"#, r#"{"outcome":"completed","message":""}"#] {
            event(&mut t, &json!({"type":"item.completed","item":{"type":"agent_message","text":text}}));
            assert_eq!(t.attention.as_deref(), Some("unconfirmed"));
        }
        let mut old = serde_json::to_value(task("done", 0)).unwrap();
        old.as_object_mut().unwrap().remove("attention");
        assert!(serde_json::from_value::<Task>(old).unwrap().attention.is_none());
    }
}
