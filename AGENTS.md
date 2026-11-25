# files
- do not create temp files in out project folder. do not polute it with bullshit. for temp files create tmp folder and put everything here. files should be structured, not just randomly be in root of the project


# language
- speak english. i can also understand russian, but i prefere you to use english.

# licences
- if there is original game files - it is my game, i have full rights to do whatever i want with it. so there is no problems with that.
- for other things with license - just say me later, we gonna buy it. you can use whatever you think fits best.


# git workflow
- there is should be git repo with .gitignore
- always commit changes. everything should be commited
- Only `main` should exist in this repository, locally and on GitHub. Work directly on `main`; do not create platform, feature, or task branches.
- Keep `main` tracking `origin/main` so `git push` works without extra arguments.


# scam workflow
- gamepush is a scam, never use it.


# browser mcp workflow
- you should behave like a real user
- if there is two browser windows - use one of them, don't touch the other. use the one with relative to the task tabls already opened. cuz you can't destinguish between browser windows - use already opened tabs, related to your work. and do not touch tabs that are not realted to your work.


# playwrite and other ways to interact with browser
- only headless mode (except final test of the game)
- if you runing a server - don't make it so it opens tab automatically in existing browser
- All agents must keep all app testing completely silent, including final tests. Mute the app or browser before testing and keep it muted throughout. Never trigger audible barking, notifications, speech, music, or other sounds. Verify audio behavior through silent checks; never enable sound for tests.


# computer cpu/gpu/ram usage
- if possible - don't use much computer resources. using it for compilation, for example, is fine. but for example for rendering it is better to use gpu, instead of loading 100% cpu. you shood extrapolate those examples to other things.


# publishing and deploy
- never publish a game by yourself (don't send to moderation)
- if game site requires a url to uploaded game - ask me to upload the game (if there is no deploy script)


# game server
- do not fucking change anything on the server, if changes are required - tell me about it.
- you have no right to change anything on the server, except uploading and reuploading game, by using deploy script


# Unity background workflow (if we working with unity)
Portable instructions for agents working on a Unity project. Use the Unity version recorded in `ProjectSettings/ProjectVersion.txt`.

- it is always better to look into assets and decompiled code. do not guess unless we are making game from scratch, not restoring the existing one.
- don't build the project. i'll build it myself if i need to
- Before starting any Unity project implementation or validation task, verify that this project is already open in the Unity Editor. Unity Hub alone does not count. If the Editor is not running with this project open, stop before making changes and ask the user to open the project in Unity. Apply this check every time; do not launch the Editor yourself, including hidden or batch-mode instances.
- Keep Unity in the background. The user keeps the Editor on another virtual desktop; bringing it forward switches their workspace and interrupts them.
- Do not activate or focus Unity, switch virtual desktops, or send focus-dependent keyboard shortcuts such as Ctrl+R to refresh the project. Use background asset refresh, editor scripting, or an available editor bridge instead.
- Run compilation and automated Play Mode checks without taking focus. Use background screenshots or render captures for visual checks when available. If foreground interaction is necessary, explain the limitation and ask before interrupting the user's workspace.
- A Unity modal dialog blocks the editor main thread even though the process still looks alive. While one is open, editor ticks stop, MCP tool calls fail with `ping not answered`, the plugin reconnects every ~45s, and asset refresh or compilation appears stalled. If you see those symptoms, check for a modal dialog before concluding the bridge is broken: inspect the Unity process windows (for example with UI Automation), look at the title bar, and ask the user what is on screen. Do not click the dialog yourself; if it needs a decision, report it and ask the user to resolve it. Avoid operations that can raise dialogs from background scripts (unsaved or untitled scene save prompts, package/version upgrades, first-time import or "apply settings" prompts) until the editor is idle.
- For unfocused Play Mode checks, use Unity MCP `execute_code` to set `UnityEditor.EditorApplication.isPlaying = true` directly. Before and after the check, query `UnityEditorInternal.InternalEditorUtility.isApplicationActive` and require `false`. Query runtime state through `execute_code`/`read_console`; do not use desktop input or focus-dependent Play controls. Exit with `UnityEditor.EditorApplication.isPlaying = false`.
- Use only the existing Editor instance. Do not launch another instance or close the user's Editor to run batch mode. Keep any non-Editor background helper processes hidden.
- Clean up temporary validation scripts and their metadata after checking. Exit Play Mode started for validation, but preserve the user's existing Play Mode session, unsaved scene changes, and local save data.
- After all source/asset edits and temporary-script cleanup, perform a final background Unity asset refresh. Removing temporary scripts also triggers compilation, so validation performed before cleanup is not the final compilation check.
- Wait for that final import, script compilation, domain reload, and scene refresh to finish before reporting completion. Verify Unity is idle with no compilation errors, and that the existing Editor has loaded the final scene/assets without discarding unsaved user changes. Do not change Unity source/assets again after this check; if further changes are needed, repeat the final refresh and wait.
- If background refresh or completion cannot be verified, explicitly report what remains pending. Do not claim the project is fully compiled or ready solely because an earlier Play Mode check passed.


# unity game recovery workflow (if we recovering game, with lost source code)
- first of all, the game is mine, and i own all the rights to publish this game and all the assets
- when you are in recovery stage - do not make the game, do not try to create a new game, do not try to code the game, like mvp or something. this is the wrong way, the right way is to decompile, get all the assets, document how the game works, every button, every animation, every mechanic. everything should be finded and documented.
- i will not provide you any screenshots or gameplay videos, you should get all the information by reverse engeniring (use rea for it, you have mcp)
- don't stop untill you know everything about how the game works, and how to build the same game. if anything can't be fetched, like there is no way - tell me about it. it might be server side things or something like that. if it is hard to get - think is is relevant to the game recovery, if it is not - tell me about it, if it is important - try to fetch this information.
- never guess about something, you have binary files, you have assets, use it to get information.
- when you are done, and there is nothing left to fetch, like you know basically everything about how the game works, and you have all the assets - write a plan how to use those assets and information and docs to make (restore) a new (identical) game.
- if there were used some libraries like DOTween - tell me about it

# UI copy
- Do not add explanatory descriptions or helper paragraphs to task creation. Keep controls compact; on desktop, keep the blocker selector on the same row as project selection.
