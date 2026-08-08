# 01 — Linux port: same firmware control, native Linux stack

**What to build:** port nitro-tray to Linux (x64). The machine-controlling knowledge that took tickets 05–16 to build lives on the *firmware* side — ACPI-WMI (AcerGamingFunction, BatteryControl) and the Acer HID feature-report protocol are OS-independent — so the port reuses the policy engine, opcode tables, and adapter seams and replaces the OS-specific layers: the MI transport (`mi.rs`, mi.dll), the power layer (`power.rs`, Windows power APIs), the tray/hotkey backends, and the elevation model (single elevated exe + scheduled task today).

**Why:** the target is the user's own AN16S-61, and they may run Linux on it (dual boot). The app's value — eco/balanced/performance profiles, fan auto, smart-charge cap, per-power-state enforcement — is exactly what's missing from the Linux laptop-tool ecosystem for Acer Nitro machines.

**Prior art / evidence:**

- The ACPI-WMI GUID `{7A4DDFE7-5B5D-40B4-8595-4408E0CC7F56}` is a DSDT device, not a Windows artifact: Linux's WMI framework (`drivers/platform/x86/wmi.c`) enumerates ACPI-WMI devices from `_WDG` and can execute MOF-announced methods from userspace via the WMI chardev ioctl (`WMI_IOCTL_EXEC_METHOD`, `include/uapi/linux/wmi.h`). This is the Linux analog of the instance-bound MI invoke (ticket 16, nitro-tray feature): same GUID, same method names (`SetGamingMiscSetting`…), same gmInput semantics.
- Existing Linux projects already drive these Acer gaming SKUs (e.g. the `acer-predator-turbo-and-rgb` kernel driver family), which proves the ACPI path works on Linux — and shows the fallback if the chardev route is insufficient: a tiny out-of-tree kernel driver.
- The HID device (0x1025:0x174B, 65-byte feature reports) is reachable via `/dev/hidraw` (`HIDIOCSFEATURE`/`HIDIOCGFEATURE`) — same reports, same `usage_mode_report` encoding.
- The smart-charge write semantics discovered in ticket 16 are transport-independent: a truthy return is not proof of effect; the single-pair readback match (`uFunctionList & 2`, `uFunctionStatus[1]`) gates success. The Linux port must keep readback-verified writes.

**Status:** resolved

**Blocked by:** None (design/effort ticket; no code dependency on an open ticket)

**Checklist (skeleton — split into tickets once the design questions are resolved):**

- [ ] Transport: `src/mi.rs` replaced by a Linux WMI transport — userspace WMI chardev ioctl (`WMI_IOCTL_EXEC_METHOD`) if the Acer GUID's methods are callable that way, else a minimal kernel driver. `WmiAdapter`/`SmartChargeAdapter` public APIs and circuit breakers unchanged (same seam as ticket 16).
- [ ] HID: `src/hid.rs` transport swapped to hidraw; report bytes unchanged.
- [ ] Power: `src/power.rs` rebuilt on sysfs/`cpupower`/`power-profiles-daemon` — the plan table (CPU min/max + boost per profile) maps to per-profile governor/boost settings; active-plan readback analog; nothing spawns external processes (design decision 7 holds: in-process sysfs writes and libc calls only).
- [ ] Power state: `src/power_state.rs` on `/sys/class/power_supply/` (AC0 online, capacity).
- [ ] Tray + hotkey: GTK/iced (+ ksni-style tray lib) and global-hotkey registration; `src/tray.rs`/`src/hotkey.rs` API shapes kept so `main.rs`/`app.rs` barely change.
- [ ] Lifecycle: logon auto-start via systemd user unit / XDG autostart instead of the scheduled task; single-instance mutex → lock file or D-Bus name.
- [ ] Elevation: root-owned systemd service (or polkit rule) — **the architectural decision**: split the app into a root service (hardware control) + user tray client over D-Bus, vs. a single root-run UI. Today the exe is one elevated process.
- [ ] Config/state/log: unchanged (`nitro-tray.toml`/`nitro-tray.state.toml` beside the exe).
- [ ] Verification: dual-boot Linux on the AN16S-61; replay the ticket-16 verification script — `probe_wmi`/`probe_charge`/`probe_mi` equivalents (WMI profile write+readback, cap ON/OFF/ON with readback match, HID feature write, fan auto) plus a clean startup log.

