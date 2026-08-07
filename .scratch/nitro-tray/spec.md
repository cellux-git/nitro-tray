# Nitro Tray — Spec

Status: ready-for-agent

> Synthesized from the design interview (grilling session, 2026-08-07). Target hardware: Acer Nitro 16S AI (AN16S-61). Not yet on an issue tracker — this file is the source of truth until a project dir/tracker exists; apply the `ready-for-agent` label when it lands there.

## Problem Statement

Acer NitroSense and the heavier AeroForge app both require a Windows service, a GUI webview shell, and/or Acer's own services to do a small job: keep the battery protected, switch the machine between sensible power profiles, keep the fans in auto, and show the user what is currently in effect. The user wants a single, dependency-free `.exe` that lives in the system tray, needs no service and no installer, and does this one job well — including a battery "eco" profile that NitroSense has but AeroForge does not.

The hardware control layer (ACPI-WMI `AcerGamingFunction`, Acer HID, smart-charge `BatteryControl`, `powercfg`) is proven in the AeroForge codebase and is independent of Acer's user-mode services (`AcerDeviceEnablingServiceV2` etc.), so the service and the desktop shell can be dropped entirely.

AeroForge's control paths spawn `powershell.exe` processes (CIM/PowerShell fallbacks), which is resource-costly because every spawned PowerShell process constantly re-triggers antivirus scans. Nitro Tray must therefore perform **all operations in-process**: no spawning of PowerShell (or any other interpreter) at runtime — every hardware/OS control call is made through in-process APIs (raw COM/WMI, Windows power-management APIs, HID feature reports).

## Solution

A single Rust executable (`nitro-tray.exe`) that runs in the system tray, elevated via a self-installed logon scheduled task (no service, no installer). At startup, on power transitions, and on resume it enforces a coherent power state: the Acer firmware platform profile, the Acer HID usage mode, fan mode **auto**, the smart-charge state, and the active Windows power plan — with separate profile choices for AC vs battery. It exposes in the tray the *current effective* state (read back from hardware/OS), lets the user pick the profile for the current power state, toggle smart charge, and cycle the profile with a configurable global hotkey (`Ctrl+Alt+P` default). A config file beside the exe (`nitro-tray.toml`) is optional; baked-in defaults make it run with zero config.

## User Stories

