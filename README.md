# Nitro Tray

A single, portable `nitro-tray.exe` for Acer Nitro laptops (target family: Acer
Nitro 16S AI, AN16S-61). It lives in the system tray and keeps the machine in a
coherent power state — firmware platform profile, Acer HID usage mode, fan auto,
smart charge, and the active Windows power plan — without a service, an
installer, or any runtime dependencies. Other SKUs degrade to a "Hardware
unavailable" state (see [Degraded mode](#degraded-mode)).

- **Platform:** Windows 10 1809+ / Windows 11, x64. No runtime dependencies.
- **Elevation:** the app runs elevated because the Acer WMI interface is
  admin-only. A manual launch shows the standard UAC prompt; a silent elevated
  start at logon comes from a self-installed logon scheduled task named
  `NitroTray` (not a service). The task is installed automatically on the first
  elevated run and removed with `--uninstall`.
- **Config:** optional `nitro-tray.toml` beside the exe. The app runs on
  baked-in defaults with zero config; the file is read at startup only, so a
  restart is needed to apply changes.
- **Single instance:** enforced via a named mutex.

## What it does

At startup, on AC↔battery transitions, and on resume/wake, the app enforces a
coherent power state:

- firmware platform profile (via Acer WMI),
- Acer HID usage mode,
- fan behavior **auto**,
- smart charge (80% charge cap, on by default),
- the active Windows power plan.

Profiles are chosen independently per power state:

| Power state | Profiles | Default |
|---|---|---|
| AC | quiet, balanced, performance | balanced |
| Battery | eco, balanced | eco |

Additional behavior:

- Manual profile picks persist per power state and survive restarts.
- The **eco** profile uses the firmware's native eco mode when the firmware
  accepts it (runtime-detected on first use). If the firmware rejects it, the
  eco menu entry is disabled instead of silently failing.
- The tray menu shows the **read-back** effective state — AC/battery, battery
  %, active profile, active Windows plan, and smart-charge state — not what the
  app merely intends.
- The global hotkey **Ctrl+Alt+P** (configurable) cycles forward through the
  current power state's profile list, with a balloon notification. Automatic
  switching stays silent — no notifications.
- An optional periodic re-assertion loop re-applies the firmware-level items
  (WMI profile, HID mode, fan auto, smart-charge state) and **never** touches
  the active Windows plan, so manually chosen plans are respected. Off by
  default.
- Quitting leaves the current profile and plan in place.

### Degraded mode

If the Acer WMI interface is unreachable, the tray shows "Hardware
unavailable", profile and smart-charge items are greyed out, and only plan
switching is offered.

## Configuration

The config file is optional and lives **beside the exe**: `nitro-tray.toml`.
With no file present the app runs on the baked-in defaults. The file is read at
startup; restart the app to apply changes. A reference file with every setting
at its default lives in the repo as `nitro-tray.example.toml` — copy it next to
the exe as `nitro-tray.toml` and edit.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `smart_charge` | bool | `true` | Smart charge (80% cap) on at startup |
| `ac_profile` | string | `"balanced"` | Default AC profile: `quiet` \| `balanced` \| `performance` |
| `battery_profile` | string | `"eco"` | Default battery profile: `eco` \| `balanced` |
| `auto_switch` | bool | `true` | Auto-apply the profile on power transitions |
| `reapply` | bool | `false` | Periodic firmware re-assertion loop |
| `reapply_interval_secs` | integer | `30` | Loop interval (seconds) |
| `hotkey` | string | `"ctrl-alt-p"` | Global hotkey: `ctrl\|alt\|shift\|win` modifiers + key (`a`–`z`, `0`–`9`, `f1`–`f24`) |
| `log` | bool | `false` | Debug log to `nitro-tray.log` beside the exe (the `--log` flag enables it per launch) |

Example `nitro-tray.toml`:

