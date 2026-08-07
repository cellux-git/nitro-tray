# 05 — Acer WMI adapter

**What to build:** in-process raw COM/WMI control of the Acer gaming firmware: set and read back the platform profile (quiet 0, balanced 1, performance 4, eco 6) and set/read fan behavior to auto, against `AcerGamingFunction`. All opcode/method encodings match the proven AeroForge tables and carry unit tests. Readback is verified on the target machine with a probe, because the interface cannot be meaningfully mocked.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] Platform profile can be set and read back via in-process COM/WMI (write + readback round trip).
- [ ] Fan behavior can be set to auto (0x00410009) and read back.
- [ ] Opcode/method encoding unit tests cover the profile and fan tables (prior art: AeroForge's encoding tests).
- [ ] On-device probe verification exercises real WMI writes and readbacks (elevated, test-time only; the app itself never spawns processes).
- [ ] No PowerShell or other interpreter is ever spawned; no CIM/PowerShell fallback path exists.
