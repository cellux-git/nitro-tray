//! Windows binary entry point (linux-port ticket 02): single-instance mutex,
//! logon-task install/removal (driven by the "Start at logon" state flag),
//! config + app core startup, tray creation, and the message pump that
//! dispatches tray events.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::mpsc;

use winapi::um::libloaderapi::GetModuleFileNameW;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostQuitMessage,
};
use windows_sys::core::w;

use nitro_tray::app::{AppCore, ApplyReport, apply_report_text};
use nitro_tray::config;
use nitro_tray::hotkey::Hotkey;
use nitro_tray::log;
use nitro_tray::power::PowerApi;
use nitro_tray::task;
use nitro_tray::timers;
use nitro_tray::tray::{Tray, TrayError};
use nitro_tray::tray_model::{TrayEvent, TrayView};
use nitro_tray::wiring;

pub fn run() {
    let Some(exe_path) = executable_path() else {
        return; // cannot resolve our own location; nothing sensible to do
    };
    let exe_dir = exe_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let config = config::load(&exe_dir);

    log::init(&exe_dir);
    if config.log {
        log::set_enabled(true);
    }
    log::info("nitro-tray starting");

    log::install_panic_hook();

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
    let (wmi, charge, hid) = wiring::connect_adapters();
    let mut app = AppCore::new(config, &exe_dir, wmi, charge, hid, PowerApi);
    log::info("app core initialized");

    // "Start at logon" (state file, default off): when on, make sure the
    // scheduled task exists — also on every boot, so a deleted task is
    // recreated. When off, make sure no task lingers (e.g. from an earlier
    // version that auto-installed).
    if app.start_at_logon() {
        match task::install_logon_task(&exe_path) {
            Ok(()) => log::info("logon task installed (start at logon)"),
            Err(e) => log::warn(format!("failed to install logon task: {e:?}")),
        }
    } else {
        match task::uninstall_logon_task() {
            Ok(()) => log::info("logon task absent (start at logon off)"),
            Err(e) => log::warn(format!("failed to remove stale logon task: {e:?}")),
        }
    }

    let (event_tx, event_rx) = mpsc::channel();
    let tray = match Tray::create(event_tx) {
        Ok(tray) => tray,
        Err(e) => {
            log::error(format!("failed to create tray: {e:?}"));
            let TrayError::Create(message) = &e else {
                unreachable!("Tray::create only returns TrayError::Create");
            };
            fatal(format!("Failed to create the tray icon:\n\n{message}"));
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
            log::warn(format!(
                "hotkey: failed to register {hotkey_spec:?}: {err:?}"
            ));
            None
        }
    };

    if timers::reapply_enabled(&reapply_cfg) {
        let interval = timers::reapply_interval_ms(&reapply_cfg);
        match tray.start_timer(timers::REAPPLY_TIMER_ID, interval) {
            Ok(()) => log::info(format!("reapply loop enabled; interval {interval} ms")),
            Err(err) => log::warn(format!("reapply: failed to arm timer: {err:?}")),
        }
    }

    // Recovery and the periodic readback are always armed — broken adapters
    // must recover and the tray view must refresh even when reapply is off.
    match tray.start_timer(timers::RECOVERY_TIMER_ID, timers::RECOVERY_INTERVAL_MS) {
        Ok(()) => log::info(format!(
            "recovery loop armed; interval {} ms",
            timers::RECOVERY_INTERVAL_MS
        )),
        Err(err) => log::warn(format!("recovery: failed to arm timer: {err:?}")),
    }
    match tray.start_timer(timers::READBACK_TIMER_ID, timers::READBACK_INTERVAL_MS) {
        Ok(()) => log::info(format!(
            "readback loop armed; interval {} ms",
            timers::READBACK_INTERVAL_MS
        )),
        Err(err) => log::warn(format!("recovery: failed to arm readback timer: {err:?}")),
    }

    app.on_startup();
    log::info("startup enforcement complete");

    log::info("entering message pump");
    message_pump(&tray, &mut app, &event_rx, &exe_path);
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
    let len =
        unsafe { GetModuleFileNameW(ptr::null_mut(), buffer.as_mut_ptr(), buffer.len() as u32) };
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
fn message_pump(
    tray: &Tray,
    app: &mut AppCore,
    events: &mpsc::Receiver<TrayEvent>,
    exe_path: &Path,
) {
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
            handle_event(app, tray, ev, exe_path);
        }
    }
}

/// Build the tray view from the app's read-back effective state (the shared
/// core in `tray_model::view_from`): one `effective()` read per view push;
/// Windows passes `degraded` for both the profile greying and the "Windows
/// plan" section — menu byte-identical to the pre-split build.
fn view_from(app: &AppCore) -> TrayView {
    let e = app.effective();
    nitro_tray::tray_model::view_from(&e, !e.wmi_available, !e.wmi_available, app.start_at_logon())
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
fn handle_event(app: &mut AppCore, tray: &Tray, ev: TrayEvent, exe_path: &Path) {
    match ev {
        TrayEvent::Quit => {
            log::info("quit requested");
            // later tickets: unregister hotkey (11), destroy tray icon (08)
            unsafe { PostQuitMessage(0) };
        }
        TrayEvent::ToggleLogonTask => {
            log::info("start-at-logon toggled");
            let enable = !app.start_at_logon();
            let outcome = if enable {
                task::install_logon_task(exe_path)
            } else {
                task::uninstall_logon_task()
            };
            let mut report = ApplyReport::default();
            match outcome {
                Ok(()) => {
                    app.set_start_at_logon(enable);
                    log::info(format!(
                        "start at logon {}",
                        if enable { "enabled" } else { "disabled" }
                    ));
                }
                Err(err) => {
                    log::warn(format!("logon task: {err:?}"));
                    report.failed.push("logon task");
                }
            }
            update_with_status(app, tray, &report);
        }
        TrayEvent::SelectProfile(profile) => {
            log::info(format!("profile selected: {}", profile.as_str()));
            let failed = app.apply_profile(profile);
            update_with_status(app, tray, &failed);
        }
        TrayEvent::PowerChanged => {
            log::info("power state changed");
            app.on_power_changed();
            if let Err(e) = tray.update(&view_from(app)) {
                log::warn(format!("failed to update tray view: {e:?}"));
            }
        }
        TrayEvent::Resume => {
            log::info("system resumed");
            app.on_resume();
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
            app.on_reapply_tick();
        }
        TrayEvent::RecoveryTick => {
            if app.on_recovery_tick() {
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
            // Smart charge reads back off -> re-enable (always armed, so a
            // silent external disable is fixed within a minute).
            app.reassert_smart_charge();
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
