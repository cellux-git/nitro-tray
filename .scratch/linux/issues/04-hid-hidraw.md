# 04 — HID transport on hidraw

**What to build:** Usage-mode control on Linux via `/dev/hidraw`: the HidTransport seam implemented over `HIDIOCSFEATURE`/`HIDIOCGFEATURE` with the same 65-byte feature reports and 4×750 ms retry behavior, device discovery by VID 0x1025 across hidraw nodes, and a udev rule for non-root access. On-device probe writes the usage-mode report and checks the raw echo. Known device facts: on the AN16S-61, 0x1025:0x174B enumerates as I2C-HID (keyboard input); RGB is a separate ENE K5130 device and out of scope. If the device silently accepts but ignores the write (the documented I2C-HID failure class), the adapter degrades — logs and skips — per the never-terminal policy, never crashing the app.

**Blocked by:** 02, 09 (candidate 5: platform files)

**Status:** ready-for-agent

- [ ] Probe on the AN16S-61: usage-mode write + echo succeeds, or silent-ignore is documented and the adapter degrades cleanly
- [ ] Discovery finds the correct hidraw node on the machine
- [ ] Degrade path exercised: device absent → adapter unavailable, app continues, recovery loop attempts reconnect
- [ ] Wire-shape parity with the Windows encoding (same report bytes, same retries)

## Comments

2026-08-08: Adjusted for ticket 09 (done first). After candidate 09-5 the hidraw transport + discovery land in the platform file (`hid/linux.rs`), not inside hid.rs. Note: the constructor collapse (09-2) applies to the MI seam's `connect()`; HID discovery is inherently platform-specific, so `HidAdapter::open()` stays per-platform. Related gap surfaced by the review (not caused by 09): `AppCore::reconnect_unavailable` reconnects wmi + charge only — the HID adapter is never reconnected, so this ticket's "recovery loop attempts reconnect" needs either a core change (reconnect HID too) or rewording; decide in 09's grilling.
