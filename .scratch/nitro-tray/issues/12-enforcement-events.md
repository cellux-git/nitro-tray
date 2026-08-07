# 12 — Enforcement on startup, power transitions, and resume

**What to build:** the app actively enforces a coherent power state at every important moment: at startup (the four Nitro plans are ensured to exist — recreated if deleted — and the profile for the current power state, fan auto, smart-charge default, and active plan are applied), on every AC↔battery transition (the profile for the new power state is applied silently, per config and persisted picks), and on resume/wake (re-enforced after firmware or OS resets). This must all keep working with Acer's user-mode services stopped or uninstalled.

**Blocked by:** 08 — Tray effective state (power notifications); 09 — Profile selection from tray (apply path).

**Status:** ready-for-agent

- [x] At startup: the four Nitro plans are ensured (a deleted plan is recreated), and the intended state for the current power state is applied (profile, fan auto, smart charge, active plan).
- [x] On unplug: the battery profile is applied automatically (eco by default).
- [x] On plug-in: the AC profile is applied automatically (balanced by default).
- [x] Automatic transitions are silent (no notifications).
- [x] On resume/wake: the intended state is re-enforced.
- [ ] Everything works with Acer's user-mode services stopped or uninstalled (verify on device).

## Comments

Implemented ticket 12 (enforcement on startup, power transitions, resume):

- `src/enforcement.rs`: implemented `on_startup` (ensures plans, then full
  enforcement), `on_power_changed` (honors `config.auto_switch` via the new
  `AppCore::auto_switch()` accessor; no-op when disabled; otherwise re-runs
  eco acceptance and enforces for the new power state), and `on_resume`
  (re-evaluates eco acceptance and re-enforces). All paths silent (no
  notification code; only log lines).
- `src/main.rs`: startup hook `enforcement::on_startup(&mut app)` after the
  tray exists and the initial view is set, before the message pump; the
  `TrayEvent::PowerChanged` and `TrayEvent::Resume` arms call the matching
  enforcement functions and refresh the tray view (warn-log on update error).
- `src/app.rs` (minimal additive extension): `AppCore` now stores
  `auto_switch: bool` (from config) with a tiny `pub fn auto_switch(&self) ->
  bool` accessor — the config is not otherwise retained by the core.
- Tests: no OS-independent logic exists in enforcement.rs (every path calls
  through `AppCore` into OS/hardware APIs), so the tests module documents
  that coverage is on-device; no fake tests added.
- `cargo build --all-targets` clean, `cargo test` green (68 tests).

Needs on-device verification: unplugging the charger applies the battery
profile (eco default), plugging in applies the AC profile (balanced default),
resume re-enforces, and everything works with Acer's user-mode services
stopped or uninstalled (checklist item 6).
