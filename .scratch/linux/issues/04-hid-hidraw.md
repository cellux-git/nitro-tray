# 04 — HID transport on hidraw

**What to build:** Usage-mode control on Linux via `/dev/hidraw`: the HidTransport seam implemented over `HIDIOCSFEATURE`/`HIDIOCGFEATURE` with the same 65-byte feature reports and 4×750 ms retry behavior, device discovery by VID 0x1025 across hidraw nodes, and a udev rule for non-root access. On-device probe writes the usage-mode report and checks the raw echo. Known device facts: on the AN16S-61, 0x1025:0x174B enumerates as I2C-HID (keyboard input); RGB is a separate ENE K5130 device and out of scope. If the device silently accepts but ignores the write (the documented I2C-HID failure class), the adapter degrades — logs and skips — per the never-terminal policy, never crashing the app.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Probe on the AN16S-61: usage-mode write + echo succeeds, or silent-ignore is documented and the adapter degrades cleanly
- [ ] Discovery finds the correct hidraw node on the machine
- [ ] Degrade path exercised: device absent → adapter unavailable, app continues, recovery loop attempts reconnect
- [ ] Wire-shape parity with the Windows encoding (same report bytes, same retries)
