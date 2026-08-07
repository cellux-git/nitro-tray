# Nitro Tray — Implementation Seams (contract for sub-agents)

Source of truth for module boundaries while the tickets are implemented in
waves. Each ticket owns exactly the files listed below; agents MUST NOT edit
files owned by other tickets. The public APIs pinned in the `src/*.rs` stubs
are contracts — keep signatures stable; extend only where the owning ticket
requires it.

## Prior art

`.scratch/nitro-tray/prior-art-aeroforge.md` — opcode/method tables extracted
from the AeroForge codebase (sibling repo at
`D:\dev\source\aeroforge-nitrosense-alternative`, read-only). Read it before
implementing tickets 04/05/06/07.

## Shared COM/WMI module

`src/comwbem.rs` (post-review addition) owns ALL hand-rolled COM/WMI machinery
(wbemcli.h vtables, minimal VARIANT, BSTR/SAFEARRAY helpers, COM apartment
guard, `ExecMethod`/`GetObject`/`first_instance_path`). The WMI and smart-charge
adapters (`src/wmi.rs`, `src/charge.rs`) only encode their method tables and map
errors; they must NOT redeclare vtables or variants. Every `pub unsafe fn` in
comwbem carries a `# Safety` contract.

## Ownership map

| Ticket | Files owned |
|---|---|
| 01 Lifecycle | `src/main.rs`, `src/task.rs`, `src/log.rs`, manifest wiring |
| 02 Config | `src/config.rs` |
| 03 Policy | `src/policy.rs` |
| 04 Power | `src/power.rs`, `src/bin/probe_power.rs` |
| 05 WMI | `src/wmi.rs`, `src/bin/probe_wmi.rs` |
| 06 HID | `src/hid.rs`, `src/bin/probe_hid.rs` |
| 07 Charge | `src/charge.rs`, `src/bin/probe_charge.rs` |
| 08 Tray | `src/tray.rs`, `src/power_state.rs`, tray parts of `src/main.rs` |
| 09 App core | `src/app.rs`, app-core parts of `src/main.rs` |
| 10 Toggle | smart-charge parts of `src/tray.rs` + `src/main.rs` |
| 11 Hotkey | `src/hotkey.rs`, hotkey parts of `src/main.rs` |
| 12 Enforcement | `src/enforcement.rs`, event parts of `src/main.rs` |
| 13 Reapply | `src/reapply.rs`, timer parts of `src/main.rs` |
| 14 Docs | `README.md` |
| (shared) | `src/comwbem.rs` — owned by nobody/refactor; WMI + charge adapters use it |

`src/main.rs` is extended sequentially (01 -> 08 -> 09 -> 10 -> 12 -> 11 -> 13).
Each later ticket reads main.rs first and adds its own match arms; keep the
structure flat: one event-dispatch match, one set of `app` calls, one
`tray.update(...)`.

## Naming (fixed)

- exe `nitro-tray.exe`, config `nitro-tray.toml`, state `nitro-tray.state.toml`,
  log `nitro-tray.log`, task `NitroTray`, mutex `Local\NitroTray`
- plans `Nitro-Quiet / Nitro-Balanced / Nitro-Performance / Nitro-Eco`
- profiles quiet/balanced/performance/eco; AC list [quiet, balanced,
  performance], battery list [eco, balanced]; defaults AC=balanced,
  battery=eco

## Design decisions (fixed)

1. **Power-state flow**: tray window's WndProc raises `TrayEvent::PowerChanged`
   on `WM_POWERBROADCAST` `PBT_APMPOWERSTATUSCHANGE`/`PBT_POWERSETTINGCHANGE`
   and on a slow-poll timer (10 s, `power_state::SLOW_POLL_MS`) — gated on the
   AC/battery STATE changing, never on battery-% drift (manual plan edits must
   not be clobbered by percent ticks); `TrayEvent::Resume` on
   `PBT_APMRESUMEAUTOMATIC` / `PBT_APMRESUMESUSPEND`. The main loop maps events
   -> `enforcement::*`.
2. **Eco acceptance**: cached in `AppCore` as `eco_accepted: Option<bool>`.
   First eco apply writes firmware profile 6 via `WmiAdapter`, then readback;
   mismatch (or error) => rejected AND the previously active firmware profile
   is restored (best effort) so the machine is never left in an unspecified
   firmware state. Re-evaluated when currently rejected (on power transitions,
   on reapply ticks), when acceptance is still unknown and the pick is eco, and
   on each eco selection attempt. `cycle_profile` skips a disabled eco so the
   hotkey can never select it.
