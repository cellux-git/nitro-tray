# 07 — Smart charge adapter

**What to build:** in-process control of the 80% charge cap via the `BatteryControl` WMI health-status toggle, using the AMD direct-trust write path for this SKU class, with readback of the current state. No interpreter is spawned. Verified on-device with a probe.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] Smart charge (80% cap) can be toggled on and off via in-process COM/WMI `BatteryControl`.
- [ ] The current smart-charge state can be read back.
- [ ] Uses the AMD direct-trust write path for the target SKU class.
- [ ] On-device probe verification exercises a real toggle + readback round trip.
- [ ] No PowerShell or other interpreter is ever spawned.
