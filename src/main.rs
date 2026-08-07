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

use nitro_tray::app::{apply_report_text, AppCore, ApplyReport};
use nitro_tray::config;
use nitro_tray::enforcement;
use nitro_tray::hotkey::Hotkey;
use nitro_tray::log;
use nitro_tray::policy::{PowerState, Profile, AC_PROFILES, BATTERY_PROFILES};
use nitro_tray::reapply;
use nitro_tray::recovery;
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

    let config = config::load(&exe_dir);

    log::init(&exe_dir);
    if debug_log || config.log {
        log::set_enabled(true);
    }
    log::info("nitro-tray starting");

    // Route panics into the log (with a backtrace) instead of dying silently
    // on stderr, which is invisible for a GUI-subsystem app.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error(format!("PANIC: {info}"));
        let backtrace = std::backtrace::Backtrace::capture();
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            log::error(format!("backtrace:\n{backtrace}"));
        }
        default_hook(info);
    }));

    if uninstall {
        log::info("uninstall requested");
        match task::uninstall_logon_task() {
            Ok(()) => log::info("scheduled task removed"),
            Err(e) => log::error(format!("failed to remove scheduled task: {e:?}")),
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
    log::info("single-instance mutex acquired");

    let config = config::load(&exe_dir);
    log::info("config loaded");
    let hotkey_spec = config.hotkey.clone();
    let reapply_cfg = config.clone();
    let mut app = AppCore::new(config, &exe_dir);
    log::info("app core initialized");

    if let Err(e) = task::install_logon_task(&exe_path) {
        log::warn(format!("failed to install logon task: {e:?}"));
    } else {
        log::info("logon task installed");
    }

    let (event_tx, event_rx) = mpsc::channel();
    let tray = match Tray::create(event_tx) {
        Ok(tray) => tray,
        Err(e) => {
            log::error(format!("failed to create tray: {e:?}"));
            fatal(format!("failed to create tray: {e:?}"));
            return;
        }
    };
    log::info("tray created");

    let view = view_from(&app);
    if let Err(e) = tray.update(&view) {
        log::warn(format!("failed to update tray view: {e:?}"));
    }
    log::info("tray view updated");

    // Kept alive for the process lifetime; `Drop` unregisters the hotkey.
    let _hotkey = match Hotkey::register(tray.hwnd(), &hotkey_spec) {
        Ok(hotkey) => {
            log::info(format!("hotkey registered: {hotkey_spec}"));
            Some(hotkey)
        }
        Err(err) => {
            log::warn(format!("hotkey: failed to register {hotkey_spec:?}: {err:?}"));
            None
        }
    };

    if reapply::enabled(&reapply_cfg) {
        let interval = reapply::interval_ms(&reapply_cfg);
        match tray.start_timer(reapply::TIMER_ID, interval) {
            Ok(()) => log::info(format!("reapply loop enabled; interval {interval} ms")),
            Err(err) => log::warn(format!("reapply: failed to arm timer: {err:?}")),
        }
    }

    // Recovery and the periodic readback are always armed — broken adapters
    // must recover and the tray view must refresh even when reapply is off.
    match tray.start_timer(recovery::TIMER_ID, recovery::INTERVAL_MS) {
        Ok(()) => log::info(format!("recovery loop armed; interval {} ms", recovery::INTERVAL_MS)),
        Err(err) => log::warn(format!("recovery: failed to arm timer: {err:?}")),
    }
    match tray.start_timer(recovery::READBACK_TIMER_ID, recovery::READBACK_INTERVAL_MS) {
        Ok(()) => log::info(format!(
            "readback loop armed; interval {} ms",
            recovery::READBACK_INTERVAL_MS
        )),
        Err(err) => log::warn(format!("recovery: failed to arm readback timer: {err:?}")),
    }

    enforcement::on_startup(&mut app);
    log::info("startup enforcement complete");

    log::info("entering message pump");
    message_pump(&tray, &mut app, &event_rx);
    log::info("nitro-tray exiting");
}

/// Show a message box for a fatal startup error (the GUI-subsystem app has no
/// console, so a silent `return` otherwise looks like the app just vanishes).
fn fatal(message: String) {
    let wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            std::ptr::null_mut(),
            wide.as_ptr(),
            w!("Nitro Tray"),
            0x10, // MB_ICONERROR
        );
    }
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

