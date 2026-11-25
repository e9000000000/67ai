use std::sync::Mutex;
use serde_json::{Value, json};
use windows_sys::Win32::{Foundation::*, System::{JobObjects::*, Threading::*}};
use crate::{Task, Project, now};

static RUNS: Mutex<Vec<Run>> = Mutex::new(Vec::new());

struct Run {
    id: String, job: HANDLE, root: u32, task: String, project: String,
    title: String, cwd: String, agent: &'static str, started_at: u64,
}
unsafe impl Send for Run {}
impl Drop for Run { fn drop(&mut self) { unsafe { CloseHandle(self.job); } } }

pub fn attach(child: &tokio::process::Child, id: &str, task: &Task, project: &Project, recovery: bool) -> Result<(), String> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
        let run = Run { id: id.into(), job, root: child.id().unwrap(), task: task.id.clone(),
            project: task.project_id.clone(), title: task.title.clone(), cwd: project.path.clone(),
            agent: if recovery { "Merge recovery" } else if task.review_pending { "AI reviewer" } else { "Task agent" }, started_at: now() };
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32) == 0
            || AssignProcessToJobObject(job, child.raw_handle().unwrap() as HANDLE) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut runs = RUNS.lock().unwrap();
        runs.retain(|r| process_ids(r.job).map_or(true, |ids| !ids.is_empty()));
        runs.push(run);
        Ok(())
    }
}

pub fn remove(id: &str) { RUNS.lock().unwrap().retain(|r| r.id != id); }
pub fn remove_task(id: &str) { RUNS.lock().unwrap().retain(|r| r.task != id); }

fn process_ids(job: HANDLE) -> Result<Vec<u32>, String> {
    let mut buffer = vec![0usize; 64];
    loop {
        // The list starts after two DWORD counts, with pointer-sized process IDs.
        let ok = unsafe { QueryInformationJobObject(job, JobObjectBasicProcessIdList, buffer.as_mut_ptr() as *mut _,
            (buffer.len() * std::mem::size_of::<usize>()) as u32, std::ptr::null_mut()) };
        if ok != 0 {
            let list = unsafe { &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
            let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), list.NumberOfProcessIdsInList as usize) };
            return Ok(ids.iter().map(|id| *id as u32).collect());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) { return Err(error.to_string()); }
        buffer.resize(buffer.len() * 2, 0);
    }
}

fn program(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() { return None; }
        let mut path = vec![0u16; 32768];
        let mut size = path.len() as u32;
        let mut code = 0;
        let alive = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
        let ok = alive && QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut size) != 0;
        CloseHandle(handle);
        ok.then(|| String::from_utf16_lossy(&path[..size as usize]))
    }
}

pub fn list() -> Result<Vec<Value>, String> {
    let mut runs = RUNS.lock().unwrap();
    let mut items = vec![];
    let mut empty = vec![];
    for run in runs.iter() {
        let ids = process_ids(run.job)?;
        if ids.is_empty() { empty.push(run.id.clone()); }
        for pid in ids.into_iter().filter(|pid| *pid != run.root) {
            if let Some(path) = program(pid) {
                let name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
                if matches!(name.to_ascii_lowercase().as_str(), "conhost.exe" | "conhost" | "openconsole.exe") { continue; }
                items.push(json!({"id":format!("{}:{pid}", run.id), "pid":pid, "program":name, "path":path,
                    "task_id":run.task, "project_id":run.project, "task_title":run.title,
                    "cwd":run.cwd, "agent":run.agent, "started_at":run.started_at}));
            }
        }
    }
    runs.retain(|r| !empty.contains(&r.id));
    Ok(items)
}
