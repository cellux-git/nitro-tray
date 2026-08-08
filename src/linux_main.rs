//! Linux binary entry point (linux-port ticket 02): boots config + log,
//! wires the transport seams through the shared `wiring::connect_adapters`
//! helper — all "unavailable" stubs until tickets 03–05 land their real
//! backends — and runs the startup occasion through the app's never-terminal
//! degrade path, then exits cleanly. The real Linux lifecycle (tray, hotkey,
//! timers, XDG paths, autostart) lands in ticket 07.

use std::path::PathBuf;

use nitro_tray::app::AppCore;
use nitro_tray::config;
use nitro_tray::log;
use nitro_tray::power::PowerApi;
use nitro_tray::wiring;

pub fn run() {
    let exe_dir = executable_dir();
    let config = config::load(&exe_dir);

    log::init(&exe_dir);
    if config.log {
        log::set_enabled(true);
    }
    log::info("nitro-tray starting (linux)");

    log::install_panic_hook();

    // The transport seams are stubs on Linux (tickets 03–05); the shared
    // wiring helper connects them, so the core runs fully degraded — and
    // keeps running, exactly like the Windows fallback path. The recovery
    // loop's `M::connect()` (wired in ticket 07) will pick up the real
    // backends.
    let (wmi, charge, hid) = wiring::connect_adapters();
    let mut app: AppCore = AppCore::new(config, &exe_dir, wmi, charge, hid, PowerApi);
    log::info("app core initialized");

    // Startup occasion: with every seam unavailable this exercises the
    // degrade path (plans fail, firmware items skipped, smart charge
    // warned) and proves the boot sequence logs cleanly.
    app.on_startup();
    log::info("startup enforcement complete (degraded: Linux transport seams pending)");

    log::info("nitro-tray exiting cleanly (linux stub entrypoint, ticket 02)");
}

/// Directory beside the running executable (std's `current_exe`, no Win32).
fn executable_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}