/// Build the tray view from the app's read-back effective state: profiles
/// valid for the current power state (eco entry kept but greyed when the
/// firmware rejected it), read-back values for the checked profile, smart
/// charge and plan, and the degraded flags when the Acer WMI interface is
/// unavailable — in that case the "Windows plan" section still offers plan
/// switches for the current power state's profiles.
fn view_from(app: &AppCore) -> TrayView {
    let effective = app.effective();
    let profiles = match app.current_power() {
        PowerState::Ac => AC_PROFILES.to_vec(),
        PowerState::Battery => BATTERY_PROFILES.to_vec(),
    };
    let degraded = !app.wmi_available();
    let plans = if degraded { profiles.clone() } else { Vec::new() };
    // Read-back firmware profile; when WMI can't report it, the active
    // Windows plan is still OS-truth and identifies the profile in effect.
    let profile = effective
        .profile
        .or_else(|| effective.plan.as_deref().and_then(Profile::from_plan_name));
    // Read-back smart-charge state; when the adapter can't report it, show
    // the intent — smart charge is always intended on and cannot be disabled.
    let smart_charge = effective.smart_charge.or(Some(true));
    TrayView {
        power: effective.power,
        percent: effective.percent,
        profile,
        eco_disabled: app.eco_disabled(),
        profiles,
        profiles_greyed: degraded,
        plans,
        smart_charge,
        plan: effective.plan,
        // The status line is set by the user-action handlers only.
        status: None,
        degraded,
    }
}

/// Push a view whose status line reports the outcome of a user-initiated
/// change: "Applied", "Failed: <items>", "Not applied: <items>", or a mix.
/// Ephemeral — the tray clears it when the menu is dismissed; no history.
fn update_with_status(app: &AppCore, tray: &Tray, report: &ApplyReport) {
    let mut view = view_from(app);
    view.status = Some(apply_report_text(report));
    if let Err(e) = tray.update(&view) {
        log::warn(format!("failed to update tray view: {e:?}"));
    }
}

/// One flat event dispatch; later tickets add match arms here
/// (Hotkey: ticket 11, ReapplyTick: ticket 13).
fn handle_event(app: &mut AppCore, tray: &Tray, ev: TrayEvent) {
    match ev {
        TrayEvent::Quit => {
            log::info("quit requested");
            // later tickets: unregister hotkey (11), destroy tray icon (08)
            unsafe { PostQuitMessage(0) };
        }
        TrayEvent::SelectProfile(profile) => {
            log::info(format!("profile selected: {}", profile.as_str()));
            let failed = app.apply_profile(profile);
            update_with_status(app, tray, &failed);
        }
        TrayEvent::PowerChanged => {
            log::info("power state changed");
            enforcement::on_power_changed(app);
            if let Err(e) = tray.update(&view_from(app)) {
                log::warn(format!("failed to update tray view: {e:?}"));
            }
        }
        TrayEvent::Resume => {
            log::info("system resumed");
            enforcement::on_resume(app);
            if let Err(e) = tray.update(&view_from(app)) {
                log::warn(format!("failed to update tray view: {e:?}"));
            }
        }
        TrayEvent::HotkeyPressed => {
            let (profile, failed) = app.cycle_profile();
            log::info(format!("hotkey cycled to profile {}", profile.as_str()));
            update_with_status(app, tray, &failed);
            tray.notify("Nitro Tray", &format!("Profile: {}", profile.as_str()));
        }
        TrayEvent::ReapplyTick => {
            reapply::on_tick(app);
        }
        TrayEvent::RecoveryTick => {
            if recovery::on_tick(app) {
                log::info("recovery: adapter reconnected; enforcing and refreshing tray");
                if let Err(e) = tray.update(&view_from(app)) {
                    log::warn(format!("failed to update tray view: {e:?}"));
                }
            }
        }
        TrayEvent::ReadbackTick => {
            // Targeted state re-reads (single profile read, single-pair
            // smart-charge read, plan read) + view refresh; keeps a quiet
            // session from leaving stale or degraded-looking tray state.
            if let Err(e) = tray.update(&view_from(app)) {
                log::warn(format!("failed to update tray view: {e:?}"));
            }
        }
        TrayEvent::SelectPlan(profile) => {
            log::info(format!("plan selected: {}", profile.plan_name()));
            let failed = app.apply_plan(profile);
            update_with_status(app, tray, &failed);
        }
    }
}
