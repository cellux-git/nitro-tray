# 01 — Lifecycle skeleton

**What to build:** the app runs as a single portable exe with no service and no installer. Launching it (elevated) once self-installs a logon scheduled task so that at every subsequent logon the app starts silently, already elevated, with no UAC prompt and no console window. Only one copy can run at a time. The tray icon appears with a Quit item; quitting leaves the current profile and plan in place. `--uninstall` removes the scheduled task, leaving the power plans and hardware state untouched.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] Running the exe twice keeps exactly one instance alive (named-mutex single instance).
- [ ] First elevated run installs the `NitroTray` scheduled task (logon trigger, highest privileges) that launches the exe; manual launch still shows a standard UAC prompt.
- [ ] At logon the app starts silently elevated with no UAC prompt and no console window.
- [ ] `--uninstall` removes the scheduled task so the app can be cleanly removed.
- [ ] `--log` writes a debug log beside the exe with useful diagnostics.
- [ ] Tray icon shows with a Quit menu item; quitting leaves the current profile and active plan in place.
- [ ] The app has no service, no installer, and no runtime dependencies (single portable exe).
