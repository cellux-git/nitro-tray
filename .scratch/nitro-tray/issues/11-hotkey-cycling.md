# 11 — Hotkey cycling

**What to build:** a global hotkey (Ctrl+Alt+P by default, configurable via the config file) cycles forward through the current power state's profile list, wrapping at the end, and applies the resulting profile. A brief notification confirms the new profile. Automatic power-transition switching stays silent — the hotkey is the only notification path.

**Blocked by:** 02 — Config parsing; 03 — Policy engine; 09 — Profile selection from tray (reuses the apply path).

**Status:** ready-for-agent

- [x] The hotkey cycles forward through the current power state's list and wraps.
- [x] The hotkey combination is configurable and the default is ctrl-alt-p.
- [x] Pressing the hotkey applies the cycled profile and shows a brief notification.
- [x] Automatic switching on power transitions produces no notification.

## Comments

Implemented 2026-08-07 (agent). `src/hotkey.rs` (parse_spec + Hotkey::register/id +
Drop-unregister, `HOTKEY_ID = 0x4E54`), `src/main.rs` (registration after tray
creation, `TrayEvent::HotkeyPressed` arm -> `cycle_profile` + tray update +
balloon via `tray.notify`), and the one additive change in `src/tray.rs`
(HotkeyPressed variant + WM_HOTKEY arm in the WndProc). 12 unit tests for
parse_spec (default, custom, modifiers alone, unknown key, empty/blank,
f-keys, digits, uppercase letters, named keys, multiple key tokens).

Item 4 verified by grep: `tray.notify` is defined once (src/tray.rs) and
called from exactly one place — the hotkey arm in main.rs. Startup/transition/
resume paths contain no notify calls.

Needs on-device verification: actual `RegisterHotKey` success, that `WM_HOTKEY`
reaches the tray WndProc with the registered id and cycles, that the balloon
shows, and the exact KeyBoardAndMouse import location (uses
`Win32::UI::Input::KeyboardAndMouse`, not WindowsAndMessaging, in
windows-sys 0.61.2).
