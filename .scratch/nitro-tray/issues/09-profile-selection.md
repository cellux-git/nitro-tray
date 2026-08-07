# 09 — Profile selection from tray

**What to build:** the tray menu lists only the profiles valid for the current power state (AC: quiet/balanced/performance; battery: eco/balanced). Picking a profile applies it immediately — firmware profile, HID usage mode, fan auto, and the matching Nitro plan — and the pick persists per power state, never overridden by a timer. Eco is detected at runtime: the first eco attempt writes firmware profile 6 and reads back; if the firmware rejects it, the eco entry is disabled (and re-evaluated later) instead of silently failing. When the Acer WMI interface is down, plan switching is still offered.

**Blocked by:** 03 — Policy engine; 04 — Power API wrapper; 05 — Acer WMI adapter; 06 — Acer HID adapter; 08 — Tray effective state.

**Status:** ready-for-agent

- [x] Menu shows only the profiles valid for the current power state.
- [x] Selecting a profile applies it immediately: firmware profile + HID usage mode + fan auto + active Nitro plan.
- [x] Picks persist per power state and are not overridden on a timer.
- [x] Eco runtime detection: first eco attempt writes firmware profile 6 and reads back; on rejection the eco entry is disabled, re-evaluated later.
- [x] When the Acer WMI interface is unreachable, the plan switch for the chosen profile is still offered and applied.

## Comments

Implemented 2026-08-07 (ticket 09, app core). Files: `src/app.rs`, app-core
parts of `src/main.rs`.

- `AppCore` (src/app.rs): engine + runtime smart-charge intent + adapters
  (WMI/HID/charge, all degrade to `None` with a warn log), `eco_accepted`,
  cached power snapshot, state-file path. State file
  (`nitro-tray.state.toml` beside the exe) overrides engine picks (valid
  names only) and the smart-charge intent; missing/malformed file => defaults.
  Writes only on user actions (apply_profile / cycle_profile /
  toggle_smart_charge) via the pure `serialize_state` helper (toml::Value,
  no serde derive); loaded via the pure `load_state` helper.
- `effective()`: fresh `power_state::read()`; firmware profile readback
  mapped 0/1/4/6 -> Quiet/Balanced/Performance/Eco (`profile_from_firmware`,
  unknown values e.g. turbo 5 => None); plan + smart-charge readbacks;
  failure-tolerant, each error kind logged at most once per run via
  `std::sync::Once`.
- Apply path (`apply_intended`): WMI profile (only when firmware value
  Some), HID usage mode (log-only on failure, never fatal), fan auto, smart
  charge, active plan. Plan applies regardless of WMI availability, so
  plan switching survives degraded mode (checklist item 5).
- Eco detection flow: `detect_eco()` writes firmware profile 6 via the WMI
  adapter and reads back — readback == 6 => accepted; anything else (or a
  set error / failed readback while WMI is up) => rejected, cached in
  `eco_accepted`. Runs on every eco selection (apply_profile/cycle_profile),
  on the first automatic eco apply when acceptance is still unknown
  (inside `apply_full`, covering startup/transition enforcement), and via
  `re_evaluate_eco()` (re-tests only when currently rejected; writes 6
  directly, does NOT restore the old profile — callers re-apply the
  intended state right after, documented in the doc comment).
  `eco_disabled()` = `eco_accepted == Some(false)`; `view_from` filters Eco
  out of the offered profiles when disabled.
- `enforce_now()` = ensure plans (errors logged, keeps going) + full apply,
  silent. `reapply_firmware()` = `reapply_intended` items only (WMI profile,
  HID, fan auto, smart charge), never the plan.
- main.rs: `view_from(app)` builds the TrayView from `app.effective()` +
  `app.current_power()` (profiles list per power state, eco filtered when
  disabled, greyed/degraded flags from `wmi_available()`); used for the
  initial view and after profile selection. `TrayEvent::SelectProfile` arm
  implemented: `app.apply_profile(p)` then `tray.update(&view_from(app))`
  (warn on error). ToggleSmartCharge/PowerChanged/Resume arms untouched
  (tickets 10/12).
- Tests: 9 new in src/app.rs — state-file round trips (serialize then
  parse via the pure helpers, all profiles/toggles), empty/malformed/
  partial/wrong-type state files => defaults, unknown keys ignored,
  serialized text is valid TOML, firmware readback mapping. Full suite:
  68 passed (59 existing + 9 new), zero warnings in owned files.

Needs on-device verification: real WMI/HID/smart-charge connect + writes
and readbacks (this machine degrades, so effective() returns None forms);
eco acceptance on AN16S-61 (write 6 + readback path); plan switching with
WMI down; persisted picks surviving a restart.

## Comments (code review)

2026-08-07: Review fixes: (1) eco rejection now restores the previously active firmware profile (best effort) instead of leaving the machine in an unspecified firmware state; (2) re_evaluate_eco also re-tests when acceptance is unknown and the pick is eco; (3) cycle_profile skips a disabled eco entry so the hotkey cannot select it; (4) profile_from_firmware now uses the wmi::PROFILE_* constants instead of raw literals.
