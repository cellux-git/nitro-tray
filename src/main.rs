//! Binary entry point: arg parsing, single-instance mutex, logon-task
//! install/uninstall, config + app core startup, tray creation, and the
//! message pump that dispatches tray events.

#![windows_subsystem = "windows"]

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::mpsc;

use winapi::um::libloaderapi::GetModuleFileNameW;
use windows_sys::core::w;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PostQuitMessage, MSG};

use nitro_tray::app::AppCore;
use nitro_tray::config;
use nitro_tray::log;
use nitro_tray::policy::{PowerState, AC_PROFILES, BATTERY_PROFILES};
use nitro_tray::power_state;
use nitro_tray::task;
use nitro_tray::tray::{Tray, TrayEvent, TrayView};

fn main() {
    let debug_log = std::env::args_os().skip(1).any(|a| a == "--log");
    let uninstall = std::env::args_os().skip(1).any(|a| a == "--uninstall");

    let Some(exe_path) = executable_path() else {
        return; // cannot resolve our own location; nothing sensible to do
    };
    let exe_dir = exe_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    log::init(&exe_dir);
    if debug_log {
        log::set_enabled(true);
    }
    log::info("nitro-tray starting");

    if uninstall {
        log::info("uninstall requested");
        match task::uninstall_logon_task() {
            Ok(()) => log::info("scheduled task removed"),
            Err(e) => log::error(&format!("failed to remove scheduled task: {e:?}")),
        }
        return;
    }

    let _single_instance = match acquire_single_instance() {
        Ok(handle) => handle,
        Err(()) => {
            log::info("another instance is already running; exiting");
            return;
        }
    };

    let config = config::load(&exe_dir);
    let mut app = AppCore::new(config, &exe_dir);

    if let Err(e) = task::install_logon_task(&exe_path) {
        log::warn(&format!("failed to install logon task: {e:?}"));
    }

    let (event_tx, event_rx) = mpsc::channel();
    let tray = match Tray::create(event_tx) {
        Ok(tray) => tray,
        Err(e) => {
            log::error(&format!("failed to create tray: {e:?}"));
            return;
        }
    };

    let snapshot = power_state::read();
    let view = TrayView {
        power: snapshot.state,
        percent: snapshot.percent,
        profile: None,
        profiles: match snapshot.state {
            PowerState::Ac => AC_PROFILES.to_vec(),
            PowerState::Battery => BATTERY_PROFILES.to_vec(),
        },
        profiles_greyed: false,
        smart_charge: None,
        smart_charge_greyed: false,
        plan: None,
        degraded: false,
    };
    if let Err(e) = tray.update(&view) {
        log::warn(&format!("failed to update tray view: {e:?}"));
    }

    message_pump(&tray, &mut app, &event_rx);
    log::info("nitro-tray exiting");
}

/// Full path of the running executable via `GetModuleFileNameW`.
fn executable_path() -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let len = unsafe { GetModuleFileNameW(ptr::null_mut(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if len == 0 {
        log::error("GetModuleFileNameW failed");
        return None;
    }
    buffer.truncate(len as usize);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

/// Acquire the `Local\NitroTray` named mutex. A second instance finds the
/// mutex already existing and returns `Err(())`; the handle is kept alive by
/// the caller for the process lifetime.
fn acquire_single_instance() -> Result<HANDLE, ()> {
    let handle = unsafe { CreateMutexW(ptr::null(), 0, w!("Local\\NitroTray")) };
    if handle.is_null() {
        log::error("failed to create single-instance mutex");
        return Err(());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err(());
    }
    Ok(handle)
}

/// GetMessageW/DispatchMessageW loop; drains the tray event channel after
/// each message and dispatches events through `handle_event`.
fn message_pump(tray: &Tray, app: &mut AppCore, events: &mpsc::Receiver<TrayEvent>) {
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) };
        if ret == 0 {
            break; // WM_QUIT
        }
        if ret == -1 {
            log::error("message pump: GetMessageW failed");
            break;
        }
        unsafe { DispatchMessageW(&msg) };
        while let Ok(ev) = events.try_recv() {
            handle_event(app, tray, ev);
        }
    }
}

/// One flat event dispatch; later tickets add match arms here
/// (SelectProfile/ToggleSmartCharge: ticket 10, PowerChanged/Resume: ticket 12).
fn handle_event(app: &mut AppCore, tray: &Tray, ev: TrayEvent) {
    match ev {
        TrayEvent::Quit => {
            log::info("quit requested");
            // later tickets: unregister hotkey (11), destroy tray icon (08)
            unsafe { PostQuitMessage(0) };
        }
        TrayEvent::SelectProfile(_profile) => {
            // ticket 10: apply the picked profile for the current power state
        }
        TrayEvent::ToggleSmartCharge => {
            // ticket 10: toggle smart charge
        }
        TrayEvent::PowerChanged => {
            // ticket 12: enforce on power transitions
        }
        TrayEvent::Resume => {
            // ticket 12: enforce on resume
        }
    }
    let _ = (app, tray);
}
