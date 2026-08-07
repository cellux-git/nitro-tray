# 12 — Enforcement on startup, power transitions, and resume

**What to build:** the app actively enforces a coherent power state at every important moment: at startup (the four Nitro plans are ensured to exist — recreated if deleted — and the profile for the current power state, fan auto, smart-charge default, and active plan are applied), on every AC↔battery transition (the profile for the new power state is applied silently, per config and persisted picks), and on resume/wake (re-enforced after firmware or OS resets). This must all keep working with Acer's user-mode services stopped or uninstalled.

**Blocked by:** 08 — Tray effective state (power notifications); 09 — Profile selection from tray (apply path).

**Status:** ready-for-agent

- [ ] At startup: the four Nitro plans are ensured (a deleted plan is recreated), and the intended state for the current power state is applied (profile, fan auto, smart charge, active plan).
- [ ] On unplug: the battery profile is applied automatically (eco by default).
- [ ] On plug-in: the AC profile is applied automatically (balanced by default).
- [ ] Automatic transitions are silent (no notifications).
- [ ] On resume/wake: the intended state is re-enforced.
- [ ] Everything works with Acer's user-mode services stopped or uninstalled (verify on device).
