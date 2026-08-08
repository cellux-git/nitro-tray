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

- WMI transport → Linux WMI chardev ioctl (`WMI_IOCTL_EXEC_METHOD`) or a minimal
  kernel driver.
- HID transport → `/dev/hidraw` (same 65-byte feature reports).
- Power layer → sysfs / `cpupower` / power-profiles-daemon (no external
  processes, per design decision 7).
- Power state → `/sys/class/power_supply/`.
- Tray + hotkey → GTK/iced (+ ksni-style tray), global-hotkey registration.
- Lifecycle/elevation → systemd/XDG autostart; root service + user client over
  D-Bus vs. single root process (the elevation decision).

## Design questions

Transport (chardev vs. kernel driver), elevation model, GUI toolkit, power
backend, and scope (AN16S-61 only vs. general Acer Nitro) — recorded in
`issues/01-linux-port.md`.

## Out of scope

No installer, no auto-update, no interpreter spawning, fan is auto-only.

## Issues

- `issues/01-linux-port.md` — the design/effort ticket (filed 2026-08-08,
  originally 17 in the nitro-tray feature; moved here 2026-08-08). To be split
  into implementation tickets once its design questions are resolved.
