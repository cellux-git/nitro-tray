//! Binary entry point: platform dispatcher (linux-port ticket 02). Windows
//! runs the full tray app in `windows_main`; Linux runs the stub entrypoint
//! in `linux_main` until tickets 03–07 land the real backends. The shared
//! `#![windows_subsystem]` only applies on Windows (no console there).

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows_main;

#[cfg(target_os = "linux")]
mod linux_main;

#[cfg(windows)]
fn main() {
    windows_main::run();
}

#[cfg(target_os = "linux")]
fn main() {
    linux_main::run();
}
