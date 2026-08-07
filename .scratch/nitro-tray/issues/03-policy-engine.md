# 03 — Policy engine

**What to build:** the pure decision logic of the app — the one seam the whole design is built to test. Given the current power state, the config, and the user's persisted profile picks, it computes the exact intended target state: firmware profile value, HID usage mode, fan behavior (always auto), smart-charge state, and the target Nitro plan. It owns the AC and battery profile lists, forward-wrap cycling, eco acceptance/fallback, and per-power-state persistence of manual picks.

**Blocked by:** 02 — Config parsing (the engine consumes config types and defaults).

**Status:** ready-for-agent

- [x] AC profile list is quiet/balanced/performance; battery list is eco/balanced; each list is position-bound for the menu.
- [x] Defaults come from config (AC balanced, battery eco); manual picks persist and are per power state — choosing one for battery does not affect AC.
- [x] Intended state maps every profile to: firmware profile value (quiet 0, balanced 1, performance 4; eco 6 when accepted), HID usage mode, fan = auto, and the matching Nitro plan.
- [x] Forward-wrap cycling moves through the current power state's list in order and wraps at the end.
- [x] Eco acceptance logic: when the machine accepts eco, intended state uses firmware profile 6; when it does not, the eco entry is disabled rather than silently failing.
- [x] Reapply (off by default) re-asserts only firmware items and never includes the active plan in its intended state.
- [x] Unit tests assert exact intended targets for representative inputs (power state × config × picks), covering defaults, per-power-state persistence, cycling order and wrap, and eco acceptance vs. disabled.

## Comments

Implemented in `src/policy.rs` on 2026-08-07. All seven checklist items are unit-testable and done (7/7).

- `PolicyEngine::new(config)` validates config profile names via `Profile::from_config_str` with fallback to AC=balanced / battery=eco, and stores `smart_charge` from config (private field, ready for ticket 10's `set_smart_charge`).
- `intended` / `reapply_intended` share one internal helper (`intended_inner`) differing only in whether the plan is included; reapply always returns `plan: None`.
- Eco with `eco_accepted=false` yields `firmware_profile: None` while HID Quiet, fan auto, smart charge, and the "Nitro-Eco" plan are still asserted (menu entry disabled, not silently failing).
- Pinned enum/const definitions and `Profile` methods untouched. Only `src/policy.rs` was edited.
- Tests: 15 new in-module tests. `cargo build` clean (no warnings), `cargo test --lib policy::` 15/15 pass, full suite 48/48 pass (33 pre-existing + 15 new).
