# 06 — Power state on sysfs

**What to build:** Power-state snapshots (AC/DC + capacity) read from `/sys/class/power_supply` (AC0 online + capacity), with the pure mapping from sysfs values to the snapshot shape unit-tested — the Linux replacement for the Windows power-status read, so the crate's tests are green on Linux.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Mapping tests green
- [ ] On-device check: unplug/plug transitions report AC/DC and capacity correctly
