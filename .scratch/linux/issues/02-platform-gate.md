# 02 — Platform-gate the crate, stub Linux backends

**What to build:** The crate compiles and tests on both platforms. All Windows-only code is target-gated so nothing Windows-specific is built on Linux; on Linux the three transport seams and the power-state reader exist as stubs that report "unavailable", and a minimal entrypoint boots, initializes config/log, and exits cleanly — the app's never-terminal degrade philosophy applied to the platform itself. Windows behavior is untouched: the full existing test suite stays green, and the Linux compile is provable from Windows via `cargo check --target x86_64-unknown-linux-gnu` (rustup target add — no linker needed for check). The stale WMI-chardev premise in the spec and the design ticket is corrected to the verified fact: mainline Linux has no generic userspace WMI API, so the Linux WMI path is a kernel module (ticket 03), not a chardev ioctl.

**Blocked by:** None — can start immediately (design questions resolved in 01)

**Status:** ready-for-agent

- [x] All existing tests pass on Windows unchanged (176)
- [x] Windows-only dependencies and the manifest build step are target-gated; no Windows imports leak into the Linux build
- [x] `cargo check --target x86_64-unknown-linux-gnu` succeeds from Windows
- [x] Linux stubs: transport/plan seams report unavailable; power-state reads `/sys/class/power_supply`; minimal entrypoint boots and logs cleanly
- [ ] After reboot: `cargo test` green on Linux (core policy/config/state/log tests run; hardware adapters degrade)
- [x] Spec + design ticket corrected: no generic WMI chardev exists; Linux WMI transport is a kernel module

## Comments

2026-08-08: Implemented. Split per seam, platform bits in `#[cfg(windows)]`/`#[cfg(target_os = "linux")]` blocks inside the seam's own module (`mi`, `hid`, `power`, `power_state`); `MiConnection`/`RealHidTransport`/`PowerApi` keep their names on Linux so `AppCore`'s default generics and the entry points (`WmiAdapter::connect()`, `SmartChargeAdapter::connect()`, `HidAdapter::open()`) stay platform-agnostic — the Linux stubs report `NotAvailable`/`PowerError::Unavailable`/`NotFound`, and the Windows entry-point wiring shape runs unchanged (degrade-and-continue). `main.rs` is now a dispatcher over `windows_main.rs` (the moved tray app, byte-identical behavior) and `linux_main.rs` (boot config/log, wire stubs, run `on_startup` through the degrade path, exit cleanly). The six probe bins are Windows-only (wrapped in `#[cfg(windows)] mod probe` with stub mains), `tray`/`hotkey`/`task` modules are `#[cfg(windows)]`, `winapi`/`windows-sys`/`embed-manifest` are target-gated in Cargo.toml, and build.rs keys manifest embedding off `CARGO_CFG_TARGET_OS`. Verified from Windows: 176 tests green, `cargo clippy` clean on both `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` (all targets, including `--tests`). The remaining checklist item (post-reboot `cargo test` on Linux) is the on-device acceptance.

2026-08-08: Code review fixes: build.rs manifest block is now `#[cfg(windows)]`-gated as well as target-checked — `[target.'cfg(windows)'.build-dependencies]` matches the HOST, so a native Linux build would otherwise fail to resolve `embed_manifest`; Linux-side cfg gates unified on `#[cfg(target_os = "linux")]` (was a mix of `not(windows)`); the two identical Linux-stub error literals in `mi.rs` collapsed into one `linux_unavailable` helper; `read_sysfs_value` now falls through to the next matching supply when a file is unreadable/unparseable.

**Status:** ready-for-human (Windows side verified; the Linux-side `cargo test` needs the reboot)
