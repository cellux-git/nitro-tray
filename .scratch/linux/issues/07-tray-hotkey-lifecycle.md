# 07 — Tray, hotkey, notifications, lifecycle, paths

**What to build:** The user-facing Linux app: a StatusNotifier tray icon with the existing menu/tooltip/battery-glyph models, D-Bus notifications, global hotkey (X11; Wayland degrades gracefully), single-instance enforcement via lock file, config/state/log relocated to XDG directories, an XDG autostart `.desktop` file toggled from the tray (replacing the Windows scheduled task), and the real entrypoint wiring the Linux adapters with the circuit-breaker recovery loop and the reapply/readback/recovery timers.

**Blocked by:** 03, 04, 05, 06, 09 (candidates 3 and 6: shared tray model + shared boot)

**Status:** ready-for-agent

- [ ] Tray shows the effective state (profile, AC/DC, capacity glyph); menu drives profile switch, smart-charge cap, and autostart toggle
- [ ] ksni adapter consumes the shared tray model (candidate 09-3) — `TrayView`/`TrayEvent`/`menu_items`/`tooltip_text`/`view_from` — no second menu derivation; the plan-name fallback in `view_from` follows the candidate-1 widened seam
- [ ] Global hotkey switches profiles on X11; Wayland logs degradation
- [ ] Second instance exits cleanly
- [ ] Config/state/log live in XDG locations; state round-trips
- [ ] Autostart toggle installs/removes the `.desktop` file
- [ ] Recovery loop reconnects adapters after failure (log-verified on-device); the shared boot helpers from candidate 09-6 carry the panic hook and adapter wiring

## Comments

2026-08-08: Adjusted for ticket 09 (done first). Candidate 09-3 extracts the pure tray model (now trapped in `cfg(windows)` tray.rs) into a shared module before this ticket starts, so this ticket's tray work is the ksni adapter against that model, not the extraction. Candidate 09-6 moves the panic hook and adapter-wiring block into shared helpers, so `linux_main` grows the tray/hotkey/timer wiring onto them. Under candidate 1, the "Windows plan" degraded section and `Profile::from_plan_name` fallback are re-expressed (plan names stay behind the Windows adapter); the `start_at_logon` flag keeps its shared meaning (scheduled task on Windows, `.desktop` toggle on Linux).
