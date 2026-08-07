# 02 — Config parsing

**What to build:** an optional config file beside the exe (`nitro-tray.toml`) that adjusts defaults and the hotkey, while the app remains fully usable with no config file at all. Every documented key has a baked-in default; a partial file fills the gaps; invalid values are rejected gracefully so the app still starts.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [x] With no config file, the app runs on baked-in defaults: smart-charge on, AC profile balanced, battery profile eco, auto-switch on, reapply off, reapply interval 30s, hotkey ctrl-alt-p.
- [x] A partial config file leaves unspecified keys at their defaults.
- [x] Invalid config values are rejected gracefully (clear diagnostic, app still starts with defaults).
- [x] Config is read at startup; changing it takes effect on restart.
- [x] Config parsing covered by unit tests (no file, partial file, invalid values).

## Comments

- `src/config.rs` implemented: `parse` uses `toml::from_str::<toml::Value>` (no serde derive) so partial files work; per-key extraction with `as_bool`/`as_str`/`as_integer`; unknown keys ignored; invalid values keep the default and append a diagnostic (`config: bad value for '<key>' (<reason>), using default <default>`); malformed TOML yields defaults + one diagnostic. `reapply_interval_secs < 1` rejected (must be >= 1) with default 30.
- `load` reads `exe_dir/nitro-tray.toml`; missing file -> defaults (no diagnostics); other read errors -> defaults + `log::warn` diagnostic.
- Pinned API unchanged: `CONFIG_FILE_NAME`, `Config` (fields + `Default`), `parse`, `load`.
- 12 unit tests, all passing: `cargo test --lib config::` green; `cargo build` clean (only pre-existing dead-code warning in `src/policy.rs`, another ticket's file).
- Startup wiring (calling `load` with the exe dir) is owned by `src/main.rs` (tickets 01/09), not this ticket.
- Test coverage note: the "read error other than NotFound" path logs via `log::warn`, which is still a `todo!()` stub owned by ticket 01 and panics if called; not unit-testable until ticket 01 lands (verified: a test hitting that path panicked in `src/log.rs`, so it was dropped). The code path is in place.