1. As a Nitro laptop user, I want a single portable `.exe` with no installers, services, or runtime dependencies, so that the app is trivially simple to run.
2. As a Nitro laptop user, I want the app to start automatically at logon without any UAC prompt, so that my power settings are enforced without me remembering to launch it.
3. As a Nitro laptop user, I want the app to run in the system tray, so that it stays out of my way.
4. As a Nitro laptop user, I want smart charge (80% charge cap) enabled by default at startup, so that my battery is protected without configuration.
5. As a Nitro laptop user, I want to see the current effective state in the tray menu (AC/battery, battery %, active profile, active Windows plan, smart-charge state), so that I always know what the machine is actually doing.
6. As a Nitro laptop user, I want the effective state shown in the tray to be read back from the hardware/OS, not just what the app intends, so that I can trust the display.
7. As a Nitro laptop user on AC, I want to choose between quiet, balanced, and performance profiles, so that I can trade performance for acoustics/heat.
8. As a Nitro laptop user on battery, I want to choose between eco and balanced profiles, so that I can trade performance for battery life.
9. As a Nitro laptop user, I want the tray profile menu to show only the profiles valid for the current power state, so that I don't have to guess what's available.
10. As a Nitro laptop user, I want the app to automatically apply my battery profile (eco by default) the moment the power is unplugged, so that battery life is maximized without action.
11. As a Nitro laptop user, I want the app to automatically apply my AC profile (balanced by default) the moment the power is plugged back in, so that performance returns automatically.
12. As a Nitro laptop user, I want to be able to manually pick a profile that persists, so that my choice is not overridden on a timer.
13. As a Nitro laptop user, I want my manual profile choice for one power state not to affect the other power state's profile, so that AC and battery choices stay independent.
14. As a Nitro laptop user, I want selecting a profile to switch to a real, visible Windows power plan named for that profile (e.g. Nitro-Performance), so that Windows itself runs in the matching power configuration.
15. As a Nitro laptop user, I want the four Nitro power plans created once from the Windows Balanced plan, so that no extra power plans exist before I need them.
16. As a Nitro laptop user, I want my manual edits to a Nitro power plan to be respected, so that I can fine-tune a profile without the app undoing it.
17. As a Nitro laptop user, I want a deleted Nitro power plan to be recreated automatically, so that the app heals itself.
18. As a Nitro laptop user, I want the fans forced to auto on every profile and at startup/resume, so that I never end up in a noisy manual fan state.
19. As a Nitro laptop user, I want to be able to toggle smart charge from the tray, so that I can charge to 100% when I want.
20. As a Nitro laptop user, I want the tray to show the read-back smart-charge state, so that I can confirm it took effect.
21. As a Nitro laptop user, I want the "eco" battery profile to use the firmware's native eco mode when the machine supports it, so that I get the most battery-efficient behavior.
22. As a Nitro laptop user on a machine whose firmware rejects eco, I want the eco menu entry disabled rather than silently failing, so that I never select a profile that does nothing.
23. As a Nitro laptop user, I want to cycle the profile with a global hotkey (Ctrl+Alt+P by default), so that I can switch profiles without opening the tray.
24. As a Nitro laptop user, I want the hotkey to cycle forward through the current power state's profile list and wrap, so that cycling is predictable.
25. As a Nitro laptop user, I want a brief notification when I press the hotkey, so that I get feedback on the new profile.
26. As a Nitro laptop user, I want automatic power-transition switching to stay silent (no notifications), so that I'm not spammed when the power state changes.
27. As a Nitro laptop user, I want enforcement to run on startup, power transitions, and resume/wake, so that firmware or OS resets are corrected.
28. As a Nitro laptop user, I want the periodic re-assertion loop to be off by default but configurable, so that it can be enabled if a particular vendor process fights the settings.
29. As a Nitro laptop user, I want to run the app with a `--log` flag to write a debug log, so that issues can be diagnosed.
30. As a Nitro laptop user, I want a config file beside the exe with documented contents, so that defaults and the hotkey can be adjusted.
31. As a Nitro laptop user, I want the app to run with no config file at all, so that it is usable out of the box.
32. As a Nitro laptop user, I want only a single instance of the app running, so that two copies don't fight over the hardware.
33. As a Nitro laptop user, I want the app to keep working when Acer's services are stopped or uninstalled, so that I can remove Acer software completely.
34. As a Nitro laptop user, I want an `--uninstall` action that removes the scheduled task, so that the app can be cleanly removed.
35. As a Nitro laptop user, I want the Nitro power plans to remain after uninstall, so that Windows is left in a sensible state if I ever reinstall.
36. As a Nitro laptop user, I want the tray to show a degraded "Hardware unavailable" state with profile/smart-charge items greyed out if the Acer WMI interface is unreachable, so that I understand why the app can't control the hardware.
37. As a Nitro laptop user, I want a plan switch to still be offered if the Windows power APIs work even when Acer WMI is unavailable, so that partial function is preserved.
38. As a Nitro laptop user, I want quitting the app to leave the current profile and plan in place, so that nothing snaps back to a default.
39. As a Nitro laptop user, I want the app to work on both Windows 10 (1809+) and Windows 11 x64, so that I'm not restricted to one OS.
40. As a Nitro laptop user, I want the app to perform all control operations in-process without spawning PowerShell or any external interpreter, so that antivirus is not repeatedly triggered and CPU/resource overhead stays minimal.

## Implementation Decisions

### Architecture
- **Single executable**, no service, no installer, no runtime dependencies. Rust, x64, Windows 10 1809+ / Windows 11. Manifest `requireAdministrator` because the Acer WMI classes (`AcerGamingFunction`, `BatteryControl`) are admin-restricted even for reads on the target hardware.
- **Elevation via scheduled task**: the first elevated run self-installs a logon scheduled task (run only when the user is logged on, highest privileges) that launches the exe, giving a silent elevated start at logon. Manual launch still yields a standard UAC prompt. The task is not a service.
- **Single instance** enforced via a named mutex.
- **In-process only**: the app never spawns PowerShell or any external interpreter. PowerShell/CIM fallbacks used by AeroForge are dropped; all control and readback is done through in-process COM/WMI and Windows APIs to avoid repeated antivirus triggers and process-spawn overhead.
- **No auto-update.** Quitting leaves profile/plan state as-is.
- Porting the hardware opcode/method tables from the AeroForge service code (proven against ANV15-41 / ANV16-41 / AN16S-61) rather than reimplementing them.

