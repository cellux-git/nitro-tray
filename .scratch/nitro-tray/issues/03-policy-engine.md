# 03 — Policy engine

**What to build:** the pure decision logic of the app — the one seam the whole design is built to test. Given the current power state, the config, and the user's persisted profile picks, it computes the exact intended target state: firmware profile value, HID usage mode, fan behavior (always auto), smart-charge state, and the target Nitro plan. It owns the AC and battery profile lists, forward-wrap cycling, eco acceptance/fallback, and per-power-state persistence of manual picks.

**Blocked by:** 02 — Config parsing (the engine consumes config types and defaults).

**Status:** ready-for-agent

- [ ] AC profile list is quiet/balanced/performance; battery list is eco/balanced; each list is position-bound for the menu.
- [ ] Defaults come from config (AC balanced, battery eco); manual picks persist and are per power state — choosing one for battery does not affect AC.
- [ ] Intended state maps every profile to: firmware profile value (quiet 0, balanced 1, performance 4; eco 6 when accepted), HID usage mode, fan = auto, and the matching Nitro plan.
- [ ] Forward-wrap cycling moves through the current power state's list in order and wraps at the end.
- [ ] Eco acceptance logic: when the machine accepts eco, intended state uses firmware profile 6; when it does not, the eco entry is disabled rather than silently failing.
- [ ] Reapply (off by default) re-asserts only firmware items and never includes the active plan in its intended state.
- [ ] Unit tests assert exact intended targets for representative inputs (power state × config × picks), covering defaults, per-power-state persistence, cycling order and wrap, and eco acceptance vs. disabled.
