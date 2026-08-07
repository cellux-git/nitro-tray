# 02 — Config parsing

**What to build:** an optional config file beside the exe (`nitro-tray.toml`) that adjusts defaults and the hotkey, while the app remains fully usable with no config file at all. Every documented key has a baked-in default; a partial file fills the gaps; invalid values are rejected gracefully so the app still starts.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] With no config file, the app runs on baked-in defaults: smart-charge on, AC profile balanced, battery profile eco, auto-switch on, reapply off, reapply interval 30s, hotkey ctrl-alt-p.
- [ ] A partial config file leaves unspecified keys at their defaults.
- [ ] Invalid config values are rejected gracefully (clear diagnostic, app still starts with defaults).
- [ ] Config is read at startup; changing it takes effect on restart.
- [ ] Config parsing covered by unit tests (no file, partial file, invalid values).
