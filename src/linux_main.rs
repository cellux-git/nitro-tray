//! Linux binary entry point (linux-port ticket 02): boots config + log,
//! wires the transport seams — all "unavailable" stubs until tickets 03–05
//! land their real backends — and runs the startup occasion through the
//! app's never-terminal degrade path, then exits cleanly. The real Linux
//! lifecycle (tray, hotkey, timers, XDG paths, autostart) lands in ticket 07.

use std::path::PathBuf;

use nitro_tray::app::AppCore;
use nitro_tray::charge::SmartChargeAdapter;
use nitro_tray::config;
use nitro_tray::hid::HidAdapter;
use nitro_tray::log;
use nitro_tray::power::PowerApi;
use nitro_tray::wmi::WmiAdapter;

pub fn run() {
    let exe_dir = executable_dir();
    let config = config::load(&exe_dir);

    log::init(&exe_dir);
    if config.log {
        log::set_enabled(true);
    }
    log::info("nitro-tray starting (linux)");

    // Route panics into the log (with a backtrace) like the Windows entry
    // point; the GUI-less stub may still be launched from a session that
    // has no visible stderr.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error(format!("PANIC: {info}"));
        let backtrace = std::backtrace::Backtrace::capture();
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            log::error(format!("backtrace:\n{backtrace}"));
        }
        default_hook(info);
    }));

    // The transport seams are stubs on Linux (tickets 03–05): connect/open
    // report unavailable, so the core runs fully degraded — and keeps
    // running, exactly like the Windows fallback path. The recovery loop's
    // `M::connect()` (wired in ticket 07) will pick up the real backends.
    let wmi = match WmiAdapter::connect() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!(
                "wmi: adapter unavailable; running degraded: {err:?}"
            ));
            None
        }
    };
    let charge = match SmartChargeAdapter::connect() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!("charge: smart-charge adapter unavailable: {err:?}"));
            None
        }
    };
    let hid = match HidAdapter::open() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!("hid: usage-mode adapter unavailable: {err:?}"));
            None
        }
    };
    let mut app = AppCore::new(config, &exe_dir, wmi, charge, hid, PowerApi);
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
