# Gen tasks implementation plan

Add Gen beside Hard and Easy. A Gen task researches the selected project and creates actionable tasks in Future. It never implements its findings. It uses its own model setting and the same full Codex permissions as other tasks, including tools for investigation and reverse engineering.

This is a plan only. The implementation steps below are proposed changes.

## User flow

Put these actions first in the application burger menu, in this order, followed by the existing New task, Add project, and Settings actions:

| Action | Research scope |
| --- | --- |
| Find issues | Find concrete bugs, broken behavior, and other project problems. Create one task per distinct issue with evidence and a way to verify the eventual fix. |
| Gen more content | Identify extensions of existing content and systems: more levels using existing mechanics, continuation of an existing story, additional entries in existing content collections. Preserve the project's behavior and direction. |
| Gen new features | Propose useful new capabilities grounded in the project's current purpose, code, and constraints. Describe the intended behavior and acceptance criteria. |

Do not add a Find optimisations menu item. The later request for a distinct Gen type supersedes the initial suggestion to run these actions as Hard tasks: all three use Gen and its configured model.

Each preset opens a small form showing the project, the research instructions, optional extra direction, and a **Start Gen** button. Preselect the active project. On the All projects view require a project selection; never run against every project implicitly. With no projects, use the existing add-project flow. Submitting queues one Gen task in Todo; the normal queue starts it when a worker is available. The research task ends in Done and its proposals appear in Future.

Add a **Gen** submit button to the ordinary New task form for custom research requests. Preserve its current Future/Todo destination and the existing Ctrl+Enter shortcut for Easy. Show a distinct Gen badge on cards and in the task header. Add **Gen task model** to Settings using the current model picker and custom-ID support. Keep model changes independent of the saved task type.

## Research instructions

Append the Gen instructions on every run, including resumed conversations. Follow-ups can refine the research but cannot turn a Gen task into an implementation task; requests to implement should become Future proposals.

The instructions must require Codex to:

- Inspect the actual project and its instructions before proposing work. Use code, assets, tests, runtime inspection, documentation, and reverse engineering where relevant and available.
- Research and propose only. Do not fix bugs, generate production content, implement features, change project configuration, commit, switch branches, merge, deploy, or publish. Do not call the board API or edit its data file; return proposals in the response schema.
- Permit temporary research helpers and extracted inspection material under `tmp`, following project constraints. Clean up only helpers and processes created by this run. Preserve existing files, user data, and sessions.
- Examine existing project tasks to avoid proposing work already tracked or completed. Include all statuses in this context and exclude the current research task. Recheck current tasks when saving results to catch exact duplicates added during research.
- Ground issues in evidence. Distinguish observed facts from proposed behavior and unverified hypotheses. If relevant information cannot be obtained, describe the limitation rather than invent findings.
- Return independently actionable tasks with a clear title, scope, relevant files or assets, evidence or rationale, and acceptance criteria. Split distinct issues; keep tightly coupled work together. Do not create filler to reach a task count.

For content research, a new mechanic, system, or capability belongs under new features. Content proposals must name the existing format or system they extend. For example, adding levels that use current mechanics fits; introducing a level editor is a new feature. Returning no proposals is valid when nothing suitable is found.

Full permissions remain enabled with `--dangerously-bypass-approvals-and-sandbox`. The restriction on implementation is an instruction to the agent, not a filesystem sandbox guarantee. The server must also avoid its own automatic implementation operations for Gen.

## Backend changes

Use the existing structures in `src/main.rs`; avoid a separate scheduler or task framework.

1. Extend `Difficulty` with `Gen` serialized as `gen`. Keep the current field name for compatibility. Add `gen_model` to `Settings`, its action payload, and initial settings. For older saved data, default it to an empty string, meaning the Codex configuration model. For new installations initialize it from the same configured model as Hard and Easy. Save a model snapshot on each newly created Gen task.
2. Reuse `CreateTask` for both custom Gen tasks and presets. Presets supply ordinary title/instructions, `difficulty: gen`, and the Gen model. No separate persisted preset enum is needed: the saved request defines the scope and remains in the conversation.
3. Add `src/gen-schema.json`, embed and write it beside the existing schemas at startup, and select it for Gen execution. Keep the usual `outcome` and `message`, adding `tasks`, an array of objects with required nonempty `title` and `text` fields. Keep the four existing outcomes. Require an empty task array for question, blocked, and refused outcomes.
4. Add a small Gen result parser alongside `event` and `review_event`. Continue streaming research activity into the conversation, but stage proposals in memory rather than inserting cards as individual messages arrive. An invalid response must remain unconfirmed or failed; ordinary commentary cannot authorize task creation.
5. After successful process exit and `turn.completed`, validate the final response and persist proposals together with the source run's final state and consumed-message position through one `Store::commit`. Only a completed outcome creates tasks. A stopped, failed, limit-interrupted, invalid, or incomplete run creates none. Keep this result handling separate from the current single-task `update_task` closure because it needs to append tasks to `Data`.
6. Create each proposal as an ordinary **Hard** task in **Future**, using the current Hard model setting at creation. This is the proposed default for later implementation; the proposal itself is never Gen. Assign project ID and status on the server, without accepting them from model output. Initialize the normal task lifecycle fields. Include a source Gen task reference in its initial conversation and record created IDs/titles in the source conversation. These references must not cascade deletion: approving or deleting the research task leaves its Future tasks intact.
7. Provide existing task titles and instructions in the research prompt. Before insertion skip exact normalized title matches in the same project and duplicates within the returned batch; record skipped proposals in the summary. Let Codex handle semantic overlap from task context. Do not introduce embeddings or a deduplication service.

