# Linux — native port of nitro-tray

Spec for porting nitro-tray to Linux (x64), targeting the user's own AN16S-61
(dual boot). The machine-controlling knowledge built through the Windows
tickets lives on the *firmware* side and is OS-independent; the port reuses it
and replaces the OS-specific seams.

## Reused as-is

- Policy engine, config, state-file handling, opcode tables, smart-charge
  readback-verified write semantics.
- The adapter-level requirements: instance-bound invocation, readback-verified
  writes, circuit breaker + recovery loop (never terminal).

## Rebuilt per seam

- WMI transport → own GPL-2.0 DKMS kernel module (misc chardev); mainline Linux
  has no generic userspace WMI.
- HID transport → `/dev/hidraw` (same 65-byte feature reports).
- Power layer → sysfs / `cpupower` / power-profiles-daemon (no external
  processes, per design decision 7).
- Power state → `/sys/class/power_supply/`.
- Tray + hotkey → pure-Rust: ksni (StatusNotifier), notify-rust, global-hotkey (X11; Wayland degrades).
- Lifecycle/elevation → XDG autostart; single unprivileged process with
  one-time root setup (udev rules) — no root service.

## Design questions

Resolved 2026-08-08 — answers recorded in the Answer section of
`issues/01-linux-port.md`. The chardev-ioctl premise was disproven during
triage: mainline Linux has no generic userspace WMI API, so the WMI transport
is a kernel module.

## Out of scope

No installer, no auto-update, no interpreter spawning, fan is auto-only.

## Issues

- `issues/01-linux-port.md` — the design/effort ticket; resolved 2026-08-08,
  answers in its Answer section.
- `issues/02-platform-gate.md` — platform-gate + Linux stubs (Windows-first).
- `issues/03-wmi-kernel-module.md` — GPL-2.0 kernel module + Linux WMI transport.
- `issues/04-hid-hidraw.md` — HID transport on `/dev/hidraw`.
- `issues/05-power-sysfs.md` — power layer on sysfs.
- `issues/06-power-state-sysfs.md` — power state on `/sys/class/power_supply`.
- `issues/07-tray-hotkey-lifecycle.md` — tray/hotkey/lifecycle/paths.
- `issues/08-end-to-end-verification.md` — final on-device acceptance (ready-for-human).
- `issues/09-architecture-deepening.md` — platform-gate review candidates (deepen the plan seam, collapse transport constructors, extract the tray model, sysfs dir seam, rehome platform bodies, share entry-point boot).