### Module breakdown (interfaces, not paths)
- **Policy engine** (highest testing seam — pure decision logic): inputs are current power state (AC/battery), config, and persisted profile selections; output is the intended target state: firmware profile value, HID usage mode, fan behavior (always auto), smart-charge state, and the target Nitro plan. Owns the AC/battery profile lists, the position-bound mapping, forward-wrap cycling, eco acceptance/fallback, and the reapply (off-by-default) schedule.
- **Acer WMI adapter**: raw COM/WMI `ExecMethod` against `AcerGamingFunction` (`SetGamingMiscSetting(0x0B, …)` platform profile; `SetGamingFanBehavior` auto `0x00410009`; readback via `GetGamingMiscSetting`/`GetGamingFanBehavior`/`GetGamingSysInfo`). Admin-only on target hardware. **No PowerShell/CIM fallback** — the AeroForge `powershell.exe` fallback path is intentionally not ported.
- **Acer HID adapter**: feature report writes for system-usage mode (Quiet/Normal/Performance) on the vendor 0x1025 device. HID write failure is non-fatal (logged; WMI + plan still applied).
- **Smart charge adapter**: `BatteryControl` WMI health-status toggle (80% cap) via raw in-process COM (no PowerShell). Readback via `GetBatteryHealthControlStatus`. Uses the AMD "direct-trust" write path for this SKU class.
- **Power API wrapper (in-process)**: uses the Windows power-management APIs via `windows-sys` (`PowerDuplicateScheme`, `PowerSetActiveScheme`, `PowerGetActiveScheme`, `PowerWriteACValueIndex`/`PowerWriteDCValueIndex`, `PowerReadDCValueIndex`/`PowerReadACValueIndex`) to create the four Nitro plans once (duplicate Balanced + rename), tune per the plan table, activate the target plan, and read the active plan and processor states. **Never spawns `powercfg.exe`**; the `powercfg /setactive` / `/duplicatescheme` approach from the design interview is implemented in-process.
- **Power state source**: `GetSystemPowerStatus` (AC/battery, battery %); event-driven via power notifications (WM_POWERBROADCAST / registered power-setting notifications) with a slow poll fallback.
- **Config**: optional TOML beside the exe (documented filename in the app README), baked-in defaults, read at startup (restart to apply).
- **Tray UI**: menu per the shared understanding; read-back of "current effective" rather than intent; tooltip with battery % and profile; static icon; left-click opens the menu.
- **Hotkey**: `RegisterHotKey`, configurable (`hotkey`, default `ctrl-alt-p`), forward-wrap cycle within the current power state's list, transient balloon feedback. Auto-switch stays silent.
- **Lifecycle**: scheduled-task installer, `--uninstall` (removes task; plans left in place), `--log` debug logging (`nitro-tray.log` beside the exe), mutex.

### Profiles and plans
- AC list: quiet, balanced, performance. Battery list: eco, balanced. Defaults: AC = balanced, battery = eco (config-overridable). Manual picks persist and are per-power-state.
- Firmware profile mapping: quiet 0, balanced 1, performance 4, turbo 5 (turbo unused). Eco uses firmware profile 6 when accepted; otherwise the eco menu entry is disabled (detected at runtime on first eco attempt via write+readback; re-evaluated later).
- HID usage mode per profile: quiet → Quiet, balanced → Normal, performance → Performance, eco → Quiet.
- Plans `Nitro-Quiet/-Balanced/-Performance/-Eco` are created once (duplicate Balanced, rename), detected by name at startup, recreated if deleted, **never re-tuned** after creation (activation only, via in-process `PowerSetActiveScheme`).