Persisting task insertion and source completion together prevents a restart from replaying a saved result into duplicate cards. A follow-up sent during research retains the existing queued follow-up behavior. The next run receives refreshed task context, including proposals already saved by the prior run.

## Execution and lifecycle

Gen runs use the selected project's existing folder for investigation and consume a normal task-worker slot. Bypass `prepare_worktree`, `recover_main`, and all merge setup for Gen. Research does not need a clean repository, a new branch, or a commit identity. Preserve any pre-existing local changes.

Adapt `next_task` so a Gen run and another run in the same project cannot overlap in either start order. Runs in other projects may continue when parallelism allows it. Keep the merge worker's existing exclusion for active runs without a task worktree, and verify that a merge already in progress prevents Gen from starting. This gives research a stable view of the project during app-controlled work without holding the global Git lock for the entire investigation.

Never set `awaiting_merge` for Gen, and never route Gen through automatic commit or merge recovery. Keep pause, Stop, elapsed time, usage-limit retry, persisted conversations, attachments, and questions using the existing lifecycle. Reapply the research-only instructions after every resume.

Do not apply the Easy or Hard automatic AI approval toggles to Gen. Keep normal manual approval and cleanup. If manual AI approve is used on a Gen task, the reviewer should assess the research and generated tasks against the requested scope, not require implemented changes. Give that reviewer the generated task records. A rejection returns the original Gen agent to research under the same restrictions; it must not start implementation or duplicate existing proposals.

## Implementation order

1. Add the type, setting, migration defaults, and Gen response schema.
2. Implement research prompting, task context, staged results, and atomic Future task creation.
3. Add the Gen execution path, scheduler exclusions, merge exclusions, and review context.
4. Add the menu presets, custom Gen creation, badge, model setting, and accurate permissions help text in `web/index.html`, `web/app.js`, and `web/style.css`. The current Settings help incorrectly describes workspace-write permissions; align it with actual full-permission execution.
5. Extend the existing tests and fake Codex, then update the workflow documentation in `README.md`.

## Acceptance checks

- Old data loads without changing existing Hard/Easy tasks. Gen type and model persist across restarts. Shared model IDs never make an old untyped task appear as Gen; show Gen only for explicitly saved Gen tasks and preserve unambiguous legacy Hard/Easy badge inference.
- All three presets select exactly one project, queue Gen with its configured model, and preserve their distinct scopes. Custom Gen creation works from Future and Todo; Easy keyboard behavior stays unchanged.
- The fake Codex verifies the full-permission flag, Gen schema, research instructions on first and resumed runs, and existing-task context. A completed response with two valid proposals produces exactly two Hard Future cards in the same project and a summary in the Gen conversation.
- No-proposal completion works. Invalid output, unsuccessful exit after structured output, question, blocked, refused, Stop, shutdown, and usage limits create no proposals. Retry and restart do not duplicate persisted results. Follow-ups can create additional distinct proposals without implementing them.
- Gen does not create a worktree, trigger dirty-main recovery, commit, or enter the merge queue with parallelism on or off. Same-project tasks and merges respect both start orders; different projects still use available worker slots.
- Generated cards survive source approval/cleanup. Gen never inherits Hard/Easy automatic approval settings, and a manual reviewer rejection resumes research with existing proposals in context.
- Run `cargo test`, `node --check web/app.js`, the updated difficulty checks, and integration suites covering Gen, parallelism, and AI approval. Use isolated data under `tmp` and the deterministic fake Codex; do not spend live model calls for these checks. Inspect the new menu, forms, badges, Settings, and resulting Future cards in a headless browser without interrupting the user's running server.
