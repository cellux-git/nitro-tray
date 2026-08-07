# 13 — Reapply loop

**What to build:** an optional periodic re-assertion loop (off by default; interval 30s, both configurable) that re-asserts only the firmware-level items — WMI profile, HID mode, fan auto, smart-charge state — and never the active Windows plan, so manually chosen plans are respected even when enabled.

**Blocked by:** 02 — Config parsing (reapply keys); 09 — Profile selection from tray (shared firmware apply path).

**Status:** ready-for-agent

- [x] The loop is off by default and enabled via config with a configurable interval (default 30s).
- [x] When enabled, it periodically re-asserts WMI profile, HID mode, fan auto, and smart-charge state.
- [x] It never re-asserts the active Windows plan, so manual plan edits are respected.

## Comments

Implemented in `src/reapply.rs`, timer parts of `src/main.rs`, one additive
extension in `src/tray.rs` (ticket 13 owns reapply + main.rs timer parts; the
tray change is the single start_timer/stop_timer + `TrayEvent::ReapplyTick`
extension the seam contract allows).

- `enabled(cfg)` = `cfg.reapply`; `interval_ms(cfg)` = `max(secs, 1) * 1000`
  (u32-saturating), pure, unit-tested: 30 => 30000, 0 => 1000, 1 => 1000,
  5 => 5000, u64::MAX => u32::MAX.
- `on_tick(app)` = `re_evaluate_eco()` then `reapply_firmware()`; silent, no
  notify, no tray.update (display does not change on re-assert).
- main.rs: clones config before the move into `AppCore::new` (ticket 11
  pattern), arms `tray.start_timer(reapply::TIMER_ID, interval_ms)` when
  enabled (warn-log on error), and adds the `TrayEvent::ReapplyTick` arm
  calling `reapply::on_tick`.
- tray.rs: `start_timer`/`stop_timer` wrap SetTimer/KillTimerW; the existing
  WM_TIMER branch now matches on the id — POLL_TIMER_ID => poll, reapply
  TIMER_ID => `ReapplyTick` via the established send + WAKE_MSG pattern.
- `AppCore::reapply_firmware` never touches the plan: it builds intent via
  `engine.reapply_intended(state, eco_ok())` (src/app.rs:221), and
  `reapply_intended` always returns `plan: None` (src/policy.rs:178-180,
  197), so `apply_intended`'s plan branch (src/app.rs:353) is never reached.
- `cargo build --all-targets` clean (zero warnings); `cargo test`: 85 passed
  (80 existing + 5 new reapply tests).

Needs on-device verification: the real periodic loop — set `reapply = true`
(and optionally `reapply_interval_secs`) in nitro-tray.toml, confirm the
firmware profile/HID mode/fan auto/smart charge are re-asserted every
interval and that a manually selected Windows plan survives ticks; also that
the tray still quits cleanly with the timer armed.