| Plan | CPU Min/Max | Boost |
|---|---|---|
| Nitro-Quiet | 5 / 45 | off |
| Nitro-Balanced | 5 / 99 | default |
| Nitro-Performance | 5 / 100 | aggressive |
| Nitro-Eco | 5 / 40 | off |

### Enforcement
- Always on events (startup, AC↔battery transition, resume/wake): WMI platform profile, HID usage mode, fan behavior auto, smart-charge state, active plan.
- Periodic re-assertion loop: configurable (`reapply`, default `false`; `reapply_interval_secs` default 30). When enabled it re-asserts only the firmware-level items (WMI profile, HID mode, fan auto, smart-charge state) and **never** the active Windows plan, so manually chosen plans are respected.
- Intended state derives from config + current power state; tray toggles update intent immediately.

### Config keys
`smart_charge` (default `true`), `ac_profile` (default `"balanced"`), `battery_profile` (default `"eco"`), `auto_switch` (default `true`), `reapply` (default `false`), `reapply_interval_secs` (default `30`), `hotkey` (default `"ctrl-alt-p"`).

### Naming
exe `nitro-tray.exe`, config `nitro-tray.toml`, scheduled task `NitroTray`, log `nitro-tray.log`, plans `Nitro-Quiet / Nitro-Balanced / Nitro-Performance / Nitro-Eco`.

## Testing Decisions

- **What makes a good test**: only external behavior of the policy engine — given a power state, config, profile selections, and (for readback paths) a current-effective snapshot, assert the exact intended target (firmware profile value, HID mode, fan=auto, smart-charge state, target plan). Not implementation details.
- **Policy engine unit tests** (the one primary seam): profile chosen per power state; position-bound menu lists; forward-wrap cycling order and wrap; eco acceptance → firmware eco vs entry disabled; reapply on/off behavior (and that the plan is never re-asserted by the timer); manual pick persistence; defaults.
- **Config parsing tests**: no-file → baked defaults; partial file → defaults fill gaps; invalid values rejected gracefully.
- **Power API wrapper tests**: plan detection by name, active-plan readback, and processor-state read/write encoding via the in-process power APIs.
- **Opcode-encoding tests** for the WMI/HID tables (prior art: the AeroForge service already has unit tests asserting fan-speed/profile opcode encodings, e.g. `0x1401` for CPU 20%, `0x5004` for GPU 80%).
- **Integration seam (hardware)**: on-device verification only — an elevated probe script in the style of the existing AeroForge probe scripts exercises real WMI/HID/power-API writes and readbacks (a test-time diagnostic only; the app itself never spawns external processes), since these interfaces cannot be mocked meaningfully and the target is the user's own machine.
- Hardware adapters deliberately kept thin so the policy engine remains the single tested seam; no test doubles for the OS/WMI layer.

## Out of Scope

- Fan control UI or manual fan curves (fan is auto-only enforcement; fan display was explicitly deferred).
- CPU PL1/RAPL limiting (Intel-MSR-only) and any GPU tuning (NVML/NVAPI/Whisper).
- Boot-logo / EFI writing, display/refresh, blue-light, Nitro-key handling, telemetry history/history dashboards.
- AeroForge-style NitroSense process management (user assumes NitroSense is not installed).
- Installer, auto-update, notifications on automatic switching.
- Validation on SKUs other than the target hardware family (unsupported hardware handled by the degraded "Hardware unavailable" state only).

## Further Notes

- **Open verification (non-blocking)**: (1) whether AeroForge still applies profiles with Acer user-mode services disabled — structural evidence says yes, the user's disable experiment will confirm; (2) whether firmware eco (profile 6) is accepted on AN16S-61 — runtime detection at first eco selection.
- Acer's user-mode services (`AcerDeviceEnablingServiceV2`, etc.) are not required: `AcerGamingFunction` is provided by Windows' ACPI-WMI provider (WmiProv, guid `{7A4DDFE7-5B5D-40B4-8595-4408E0CC7F56}`) and executes ACPI methods in the firmware DSDT.
- The app README must document the config filename and all config keys.
- This spec is intended to be published to the project issue tracker with the `ready-for-agent` label once a project dir/tracker exists.
