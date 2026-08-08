# 06 — Power state on sysfs

**What to build:** Power-state snapshots (AC/DC + capacity) read from `/sys/class/power_supply` (AC0 online + capacity), with the pure mapping from sysfs values to the snapshot shape unit-tested — the Linux replacement for the Windows power-status read, so the crate's tests are green on Linux.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Mapping tests green
- [ ] On-device check: unplug/plug transitions report AC/DC and capacity correctly

## Comments

2026-08-08: Adjusted for ticket 09 (done first) — minor. The pure mapping tests landed with ticket 02 and stay; candidate 09-4 additionally gives the reader glue (`read_sysfs_value`'s fall-through logic, untested so far) a directory seam so it is unit-testable against fixture temp dirs on both platforms. No change to the on-device check. Not blocked by 09: the sysfs reader itself already exists; 09-4 only strengthens its tests.
