# 01 — Lifecycle skeleton

**What to build:** the app runs as a single portable exe with no service and no installer. Launching it (elevated) once self-installs a logon scheduled task so that at every subsequent logon the app starts silently, already elevated, with no UAC prompt and no console window. Only one copy can run at a time. The tray icon appears with a Quit item; quitting leaves the current profile and plan in place. `--uninstall` removes the scheduled task, leaving the power plans and hardware state untouched.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [x] Running the exe twice keeps exactly one instance alive (named-mutex single instance). *(implemented `Local\NitroTray` mutex in src/main.rs; runtime behavior needs on-device check)*
- [x] First elevated run installs the `NitroTray` scheduled task (logon trigger, highest privileges) that launches the exe; manual launch still shows a standard UAC prompt. *(implemented in src/task.rs via in-process ITaskService COM; needs on-device check)*
- [ ] At logon the app starts silently elevated with no UAC prompt and no console window. *(needs on-device verification)*
- [x] `--uninstall` removes the scheduled task so the app can be cleanly removed. *(implemented; needs on-device check)*
- [x] `--log` writes a debug log beside the exe with useful diagnostics. *(implemented + unit tested)*
- [ ] Tray icon shows with a Quit menu item; quitting leaves the current profile and active plan in place. *(Quit event wiring in src/main.rs; tray icon itself is ticket 08 — not yet implemented)*
- [x] The app has no service, no installer, and no runtime dependencies (single portable exe). *(by construction; manifest requires admin, `windows_subsystem = "windows"`)*

## Comments

2026-08-07: Implemented in src/main.rs, src/task.rs, src/log.rs.
- main.rs: `--log`/`--uninstall` args, `Local\NitroTray` named-mutex single instance (handle kept for process lifetime), exe-dir resolution via GetModuleFileNameW, idempotent logon-task install on every run (create-or-update), flat `handle_event` dispatch (Quit implemented; SelectProfile/ToggleSmartCharge/PowerChanged/Resume arms stubbed for tickets 10/12), `windows_subsystem = "windows"`.
- task.rs: in-process ITaskService COM (winapi taskschd bindings — windows-sys has no TaskScheduler metadata): logon trigger, TASK_LOGON_INTERACTIVE_TOKEN, TASK_RUNLEVEL_HIGHEST, start-when-available, no battery disallow/stop, no execution-time-limit ("PT0S"), RegisterTaskDefinition create-or-update; DeleteTask with ERROR_FILE_NOT_FOUND => Ok. RAII COM init/ptr/BSTR guards.
- log.rs: `nitro-tray.log` beside the exe, UTC timestamps (hand-rolled civil-from-days, no chrono), mutex-serialized appends, silent no-op when disabled, never panics. Unit tests: disabled writes nothing; enabled appends two lines.
- On-device verification needed: dual-instance exit, task install/uninstall round trip, silent elevated logon start, UAC prompt on manual launch, tray icon + Quit behavior (with ticket 08).

## Comments (reboot verification 2026-08-07)

Verified on-device: after a reboot the NitroTray logon task auto-launches the app silently at logon (log shows a fresh startup at logon time, no UAC). First run hit the tray NIM_ADD race (Explorer not ready); fixed with a retry loop (see 08). Single-instance mutex correctly rejected nothing (first instance), and a manual second launch is rejected while the first runs.