```toml
# Smart charge (80% cap) on at startup
smart_charge = true

# Default profile on AC: quiet | balanced | performance
ac_profile = "balanced"

# Default profile on battery: eco | balanced
battery_profile = "eco"

# Auto-apply the profile on power transitions
auto_switch = true

# Periodic firmware re-assertion loop (off by default)
reapply = false
reapply_interval_secs = 30

# Global hotkey: ctrl|alt|shift|win + key (a-z, 0-9, f1-f24)
hotkey = "ctrl-alt-p"

# Debug log to nitro-tray.log beside the exe (or launch with --log)
log = false
```

## Command line

| Flag | Effect |
|---|---|
| `--log` | Write a debug log to `nitro-tray.log` beside the exe |
| `--uninstall` | Remove the scheduled task `NitroTray`; power plans and hardware state are left in place |

No config file is required for either flag.

## Power plans

The four Nitro plans are created once from the Windows Balanced plan
(duplicate + rename + tune at creation only), detected by name at startup,
recreated if deleted, and **never re-tuned** after creation — manual edits to a
Nitro plan are respected. Activation is done in-process.

| Plan | CPU Min / Max | Boost |
|---|---|---|
| `Nitro-Quiet` | 5 / 45 | off |
| `Nitro-Balanced` | 5 / 99 | default |
| `Nitro-Performance` | 5 / 100 | aggressive |
| `Nitro-Eco` | 5 / 40 | off |

Uninstall keeps the plans; quitting keeps the current profile and plan active.

## Security & trust

- The app runs elevated (admin) because the Acer WMI interface
  (`AcerGamingFunction`, `BatteryControl`) is admin-only on the target
  hardware. Manual launch shows a UAC prompt; logon startup is silent via the
  scheduled task.
- **All control is in-process** — the app never spawns PowerShell, `powercfg`,
  `schtasks`, or any external process at runtime. This is an intentional
  design goal: it avoids repeated antivirus triggers and keeps resource
  overhead minimal.
- No telemetry, no auto-update.

## Building from source

### Prerequisites

- Rust **stable** with the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools 2022 with the C++ workload and the Windows 10/11 SDK
- Git

### Build

```text
git clone <this-repo>
cd nitro-tray

# Debug build (fast, unoptimized)
cargo build

# Release build — the exe you actually run
cargo build --release
```

The release binary lands at `target\release\nitro-tray.exe` (debug:
`target\debug\nitro-tray.exe`). It is a single, self-contained exe — the admin
manifest and all runtime code are embedded, and there are no DLLs or other
files to ship next to it.

### Verify

```text
cargo test          # unit tests (policy engine, config, encodings, ...)
cargo clippy        # lints; the project is clippy-clean
```

### Probe binaries

The build also produces elevated, on-device hardware diagnostic binaries
(`target\release\probe-wmi.exe`, `probe-hid.exe`, `probe-charge.exe`,
`probe-power.exe`). Run them **elevated on the target laptop** to verify the
hardware paths (see [Development](#development)).

### Run the freshly built exe

1. Put `nitro-tray.exe` in a folder you own (the app writes
   `nitro-tray.log`, `nitro-tray.toml`, and `nitro-tray.state.toml` beside
   itself, so the folder must be writable — avoid `Program Files`).
2. Launch it once (accept the UAC prompt). The app installs the `NitroTray`
   logon scheduled task so it starts silently and already elevated at every
   logon.
3. Optional: drop a `nitro-tray.toml` beside the exe (see
   [Configuration](#configuration)); without it the app runs on defaults.

## Development

Code layout and the module seam contract live in
`.scratch/nitro-tray/interfaces.md`; the hardware opcode tables (ported from
the AeroForge project) are documented in `.scratch/nitro-tray/prior-art-aeroforge.md`.

```text
cargo build
cargo test
```

Probe binaries (run **elevated, on-device** for hardware verification):

- `probe-wmi` — Acer WMI platform profile / fan / readback
- `probe-hid` — Acer HID usage mode
- `probe-charge` — smart charge toggle / readback
- `probe-power` — power plan detection / activation / CPU tuning

Hardware paths (WMI, HID, charge, power APIs) cannot be meaningfully verified
off-device: on-device verification of these paths is required before relying on
them.
