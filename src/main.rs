use axum::{Json, Router, extract::{Path, State}, http::{HeaderMap, StatusCode}, response::{Html, IntoResponse}, routing::{get, post}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::{SystemTime, UNIX_EPOCH}, process::Stdio};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::Command, sync::{Mutex, watch}};

type App = Arc<Mutex<Store>>;
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
struct Task {
    id: String, project_id: String, title: String, model: String, status: String,
    messages: Vec<Message>, created_at: u64, queued_at: u64, elapsed_ms: u64,
    started_at: Option<u64>, thread_id: Option<String>, error: Option<String>,
    sent_count: usize, cancel: bool,
    #[serde(default)] cleanup_at: Option<u64>,
}
impl Task {
    fn message(&mut self, role: &str, text: impl Into<String>) { self.messages.push(Message { role: role.into(), text: text.into(), at: now(), attachments: vec![] }); }
}
#[derive(Clone, Serialize, Deserialize)]
struct Template { id: String, name: String, content: String }
#[derive(Clone, Serialize, Deserialize)]
struct Settings { default_model: String, codex_path: String, paused: bool }
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
    if let Ok(p) = std::env::var("TASKDESK_CODEX") { return p; }
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
        settings: Settings { default_model: model, codex_path: detect_codex(), paused: false },
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
    CreateTask { project_id: String, title: String, text: String, model: String, status: String, #[serde(default)] attachments: Vec<String> },
    EditTask { id: String, title: String, model: String },
    MoveTask { id: String, status: String },
    SendMessage { id: String, text: String, #[serde(default)] attachments: Vec<String> },
    StopTask { id: String },
    DeleteTask { id: String },
    ApproveTask { id: String },
    ReviveTask { id: String },
    Settings { default_model: String, codex_path: String, paused: bool },
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
    if t.status == "now" { Err(bad("Stop the running task first")) } else { Ok(()) }
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
        Action::CreateTask { project_id, title, text, model, status, attachments } => {
            let attachments = load_attachments(&store, attachments)?;
            nonempty(&title, "Title")?; if attachments.is_empty() { nonempty(&text, "Task instructions")?; } valid_status(&status)?;
            if status == "done" { return Err(bad("New tasks must start in future or todo")); }
            if !d.projects.iter().any(|p| p.id == project_id) { return Err(bad("Project not found")); }
            let mut task = Task { id: id(), project_id, title: title.trim().into(), model, status, messages: vec![], created_at: now(), queued_at: now(), elapsed_ms: 0, started_at: None, thread_id: None, error: None, sent_count: 0, cancel: false, cleanup_at: None };
            task.message("user", text.trim()); task.messages.last_mut().unwrap().attachments = attachments;
            result = json!({"id": task.id}); d.tasks.push(task);
        }
        Action::EditTask { id, title, model } => {
            nonempty(&title, "Title")?; let t = task_mut(&mut d, &id)?; idle(t)?;
            t.title = title.trim().into(); t.model = model;
        }
        Action::MoveTask { id, status } => {
            valid_status(&status)?; let t = task_mut(&mut d, &id)?; idle(t)?;
            t.cleanup_at = None;
            if status == "todo" {
                if t.sent_count >= t.messages.len() || !t.messages[t.sent_count..].iter().any(|m| m.role == "user") {
                    t.message("user", "Continue this task. Check the current work and complete any remaining steps, then report the result.");
                }
                t.queued_at = now(); t.error = None; t.cancel = false;
            }
            t.status = status;
        }
        Action::SendMessage { id, text, attachments } => {
            let attachments = load_attachments(&store, attachments)?;
            if attachments.is_empty() { nonempty(&text, "Message")?; } let t = task_mut(&mut d, &id)?;
            t.message("user", text.trim());
            t.messages.last_mut().unwrap().attachments = attachments;
            t.cleanup_at = None;
            if t.status == "done" { t.status = "todo".into(); t.queued_at = now(); }
            t.error = None;
        }
        Action::StopTask { id } => {
            let t = task_mut(&mut d, &id)?;
            if t.status != "now" { return Err(bad("Task is not running")); } t.cancel = true;
        }
        Action::DeleteTask { id } => { idle(task_mut(&mut d, &id)?)?; d.tasks.retain(|t| t.id != id); }
        Action::ApproveTask { id } => {
            let t = task_mut(&mut d, &id)?;
            if t.status != "done" { return Err(bad("Only done tasks can be approved")); }
            if t.cleanup_at.is_none() { t.cleanup_at = Some(now() + 30 * 60 * 1000); }
            t.error = None;
        }
        Action::ReviveTask { id } => {
            let t = task_mut(&mut d, &id)?; idle(t)?;
            t.cleanup_at = None; t.error = None;
        }
        Action::Settings { default_model, codex_path, paused } => {
            nonempty(&codex_path, "Codex executable")?;
            d.settings = Settings { default_model: default_model.trim().into(), codex_path: codex_path.trim().into(), paused };
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
            if d.tasks.iter().any(|t| t.project_id == project_id && t.status == "now") { return Err(bad("Wait for this project's running task to finish before editing instructions")); }
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
                "agent_message" => if let Some(text) = item["text"].as_str() { task.message("assistant", text); },
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

async fn run_codex(app: &App, task: &Task, project: &Project, settings: &Settings, shutdown: &mut watch::Receiver<bool>) -> Result<(), String> {
    let mut images = vec![];
    let store = app.lock().await;
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
    let prompt = format!("Task: {}\n\n{}", task.title, prompt);
    let mut command = Command::new(&settings.codex_path);
    command.arg("exec");
    if let Some(thread) = &task.thread_id { command.args(["resume", thread]); }
    command.args(["--dangerously-bypass-approvals-and-sandbox", "--json", "--skip-git-repo-check"]);
    let model = if task.model.is_empty() { &settings.default_model } else { &task.model };
    if !model.is_empty() { command.args(["--model", model]); }
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
                    update_task(app, &task.id, |t| { if !event(t, &value) { failed = true; } }).await?;
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
    Ok(())
}

fn next_task(d: &Data) -> Option<usize> {
    if d.settings.paused || d.tasks.iter().any(|t| t.status == "now") { return None; }
    d.tasks.iter().enumerate().filter(|(_, t)| t.status == "todo" && t.error.is_none()).min_by_key(|(_, t)| t.queued_at).map(|(i, _)| i)
}
async fn worker(app: App, mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() { break; }
        let job = {
            let mut store = app.lock().await;
            let mut d = store.data.clone();
            if let Some(index) = next_task(&d) {
                let t = &mut d.tasks[index]; t.status = "now".into(); t.started_at = Some(now()); t.cancel = false;
                t.message("system", "Codex started working.");
                let task = t.clone();
                let project = d.projects.iter().find(|p| p.id == task.project_id).unwrap().clone();
                let settings = d.settings.clone();
                if let Err(e) = store.commit(d) { eprintln!("Cannot persist queue: {e}"); None } else { Some((task, project, settings)) }
            } else { None }
        };
        if let Some((task, project, settings)) = job {
            let result = run_codex(&app, &task, &project, &settings, &mut shutdown).await;
            let stopped = *shutdown.borrow();
            if let Err(e) = update_task(&app, &task.id, |t| {
                t.elapsed_ms += now().saturating_sub(t.started_at.unwrap_or(now())); t.started_at = None;
                match result {
                    Ok(()) => {
                        t.sent_count = task.messages.len();
                        let pending = t.messages[t.sent_count..].iter().any(|m| m.role == "user");
                        t.status = if pending { "todo" } else { "done" }.into();
                        t.queued_at = now(); t.error = None;
                        t.message("system", if pending { "Run completed. Your follow-up is queued." } else { "Run completed." });
                    }
                    Err(e) => { t.status = if t.cancel || stopped { "future" } else { "todo" }.into(); t.error = Some(e.clone()); t.message("system", e); }
                }
                t.cancel = false;
            }).await { eprintln!("Cannot persist result; worker stopped: {e}"); break; }
        } else {
            tokio::select! { _ = shutdown.changed() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {} }
        }
    }
}

async fn delete_session(executable: &str, thread: &str) -> Result<(), String> {
    let mut command = Command::new(executable);
    command.args(["app-server", "--stdio"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        for request in [
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"taskdesk","version":"0.1.0"},"capabilities":null}}),
            json!({"id":2,"method":"thread/delete","params":{"threadId":thread}}),
        ] {
            input.write_all(format!("{request}\n").as_bytes()).await.map_err(|e| e.to_string())?;
            loop {
                let line = output.next_line().await.map_err(|e| e.to_string())?.ok_or("Codex closed before confirming session deletion")?;
                let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                if value["id"] != request["id"] { continue; }
                if let Some(error) = value.get("error") { return Err(error.to_string()); }
                if value.get("result").is_none() { return Err("Invalid Codex response".into()); }
                break;
            }
            if request["id"] == 1 {
                input.write_all(b"{\"method\":\"initialized\"}\n").await.map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }).await.unwrap_or_else(|_| Err("Codex session deletion timed out".into()));
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
    let port = std::env::var("TASKDESK_PORT").unwrap_or_else(|_| "3030".into()).parse::<u16>()?;
    // Bind before loading state: a second launch must not recover another server's running tasks.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let dir = std::env::var_os("TASKDESK_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"));
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("taskdesk.lock"))?;
    lock.try_lock().map_err(|_| "This Taskdesk data directory is already open in another server")?;
    let file = dir.join("taskdesk.json");
    let mut data: Data = if file.exists() { serde_json::from_slice(&std::fs::read(&file)?)? } else { initial_data() };
    for t in &mut data.tasks {
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
    let worker = tokio::spawn(worker(app.clone(), shutdown));
    let router = Router::new()
        .route("/", get(|| async { Html(include_str!("../web/index.html")) }))
        .route("/app.js", get(|| async { ([("content-type", "text/javascript")], include_str!("../web/app.js")) }))
        .route("/style.css", get(|| async { ([("content-type", "text/css")], include_str!("../web/style.css")) }))
        .route("/api/state", get(state)).route("/api/action", post(action)).route("/api/agents/{id}", get(agents))
        .route("/api/attachments", post(upload).layer(axum::extract::DefaultBodyLimit::max(25 * 1024 * 1024)))
        .route("/api/attachments/{id}", get(attachment))
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(axum::middleware::from_fn(local_only)).with_state(app);
    println!("Taskdesk running at http://127.0.0.1:{port}");
    axum::serve(listener, router).with_graceful_shutdown(async move { let _ = tokio::signal::ctrl_c().await; let _ = stop.send(true); }).await?;
    let _ = worker.await;
    let _ = cleanup.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn approval_revive_feedback_restart_and_expiry() {
        let dir = std::env::current_dir().unwrap().join("test-results").join(format!("approval-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = initial_data();
        d.settings.paused = true;
        d.settings.codex_path = dir.join("missing-codex.exe").to_string_lossy().into();
        let t = task("done", 0); let key = t.id.clone(); d.tasks.push(t);
        let app = Arc::new(Mutex::new(Store { data: d, file: dir.join("taskdesk.json") }));
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
        let app = Arc::new(Mutex::new(Store { data, file: dir.join("taskdesk.json") }));
        let Json(uploaded) = upload(State(app.clone()), axum::extract::Query(Upload { name: "notes.txt".into(), media_type: "text/plain".into() }), axum::body::Bytes::from_static(b"file contents")).await.unwrap();
        let key = uploaded["id"].as_str().unwrap().to_string();
        let Json(created) = action(State(app.clone()), Json(Action::CreateTask { project_id: "p".into(), title: "Files".into(), text: "".into(), model: "".into(), status: "future".into(), attachments: vec![key.clone()] })).await.unwrap();
        let task_id = created["id"].as_str().unwrap().to_string();
        let _ = action(State(app.clone()), Json(Action::SendMessage { id: task_id.clone(), text: "".into(), attachments: vec![key.clone()] })).await.unwrap();
        assert!(action(State(app.clone()), Json(Action::SendMessage { id: task_id.clone(), text: "".into(), attachments: vec![] })).await.is_err());
        assert!(action(State(app.clone()), Json(Action::SendMessage { id: task_id, text: "bad".into(), attachments: vec!["../taskdesk.json".into()] })).await.is_err());
        let saved: Data = serde_json::from_slice(&std::fs::read(dir.join("taskdesk.json")).unwrap()).unwrap();
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
        Task { id: id(), project_id: "p".into(), title: "Test".into(), model: "".into(), status: status.into(), messages: vec![], created_at: 0, queued_at, elapsed_ms: 0, started_at: None, thread_id: None, error: None, sent_count: 0, cancel: false, cleanup_at: None }
    }
    #[test]
    fn queue_is_global_fifo_and_never_runs_future_or_parallel() {
        let mut d = initial_data(); d.tasks = vec![task("future", 0), task("todo", 20), task("todo", 10)];
        assert_eq!(next_task(&d), Some(2));
        d.tasks[2].status = "now".into(); assert_eq!(next_task(&d), None);
        d.tasks[2].status = "done".into(); assert_eq!(next_task(&d), Some(1));
        d.settings.paused = true; assert_eq!(next_task(&d), None);
        d.settings.paused = false; d.tasks[1].error = Some("failed".into()); assert_eq!(next_task(&d), None);
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
}
