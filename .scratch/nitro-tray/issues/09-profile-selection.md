# 09 — Profile selection from tray

**What to build:** the tray menu lists only the profiles valid for the current power state (AC: quiet/balanced/performance; battery: eco/balanced). Picking a profile applies it immediately — firmware profile, HID usage mode, fan auto, and the matching Nitro plan — and the pick persists per power state, never overridden by a timer. Eco is detected at runtime: the first eco attempt writes firmware profile 6 and reads back; if the firmware rejects it, the eco entry is disabled (and re-evaluated later) instead of silently failing. When the Acer WMI interface is down, plan switching is still offered.

**Blocked by:** 03 — Policy engine; 04 — Power API wrapper; 05 — Acer WMI adapter; 06 — Acer HID adapter; 08 — Tray effective state.

**Status:** ready-for-agent

- [ ] Menu shows only the profiles valid for the current power state.
- [ ] Selecting a profile applies it immediately: firmware profile + HID usage mode + fan auto + active Nitro plan.
- [ ] Picks persist per power state and are not overridden on a timer.
- [ ] Eco runtime detection: first eco attempt writes firmware profile 6 and reads back; on rejection the eco entry is disabled, re-evaluated later.
- [ ] When the Acer WMI interface is unreachable, the plan switch for the chosen profile is still offered and applied.
