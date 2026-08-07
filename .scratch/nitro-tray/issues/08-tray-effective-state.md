# 08 — Tray dashboard (effective state)

**What to build:** the tray shows what the machine is *actually* doing, read back from hardware and OS rather than from intent: AC/battery, battery %, active profile, active Windows plan, and smart-charge state, in the menu and tooltip. A power-state source (system power status + power notifications + slow poll fallback) feeds it. If the Acer WMI interface is unreachable, the tray shows a degraded "Hardware unavailable" state with the profile and smart-charge items greyed out, so the user understands why control is missing.

**Blocked by:** 01 — Lifecycle skeleton; 04 — Power API wrapper; 05 — Acer WMI adapter; 06 — Acer HID adapter; 07 — Smart charge adapter.

**Status:** ready-for-agent

- [ ] Menu and tooltip display read-back values: AC/battery, battery %, active profile, active Windows plan, smart-charge state — all read from hardware/OS, not app intent.
- [ ] Power state is sourced from system power status, updated by power notifications, with a slow poll fallback.
- [ ] Left-click opens the menu.
- [ ] When the Acer WMI interface is unreachable, the tray shows a degraded "Hardware unavailable" state with profile and smart-charge items greyed out.