**Design questions:**

- Transport: Linux WMI chardev ioctl vs. small out-of-tree kernel driver (the chardev path is pure userspace — preferred — but method execution needs the device to expose a "WMI device driver" interface; if Acer's GUID lacks that, the driver route is required).
- Elevation model: root service + D-Bus client (clean, standard) vs. polkit-transacted single process vs. root-owned tray app (simple, ugly).
- GUI toolkit: iced/egui (Rust-native, no GTK dependency) vs. gtk-rs (native look, heavier build deps) — affects the whole tray/hotkey work.
- Power backend: power-profiles-daemon (user-friendly, but only balances performance/power-saver) vs. direct sysfs governor+boost writes (exact, matches the plan table 1:1) — or both, with the plan table as the source of truth.
- Scope: does the port target only the AN16S-61 (single-machine, like the Windows app) or general Acer Nitro support?

**Out of scope (unchanged from spec.md):** no installer, no auto-update, no interpreter spawning, fan is auto-only.

## Comments

2026-08-08: Filed from the feasibility discussion. The port reuses everything OS-independent (policy engine, config, state machine, opcode tables, readback-verified write logic) and rebuilds five OS-specific seams: WMI transport, HID transport, power layer, tray/hotkey, lifecycle/elevation. The MI-transport lessons (instance-bound invocation, readback-verified writes, breaker + recovery loop) carry over as adapter-level requirements, not Windows-specific knowledge.

2026-08-08: Moved from the nitro-tray feature (was ticket 17) to this feature dir on the tracker; renumbered to 01. Machine-specific firmware evidence and decisions live in `docs/firmware-notes.md` (the nitro-tray feature tickets were deleted when the notes were distilled).

## Answer (2026-08-08)

**This Answer supersedes the design-question and checklist premises above.** Implementation tickets: 02–08.

- **Transport:** own minimal GPL-2.0 DKMS kernel module + misc chardev. The chardev-ioctl premise is **disproven** — mainline Linux has no generic userspace WMI (`WMI_IOCTL_EXEC_METHOD` does not exist); ACPI-WMI methods execute in-kernel only. BatteryControl GUID to be discovered from the Linuwu-Sense source and `/sys/bus/wmi/devices`. → ticket 03
- **Elevation:** single unprivileged process + one-time root setup (module install, udev rules: chardev group access, hidraw VID rule, cpufreq attr chmod). No root service, no D-Bus split, no IPC.
- **HID:** `/dev/hidraw`, same 65-byte reports, VID 0x1025 discovery. On the AN16S-61 the device is I2C-HID; RGB is a separate ENE K5130 (out of scope). Usage-mode kept best-effort, probe-first, degrade on silent-ignore. → ticket 04
- **Power backend:** direct in-process sysfs (governor/EPP/boost per profile, plan table 1:1); no external processes (decision 7); ppd conflict detect-and-warn; EACCES degrades. → ticket 05
- **Power state:** `/sys/class/power_supply` AC0 online + capacity. → ticket 06
- **Tray/UI:** pure Rust — ksni (StatusNotifier), notify-rust, global-hotkey (X11; Wayland degrades); no GUI toolkit, no C build deps. → ticket 07
- **Lifecycle:** XDG autostart `.desktop` toggled from the tray; flock single-instance; XDG config/state/log paths.
- **Scope:** AN16S-61 only, tables as data.
- **Sequencing:** ticket 02 first, implemented and verified on Windows before the reboot; then 03–05 in any order on Linux (probe-first), 06, 07, 08 last (ready-for-human).
