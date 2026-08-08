# 07 — Tray, hotkey, notifications, lifecycle, paths

**What to build:** The user-facing Linux app: a StatusNotifier tray icon with the existing menu/tooltip/battery-glyph models, D-Bus notifications, global hotkey (X11; Wayland degrades gracefully), single-instance enforcement via lock file, config/state/log relocated to XDG directories, an XDG autostart `.desktop` file toggled from the tray (replacing the Windows scheduled task), and the real entrypoint wiring the Linux adapters with the circuit-breaker recovery loop and the reapply/readback/recovery timers.

**Blocked by:** 03, 04, 05, 06

**Status:** ready-for-agent

- [ ] Tray shows the effective state (profile, AC/DC, capacity glyph); menu drives profile switch, smart-charge cap, and autostart toggle
- [ ] Global hotkey switches profiles on X11; Wayland logs degradation
- [ ] Second instance exits cleanly
- [ ] Config/state/log live in XDG locations; state round-trips
- [ ] Autostart toggle installs/removes the `.desktop` file
- [ ] Recovery loop reconnects adapters after failure (log-verified on-device)
