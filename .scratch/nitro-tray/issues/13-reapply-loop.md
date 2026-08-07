# 13 — Reapply loop

**What to build:** an optional periodic re-assertion loop (off by default; interval 30s, both configurable) that re-asserts only the firmware-level items — WMI profile, HID mode, fan auto, smart-charge state — and never the active Windows plan, so manually chosen plans are respected even when enabled.

**Blocked by:** 02 — Config parsing (reapply keys); 09 — Profile selection from tray (shared firmware apply path).

**Status:** ready-for-agent

- [ ] The loop is off by default and enabled via config with a configurable interval (default 30s).
- [ ] When enabled, it periodically re-asserts WMI profile, HID mode, fan auto, and smart-charge state.
- [ ] It never re-asserts the active Windows plan, so manual plan edits are respected.
