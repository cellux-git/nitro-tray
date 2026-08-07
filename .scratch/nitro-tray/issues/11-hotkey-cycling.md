# 11 — Hotkey cycling

**What to build:** a global hotkey (Ctrl+Alt+P by default, configurable via the config file) cycles forward through the current power state's profile list, wrapping at the end, and applies the resulting profile. A brief notification confirms the new profile. Automatic power-transition switching stays silent — the hotkey is the only notification path.

**Blocked by:** 02 — Config parsing; 03 — Policy engine; 09 — Profile selection from tray (reuses the apply path).

**Status:** ready-for-agent

- [ ] The hotkey cycles forward through the current power state's list and wraps.
- [ ] The hotkey combination is configurable and the default is ctrl-alt-p.
- [ ] Pressing the hotkey applies the cycled profile and shows a brief notification.
- [ ] Automatic switching on power transitions produces no notification.