3. **Persistence**: `nitro-tray.state.toml` beside the exe holds
   `[picks] ac = "balanced" battery = "eco"` and `smart_charge = true` (only
   written when the user changes something; absent entries fall back to
   config). Toggling smart charge updates intent AND persists it, so startup
   enforcement keeps the user's choice.
4. **Degraded mode**: `WmiAdapter::connect()` failing => `wmi_available() ==
   false`; tray shows "Hardware unavailable", profile + smart-charge items
   greyed (the eco entry is greyed individually via `TrayView.eco_disabled`
   when the firmware rejected profile 6); the "Windows plan" section
   (`TrayView.plans`, raised as `TrayEvent::SelectPlan`) still offers plan
   switching via `AppCore::apply_plan` (plan-only, no firmware).
5. **Apply path (AppCore)**: full apply = WMI profile (if available), HID
   usage mode (log-only on failure), fan auto (WMI), smart charge, active
   plan. HID failure is never fatal.
6. **Silence**: automatic switching (startup/transitions/resume/reapply)
   produces no notifications. Only the hotkey path calls `tray.notify(...)`.
7. **In-process only**: never spawn PowerShell, `powercfg`, `schtasks`, or any
   external process at runtime. Build-time scripts and the probe binaries
   (test-time diagnostics) are the only exceptions. Note: windows-sys has no
   TaskScheduler bindings (taskschd was removed from the shared Windows
   metadata); `src/task.rs` uses the `winapi` crate's `taskschd` COM bindings
   (correct GUIDs/vtables) — the only module allowed to depend on winapi.
8. **Plan table (spec, authoritative — differs from AeroForge's)**: Quiet
   5/45 boost-off, Balanced 5/99 boost-default, Performance 5/100 boost-
   aggressive, Eco 5/40 boost-off. Plans created once from Windows Balanced
   (duplicate + rename + tune at creation only), detected by name, recreated
   if deleted, never re-tuned, activated via in-process `PowerSetActiveScheme`.
9. **Manifest**: `res/app.manifest` requires administrator (already wired via
   `build.rs`); `#![windows_subsystem = "windows"]` already in main.rs.

## Interfaces (pinned in stubs; do not drift)

See the stub files themselves. Key cross-module types:
`config::Config` (+`parse`/`load`), `policy::{PowerState, Profile, HidMode,
IntendedState, PolicyEngine, AC_PROFILES, BATTERY_PROFILES}`,
`power::{PowerApi, PowerError, CpuTuning, BoostMode, cpu_tuning, NITRO_PLANS}`
(NITRO_PLANS is derived from `Profile::plan_name` — single source of truth),
`wmi::{WmiAdapter, WmiError, PROFILE_*, FAN_AUTO, SETTING_PLATFORM_PROFILE}`,
`hid::{HidAdapter, HidError, usage_mode_report, usage_mode_from_selector}`,
`charge::{SmartChargeAdapter, ChargeError, direct_trust_tuple,
fallback_tuples, desired_status_from_rows, method_succeeded}` (a set attempt
only counts as success with a present, truthy, non-error `ReturnValue`),
`power_state::{read, PowerStateSnapshot, SLOW_POLL_MS}`,
`tray::{Tray, TrayView, TrayEvent, TrayError}` (TrayView also carries
`eco_disabled` + `plans`; `TrayEvent::SelectPlan`),
`app::{AppCore, EffectiveState, STATE_FILE_NAME}` (+`apply_plan`),
`hotkey::{Hotkey, parse_spec, DEFAULT_SPEC}`, `enforcement::{on_startup,
on_power_changed, on_resume}`, `reapply::{enabled, interval_ms, on_tick,
TIMER_ID}`, `task::{install_logon_task, uninstall_logon_task, TASK_NAME}`,
`log::{set_enabled, init, info, warn, error}`, `comwbem::{...}`.

## Workflow notes for agents

- Other agents may be editing other modules concurrently. If `cargo build`
  fails with errors ONLY in files you don't own, wait a moment and re-run;
  never fix files owned by other tickets.
- Verify with `cargo build` and `cargo test` (whole crate compiles with stubs).
- On-device-only items (hardware probes, elevated behavior, task install,
  tray visuals) cannot be verified in this environment: implement them, mark
  the ticket checkboxes that need on-device verification unchecked, append an
  `## Comments` section to the ticket file recording exactly what was done and
  what needs device verification.
