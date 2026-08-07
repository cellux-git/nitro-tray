# 08 — Tray dashboard (effective state)

**What to build:** the tray shows what the machine is *actually* doing, read back from hardware and OS rather than from intent: AC/battery, battery %, active profile, active Windows plan, and smart-charge state, in the menu and tooltip. A power-state source (system power status + power notifications + slow poll fallback) feeds it. If the Acer WMI interface is unreachable, the tray shows a degraded "Hardware unavailable" state with the profile and smart-charge items greyed out, so the user understands why control is missing.

**Blocked by:** 01 — Lifecycle skeleton; 04 — Power API wrapper; 05 — Acer WMI adapter; 06 — Acer HID adapter; 07 — Smart charge adapter.

**Status:** ready-for-agent

- [x] Menu and tooltip display read-back values: AC/battery, battery %, active profile, active Windows plan, smart-charge state — all read from hardware/OS, not app intent.
- [x] Power state is sourced from system power status, updated by power notifications, with a slow poll fallback.
- [x] Left-click opens the menu.
- [x] When the Acer WMI interface is unreachable, the tray shows a degraded "Hardware unavailable" state with profile and smart-charge items greyed out.

## Comments

Implemented by ticket 08. Files touched: `src/tray.rs`, `src/power_state.rs`, plus
one additive line in `Cargo.toml` (windows-sys `Win32_System_LibraryLoader`
feature — see note below). `src/main.rs` unchanged; the ticket-01 wiring
(`Tray::create`, `tray.update(&view)`, message pump) compiles and behaves as-is.

What was built:

- `power_state::read()` via `GetSystemPowerStatus` (windows-sys names it without
  the `W` suffix); `ACLineStatus == 1` -> `Ac`, else `Battery`; unknown
  `BatteryLifePercent` (255) maps to 0. Pure `snapshot_from_status` mapping is
  unit-tested (5 tests).
- `Tray::create`: hidden `WS_OVERLAPPEDWINDOW` window, custom class
  `NitroTrayTrayWnd` (registered once, guarded by an atomic), tray state in
  `GWLP_USERDATA`, runtime-generated 16x16 battery icon (32bpp top-down DIB +
  `CreateIconIndirect`; the DIB handle is kept for the icon's lifetime and both
  are freed on drop), `NIM_ADD` with `NIF_MESSAGE|NIF_ICON|NIF_TIP|NIF_SHOWTIP`,
  10 s poll timer, `RegisterPowerSettingNotification(GUID_ACDC_POWER_SOURCE,
  DEVICE_NOTIFY_WINDOW_HANDLE)` — the GUID is hardcoded (not in windows-sys).
- WndProc: `WM_LBUTTONUP`/`WM_RBUTTONUP` on the tray callback message open the
  menu (built fresh from the stored view: degraded header, profile group with
  check/grey per view, smart-charge toggle, plan info row, Quit), routed via
  `TrackPopupMenu` + `TPM_RETURNCMD`; `WM_POWERBROADCAST` raises
  `PowerChanged` (`PBT_APMPOWERSTATUSCHANGE`), `Resume`
  (`PBT_APMRESUMEAUTOMATIC|PBT_APMRESUMESUSPEND`), and the slow poll +
  `PBT_POWERSETTINGCHANGE` compare the fresh snapshot to the stored one and
  raise `PowerChanged` only on change. Every channel send posts `WAKE_MSG`
  (WM_APP+2) to unblock the `GetMessageW` pump; a closed channel stops all
  further posting.
- `update()` stores the view, rebuilds the tooltip (`tooltip_text`, pure and
  unit-tested: power state, %, profile, plan, smart charge, degraded prefix,
  `\n\n\n` two-line trick) and `NIM_MODIFY`s it.
- `notify()` balloons via `NIF_INFO`/`NIIF_INFO`.
- `Drop`: `NIM_DELETE`, unregister power notification, kill timer, clear
  userdata, `DestroyWindow`, `DestroyIcon` + `DeleteObject`.
- All failure paths map to `TrayError::Create`/`Update`.

windows-sys surprises:

- `GetSystemPowerStatus` is exported without the `W` suffix.
- `GUID_ACDC_POWER_SOURCE` is not shipped; hardcoded.
- `GetModuleHandleW` lives in `Win32_System_LibraryLoader`, which was not an
  enabled feature — added it to Cargo.toml (additive; the only manifest change).
- `w!` macro takes literals only (no `w!(CONST)`).

On-device verification still needed: icon appearance in the tray, two-line
tooltip rendering, menu layout/checked states, balloon display, and real
power-message behavior (unplug/plug, sleep/resume, and the 10 s poll fallback
when `RegisterPowerSettingNotification` is unavailable).
