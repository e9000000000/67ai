# 67ai

A local Rust server and compact dark Kanban board for Codex. No Node build, hosted service, or API key is needed: it uses your installed Codex CLI and existing login.

## Run on Windows

Download `67ai.exe` from [GitHub Releases](https://github.com/e9000000000/67ai/releases), run it from a writable folder, and open http://127.0.0.1:3030. Install Codex CLI and log in first. The downloaded executable does not require Rust; its UI is embedded. Data is saved in the working directory's `data` folder unless `67AI_DATA_DIR` is set.

### Build from source

Install Rust and Codex CLI, then run `codex login` if needed. From this folder:

```powershell
.\start.ps1
```

Open http://127.0.0.1:3030. The launcher builds the release executable and starts it in the current terminal. Ctrl+C stops the server and its active runs. For development, use `cargo run`. The HTML, CSS and JavaScript are embedded in the executable, so rebuild after editing them.

After changes, run `.\restart.ps1` to stop this checkout's existing server, rebuild the release version, start it in the background, and open your default browser when it is ready. This interrupts any running task; interrupted tasks return to Future. The script respects `67AI_PORT` and `67AI_DATA_DIR`.

## GitHub releases

All development uses `main`, the repository's only branch, which tracks `origin/main`. Push with `git push`. The Windows release workflow runs on every push to `main` (not tag pushes), tests the code, builds the Windows x64 executable, and creates a GitHub release with `67ai.exe` attached. Each run gets a unique `windows-<run number>-<attempt>` tag targeting the pushed commit. Failed tests or builds prevent release creation. It uses GitHub's built-in token; no extra secret is needed.

## Workflow

- Add a project using an existing absolute folder path.
- **Future:** parked indefinitely. Messages are saved without starting Codex.
- **Todo:** automatically enters the FIFO queue. By default, only one task runs at a time across all projects.
- **Now:** the worker owns this state. Open the task to read progress, commands, file changes, and replies. Stop moves it back to Future.
- **Done:** Codex ended its run. Questions, refusals, blockers, and failed runs appear as red cards with an attention label; they are unfinished. The browser tab reads **QUESTION** while any Done task needs attention, across all projects. Open the card and use **Send answer & continue** (or Ctrl+Enter) to reply and queue the same conversation. Unfinished tasks cannot be approved for cleanup. A follow-up message moves it to Todo and joins the queue for its project in the same Codex thread. **Approve** starts a 30-minute cleanup countdown. **Revive** cancels it and keeps the task in Done; sending feedback or changing its state also cancels cleanup. When the timer expires, 67ai deletes the Codex session, board task/conversation, and attachments not used by other tasks. Project files stay. The countdown survives restarts and runs independently of queue pause; overdue cleanup runs on startup. Session deletion requires a Codex version supporting `thread/delete`. If deletion fails, the task stays with an error; approve again to retry.

Drag cards between Future, Todo and Done. Messages sent during a run wait for a follow-up run; they do not interrupt the current turn. Pause queue prevents new runs while allowing active runs to finish. Failed tasks remain red in Done with an error and require a message or Retry before they run again. Elapsed time is cumulative across runs and excludes queue time.

Paste images or files with Ctrl+V into task instructions or a follow-up message, drag files onto either editor, or use **Attach files** (25 MB per file). Image previews and file chips can be removed before sending. Attachments persist under the data directory and appear in the conversation for download. PNG, JPEG, WebP and GIF images are passed to Codex as image inputs; other files are provided by their saved local paths. Clipboard file support depends on the browser and source application; drag-and-drop and the file picker are also available.

Use Hard or Easy to create a task with the corresponding model from Settings. Ctrl+Enter creates an Easy task or sends a follow-up message. The task header menu contains model selection, AGENTS.md, title editing, deletion, and Stop. Approve appears beside the menu for completed tasks.

Set the Hard task model and Easy task model in Settings. Both initially use your existing default model. The chosen model is saved on each new task; you can change it in the task menu. Model suggestions are read from Codex's local `models_cache.json`; enter additional model IDs in Settings. Model availability depends on your account. Settings accepts the native Codex executable path; Windows npm installations are detected automatically.

Settings also includes **Maximum task workers** (default **4**, minimum **1**) and **Parallelism allowed** (default **off**, including existing installations). When enabled, tasks from the same or different projects can run concurrently in separate Git worktrees under `data/worktrees`, each on its own `67ai/<task-id>` branch based on `main`. Project folders must be Git repository roots with a clean `main` branch checked out and a configured Git commit identity. Uncommitted main changes are never included in task worktrees.

One additional merge worker processes completed tasks sequentially. It commits any remaining task changes and merges the task branch into `main`. Tasks remain **orange** in Done until merged, including while a merge is in progress or blocked. Conflicts are aborted and reported on the card; resolve the task branch against main and click **Retry merge**. Pending merges survive restarts. Pause stops new tasks and merges; disabling parallelism makes subsequent task runs sequential but still drains completed merges. Lowering the limit does not stop already running tasks. Pending merges cannot be approved, deleted, moved, or given follow-up messages. Task worktrees and branches are retained for inspection and follow-ups, including after removing the task from the board; Git worktree cleanup is manual.

In Settings, select a project and choose **Edit or create AGENTS.md** to edit the file in that project's root, even before creating any tasks. The editor is also accessible inside every task. If the file is missing, write your own instructions or load a shared template and click **Create AGENTS.md**. Other ancestor/global instruction files are displayed read-only, including a root AGENTS.override.md when present. Nested subfolders can have additional instructions. Editing is blocked while this project has a running task, and saving detects external edits. Templates only populate the editor when creating a missing file; they are not injected into task prompts. 67ai sends the task title and pending user messages to Codex. Worktree runs also instruct Codex to stay on the task branch and leave merging to 67ai. A structured final-response schema records whether the outcome is completed, a question, blocked, or refused. Only the reply text is shown in the conversation; missing or invalid outcomes remain flagged as completion not confirmed. Existing saved tasks retain their state until their next run.

Use **Settings → Manage templates** to create, rename, edit, duplicate, or delete shared AGENTS.md templates, including the initial templates. Templates are available across projects and persist across restarts. Changing or deleting one does not modify existing project files.

## Local data and execution

Projects, tasks, conversations, templates and settings are stored in `data/67ai.json`, using a temporary file and atomic replacement. Existing installations migrate their previous data file automatically on startup; stop the old server first. Environment overrides now use the `67AI_` prefix. Codex keeps its own thread history in its normal home directory. A file lock prevents two servers from opening the same data directory. Interrupted tasks return to Future on restart rather than silently rerunning; elapsed time is checkpointed every five seconds.

The server binds only to `127.0.0.1` and rejects foreign Host and Origin headers. New and resumed Codex runs use `--dangerously-bypass-approvals-and-sandbox`: no Codex sandbox or approval prompts, with access allowed by the Windows account and any enforced administrator policies. Your configured agent features still apply. On Windows, each run is contained in a kill-on-close job so stopping or exiting the server also terminates child processes. Concurrency is controlled by the parallelism settings.

Environment overrides:

| Variable | Default |
| --- | --- |
| `67AI_PORT` | `3030` |
| `67AI_DATA_DIR` | The working directory's `data` directory |
| `67AI_CODEX` | Detected native executable; used when creating initial settings |

Run checks with `cargo test`, `node --check web/app.js`, and `cargo build; node tests/integration.mjs; node tests/parallelism.mjs`. The integration suite uses an isolated data directory and a deterministic fake Codex executable; it does not call a model. This checkout pins the installed MSVC Rust toolchain on Windows.
