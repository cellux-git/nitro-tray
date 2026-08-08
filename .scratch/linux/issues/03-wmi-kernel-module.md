# 03 — WMI kernel module + Linux transport

**What to build:** ACPI-WMI control of the AN16S-61 on Linux: platform profile, fan auto, keyboard-backlight-off, and smart-charge cap with readback-verified writes. A small GPL-2.0 DKMS kernel module exposes a misc chardev for exactly the Acer GamingFunction methods and the BatteryControl methods, using the same GUIDs and method encodings the Windows app drives (verified in `docs/firmware-notes.md`). The Linux transport implements the existing MiTransport seam in front of the chardev, preserving instance-bound invocation, the single-pair readback gate for smart charge (`uFunctionList`/`uFunctionStatus` match), and circuit-breaker semantics. The module is required: mainline Linux has no generic userspace WMI. The GamingFunction GUID is documented in ecosystem sources (acer-wmi/facer); the BatteryControl GUID must be discovered from the Linuwu-Sense source and a `/sys/bus/wmi/devices` dump. A udev rule grants the app's group chardev access so no root process is needed at runtime (elevation decision: single unprivileged process).

**Blocked by:** 02, 09 (candidates 2 and 5: constructor collapse + platform files)

**Status:** ready-for-agent

- [ ] BatteryControl GUID identified and cross-checked against Windows-verified behavior
- [ ] Module builds via DKMS on Mint, loads at boot, exposes the chardev with group access via udev
- [ ] On-device probes pass: profile write + readback match; charge cap ON/OFF/ON with single-pair readback match; fan auto; keyboard backlight off
- [ ] Linux transport satisfies the adapter seam contract (instance-bound, readback-verified, breaker + recovery never terminal)

## Comments

2026-08-08: Adjusted for ticket 09 (done first). After candidate 09-2 the per-seam constructor impls are shared and call the trait's `connect()` — this ticket no longer edits `wmi.rs`/`charge.rs`, only the transport. After candidate 09-5 the transport lands in the platform file (`mi/linux.rs`), not inside mi.rs. Added requirement: the chardev's errno values must map into the seam's `MiError` so `map_mi` still classifies availability (`EACCES`/`ENODEV` → `NotAvailable`), or the seam error type gains an explicit errno variant — decide in 09's grilling.
