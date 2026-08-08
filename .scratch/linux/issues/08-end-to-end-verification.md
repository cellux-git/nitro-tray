# 08 — End-to-end verification on the AN16S-61

**What to build:** Final acceptance on the machine: replay the Windows ticket-16 verification script as Linux equivalents — WMI profile write + readback, charge cap ON/OFF/ON with readback match, HID feature write, fan auto, power-state transitions — plus a clean startup log, the full enforcement matrix (startup, AC/DC switch, resume, reapply tick, recovery tick, readback reassert), and the degradation checks (adapter missing or tripped → warn, never crash).

**Blocked by:** 07

**Status:** ready-for-human

- [ ] All probes pass as in the Windows script
- [ ] Enforcement matrix passes: each occasion applies the intended state, verified by readback
- [ ] Degradation checks pass: removing an adapter path degrades with warnings; recovery reconnects
- [ ] Clean startup log on a fresh boot

## Comments

2026-08-08: No adjustment needed for ticket 09 (done first) — the verification script (WMI profile write + readback, charge cap ON/OFF/ON, HID feature write, fan auto, power-state transitions) is seam-shaped, not vocabulary-shaped; the only wording that shifts is "verified by readback" now meaning tuning-state readback (candidate 1) where the plan was involved.
