# Nitro Tray

Nitro-Tray is an ultra-lightweight alternative to NitroSense.
It talks directly to the laptop's firmware through Acer's WMI
interface (select platform profile, fan behavior, smart-charge, keyboard
 backlight). It is provided as-is, **use at own risk**.
Tested only on Acer Nitro 16S AI (AN16-61). 

Nitro Tray is a tiny system tray application, no background services.


## Key features

- select power profile while charging (quiet, balanced, performance), or on battery (eco, balanced)
- enforces smart charge mode (non-configurable)
- enforces auto fan speed mode (non-configurable)
- disabling key backlight
- auto-start at logon
- profile cycling via shortcut

## When settings are applied:

- at startup
- on battery transitions
- on resume/wake
- at profile change

## Configuration

The config file is optional and lives **beside the executable**:
`nitro-tray.toml`. With no file present the app runs on the baked-in
defaults. The file is read at startup; restart the app to apply changes. A
reference file with every setting at its default lives in the repo as
`nitro-tray.example.toml` — copy it next to the exe as `nitro-tray.toml` and
edit.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `ac_profile` | string | `"balanced"` | Default AC profile: `quiet` \| `balanced` \| `performance` |
| `battery_profile` | string | `"eco"` | Default battery profile: `eco` \| `balanced` |
| `auto_switch` | bool | `true` | Auto-apply the profile on power transitions |
| `reapply` | bool | `false` | Periodic firmware re-assertion loop |
| `reapply_interval_secs` | integer | `30` | Loop interval (seconds) |
| `hotkey` | string | `"ctrl-alt-p"` | Global hotkey: `ctrl\|alt\|shift\|win` modifiers + key (`a`–`z`, `0`–`9`, `f1`–`f24`) |
| `keyboard_led_off` | bool | `true` | Turn the keyboard backlight off on every apply (startup, power transitions, profile change, resume); `false` leaves keyboard lighting untouched |
| `log` | bool | `true` | Write `nitro-tray.log` beside the exe; set `false` to disable |

Smart charge (80% charge cap) is always enforced on and cannot be configured.

## Platforms

- **Linux (upcoming):**
- **Windows:** all Windows-specific behavior is documented in the
  [Windows platform](#windows-platform) section below.

---

## Windows platform

### Requirements & execution model

- Windows 10 1809+ / Windows 11, x64. Single self-contained exe — no DLLs, no runtime dependencies.
- Runs elevated (Acer WMI is admin-only) — UAC prompt on manual launch.
- Optional silent elevated start at logon: the "Start at logon" tray checkbox installs the `NitroTray` logon scheduled task. Off by default; re-installed at every boot while checked.
- Single instance via a named mutex.
- In-process control only — never spawns PowerShell, `powercfg`, `schtasks`, or any external process.

### No Acer software required — disable all Acer services

- NitroSense and all Acer user-mode services can be disabled or removed — the app talks to the firmware directly through the WMI driver stack and needs none of them.
- Disable via `services.msc` or Task Manager → Services: **AASSvc**, **Acer Care Center**, **Acer Quick Access**, **Acer Hardware Launch App**, **AcerHardwareService**, **AcerDeviceEnablingServiceV2**.
- Keep the WMI driver stack (`wmiacpi`) intact — only user-mode services are disposable.

### Windows power plans

The four Nitro plans are created once from Windows Balanced (duplicate + rename + tune at creation only), detected by name at startup, recreated if deleted — manual edits to a Nitro plan are respected.

| Plan | CPU Min / Max | Boost |
|---|---|---|
| `Nitro-Quiet` | 5 / 45 | off |
| `Nitro-Balanced` | 5 / 99 | default |
| `Nitro-Performance` | 5 / 100 | aggressive |
| `Nitro-Eco` | 5 / 40 | off |

### Building on Windows

Prerequisites:

- Rust **stable** with the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools 2022 with the C++ workload and the Windows 10/11 SDK
- Git

```text
git clone <this-repo>
cd nitro-tray

# Debug build (fast, unoptimized)
cargo build

# Release build — the exe you actually run
cargo build --release
```

The release binary lands at `target\release\nitro-tray.exe` (debug:
`target\debug\nitro-tray.exe`).

Verify:

```text
cargo test          # unit tests (policy engine, config, encodings, ...)
cargo clippy        # lints; the project is clippy-clean
```

`cargo test` is pure unit tests (encodings, policy, config) — never touches
hardware or firmware, safe on any machine.

**Run the freshly built exe:**

1. Put `nitro-tray.exe` in a writable folder you own — the app writes
   `nitro-tray.log`, `nitro-tray.toml`, `nitro-tray.state.toml` beside itself
   (avoid `Program Files`).
2. Launch it once (accept the UAC prompt).
3. Optional: add a `nitro-tray.toml` (see [Configuration](#configuration)) — otherwise defaults.
4. Check "Start at logon" in the tray menu to start silently at every logon.

### Probe binaries (Windows)

Elevated, on-device hardware diagnostics: `probe_wmi.exe`, `probe_hid.exe`,
`probe_charge.exe`, `probe_power.exe`, `probe_mi.exe`, `probe_charge_read.exe`
(read-only). Run them **elevated on the target laptop** only — they require the
actual Acer hardware and **temporarily change firmware/OS state**: `probe_wmi`
cycles the profile, `probe_charge`/`probe_mi` toggle the charge cap,
`probe_hid` writes the usage mode, `probe_power` activates plans. Never invoked
by the app, the tests, or the build.

---

## Development

Hardware opcode tables and firmware encodings are documented in `docs/firmware-notes.md`.

```text
cargo build
cargo test
```

Probe binaries (run **elevated, on-device** for hardware verification):

- `probe_wmi` — Acer WMI platform profile / fan / readback
- `probe_hid` — Acer HID usage mode
- `probe_charge` — smart charge toggle / readback
- `probe_charge_read` — smart charge read-only (health byte + full status sweep)
- `probe_power` — power plan detection / activation / CPU tuning
- `probe_mi` — raw `mi.dll` transport diagnostics (BatteryControl rows, write tuples)

Every probe writes to the hardware or OS state it inspects (see
[Probe binaries (Windows)](#probe-binaries-windows)); only `probe_charge_read`
is read-only. `cargo test` runs none of them.

Hardware paths (WMI, HID, charge, power APIs) cannot be meaningfully verified
off-device: on-device verification of these paths is required before relying on
them. The WMI/smart-charge adapters run over the in-process MI stack
(`mi.dll` — the same transport PowerShell's CIM cmdlets use), bound to the
provider-enumerated instance; no COM, no external process.
