# 06 — Acer HID adapter

**What to build:** writes to the Acer HID device (vendor 0x1025) to set the system usage mode (Quiet/Normal/Performance) via feature reports, matching each profile's usage mode. A HID write failure must never take the app down: it is logged, and the WMI profile plus the Windows plan are still applied. Verified on-device with a probe.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] Usage mode feature reports are written on the vendor 0x1025 device for Quiet, Normal, and Performance.
- [ ] Each profile maps to its usage mode (quiet → Quiet, balanced → Normal, performance → Performance, eco → Quiet).
- [ ] A HID write failure is non-fatal: logged, and WMI profile + plan application continue.
- [ ] On-device probe verification exercises real HID feature-report writes.
