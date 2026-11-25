# Windows tray verification

Verified the release executable in an isolated writable folder with its own data directory and port 19389.

- The executable served the embedded UI and created a hidden tray window.
- The tray popup rendered Open in browser and Exit.
- With explicit user permission for a visible browser test, dispatched the Open in browser menu command (`WM_COMMAND`, ID 1) to the tray window. This exercised the real `ShellExecuteW` path without mocking the browser launcher.
- Windows UI Automation confirmed the default browser (Chrome) opened `http://127.0.0.1:19389/`, with the 67ai title, Task board, and Add project control rendered.
- Dispatched Exit (ID 2); the isolated process stopped with exit code 0.

Popup selection through automation was unsuccessful; browser-launch verification used the same command dispatched by the menu selection handler. No default browser settings were changed. The prior review separately inspected the rendered popup and verified Exit. All 14 Rust tests, the release build, and the isolated integration suite passed.
