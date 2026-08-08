//! Tray UI: static icon in the system tray, left-click opens the menu, menu
//! and tooltip show the *read-back* effective state, balloon notifications,
//! and a degraded "Hardware unavailable" state when the Acer WMI interface is
//! unreachable. The tray window also owns the power-notification and timer
//! plumbing that feeds the app core.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use windows_sys::core::{w, GUID};
use windows_sys::Win32::Foundation::{
    GetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM, ERROR_CLASS_ALREADY_EXISTS,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, FindWindowW, GetCursorPos, GetWindowLongPtrW, KillTimer,
    MFT_RADIOCHECK, PostMessageW, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    TrackPopupMenu, DEVICE_NOTIFY_WINDOW_HANDLE, GWLP_USERDATA, HICON, HMENU, ICONINFO, MF_CHECKED,
    MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, PBT_APMPOWERSTATUSCHANGE,
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE, TPM_LEFTALIGN,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_TOPALIGN, WNDCLASSW, WM_APP, WM_HOTKEY, WM_LBUTTONUP,
    WM_POWERBROADCAST, WM_RBUTTONUP, WM_TIMER, WS_OVERLAPPEDWINDOW,
};

use crate::hotkey::HOTKEY_ID;
use crate::log;
use crate::policy::{profiles_for, PowerState, Profile};
use crate::power_state::{self, PowerStateSnapshot};
use crate::timers;

/// Hidden window class (tray icon owner, power notifications, poll timer).
/// The class name is a `w!` literal (the macro takes literals only).
const ICON_ID: u32 = 1;
/// uCallbackMessage: the shell posts mouse messages here.
const TRAY_MSG: u32 = WM_APP + 1;
/// Posted after every channel send to wake the GetMessageW pump.
const WAKE_MSG: u32 = WM_APP + 2;
const POLL_TIMER_ID: usize = 1;
const MENU_PROFILE_BASE: usize = 1;
const MENU_PLAN_BASE: usize = 300;
const MENU_QUIT: usize = 200;
const MENU_LOGON_TASK: usize = 400;
const ICON_SIZE: i32 = 16;
/// Explorer's taskbar window; its notification area is what `NIM_ADD`
/// registers into. Waiting for it before the first `Shell_NotifyIconW` call
/// means retries are spent only on real failures, not on Explorer being down.
const SHELL_TRAY_WINDOW: *const u16 = w!("Shell_TrayWnd");
/// How long to wait for `Shell_TrayWnd` (polls every `SHELL_POLL_MS`).
const SHELL_WAIT_MS: u64 = 10_000;
const SHELL_POLL_MS: u64 = 250;
/// `Shell_NotifyIconW NIM_ADD` retries: the `Shell_TrayWnd` wait covers
/// Explorer not being up yet at logon; the retry loop covers every other
/// failure. Retry every second for up to 5 attempts.
const NIM_ADD_ATTEMPTS: u32 = 5;
const NIM_ADD_RETRY_MS: u64 = 1_000;

/// GUID_ACDC_POWER_SOURCE; not shipped by windows-sys, hardcoded per spec
/// (5d3e9a59-e9d5-4b00-a6bd-ff34ff516548).
const GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Events raised by the tray window (menu picks, power messages, timers).
/// The main loop drains the channel and drives the app core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// The user chose Quit.
    Quit,
    /// The user toggled the "Start at logon" checkbox.
    ToggleLogonTask,
    /// The user picked a profile in the menu.
    SelectProfile(Profile),
    /// Power status changed (WM_POWERBROADCAST or slow poll fallback).
    PowerChanged,
    /// The machine resumed from sleep (WM_POWERBROADCAST).
    Resume,
    /// The global profile-cycle hotkey was pressed (WM_HOTKEY).
    HotkeyPressed,
    /// The reapply timer ticked (WM_TIMER, timers::REAPPLY_TIMER_ID).
    ReapplyTick,
    /// The recovery timer ticked (WM_TIMER, timers::RECOVERY_TIMER_ID):
    /// retry adapters that failed their circuit breaker.
    RecoveryTick,
    /// The periodic readback timer ticked (WM_TIMER, timers::READBACK_TIMER_ID).
    ReadbackTick,
    /// The user picked a Windows plan in the degraded-mode plan section.
    SelectPlan(Profile),
}

/// The view the main loop pushes into the tray after each state change:
/// read-back effective-state facts. The tray derives the menu contents
/// (profile list per power state, greys, degraded "Windows plan" section,
/// smart-charge intent fallback) from these facts in `menu_items`.
#[derive(Clone, Debug)]
pub struct TrayView {
    pub power: PowerState,
    pub percent: u8,
    /// Current effective profile (checked in the menu); `None` when unknown.
    pub profile: Option<Profile>,
    /// Show the degraded "Hardware unavailable" state: profile items greyed,
    /// "Windows plan" section offered instead.
    pub degraded: bool,
    /// Grey out just the eco entry (firmware rejected profile 6).
    pub eco_disabled: bool,
    /// Read-back smart-charge state; `None` when unavailable. The menu treats
    /// `None` as the intent — smart charge is always intended on — showing
    /// the "Smart charge (80% cap)" line checked unless the readback says off.
    pub smart_charge: Option<bool>,
    /// Active Windows plan name; `None` when unknown.
    pub plan: Option<String>,
    /// Ephemeral status line at the bottom of the menu: last apply outcome
    /// ("Applied" / "Failed: ..."), shown only until the menu is dismissed.
    /// `None` = no line.
    pub status: Option<String>,
    /// "Start at logon" checkbox state (logon scheduled task installed).
    pub start_at_logon: bool,
}

/// Errors from tray creation/updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayError {
    Create(&'static str),
    Update(&'static str),
}

/// Everything the WndProc needs, stored in GWLP_USERDATA. All access happens
/// on the UI thread (messages and `Tray` methods run there), so `RefCell` is
/// safe.
struct TrayState {
    hwnd: HWND,
    view: RefCell<TrayView>,
    last_snapshot: RefCell<PowerStateSnapshot>,
    event_tx: Sender<TrayEvent>,
    channel_closed: Cell<bool>,
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Kept for the icon's lifetime; freed in `Drop` after `DestroyIcon`.
    bitmap: HBITMAP,
    /// Monochrome mask handed to `CreateIconIndirect`; freed like `bitmap`.
    mask: HBITMAP,
    power_notify: HPOWERNOTIFY,
    state: Box<TrayState>,
}

impl Tray {
    /// Create the hidden window + tray icon. Window messages (tray clicks,
    /// WM_POWERBROADCAST, timers) raise `TrayEvent`s on `event_tx`.
    pub fn create(event_tx: Sender<TrayEvent>) -> Result<Self, TrayError> {
        unsafe {
            let hinstance = GetModuleHandleW(ptr::null());
            if hinstance.is_null() {
                return Err(TrayError::Create("GetModuleHandleW failed"));
            }
            register_class(hinstance)?;

            let hwnd = CreateWindowExW(
                0,
                w!("NitroTrayTrayWnd"),
                ptr::null(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                hinstance,
                ptr::null(),
            );
            if hwnd.is_null() {
                return Err(TrayError::Create("CreateWindowExW failed"));
            }

            let state = Box::new(TrayState {
                hwnd,
                view: RefCell::new(TrayView {
                    power: PowerState::Ac,
                    percent: 0,
                    profile: None,
                    degraded: false,
                    eco_disabled: false,
                    smart_charge: None,
                    plan: None,
                    status: None,
                    start_at_logon: false,
                }),
                last_snapshot: RefCell::new(power_state::read()),
                event_tx,
                channel_closed: Cell::new(false),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const TrayState as isize);

            let Some((icon, bitmap, mask)) = make_battery_icon() else {
                destroy_window(hwnd);
                return Err(TrayError::Create("icon creation failed"));
            };
            let nid = nid_template(hwnd, icon);
            if !wait_for_shell_tray() {
                log::debug(
                    "Shell_TrayWnd not found in time; Explorer may not be up — NIM_ADD retries are the fallback",
                );
            }
            let mut added = false;
            for attempt in 0..NIM_ADD_ATTEMPTS {
                if Shell_NotifyIconW(NIM_ADD, &nid) != 0 {
                    added = true;
                    break;
                }
                log::debug(format!(
                    "Shell_NotifyIconW NIM_ADD failed (attempt {}); retrying",
                    attempt + 1
                ));
                std::thread::sleep(std::time::Duration::from_millis(NIM_ADD_RETRY_MS));
            }
            if !added {
                log::warn(format!(
                    "Shell_NotifyIconW NIM_ADD failed after {NIM_ADD_ATTEMPTS} attempts"
                ));
                destroy_icon_assets(icon, bitmap, mask);
                destroy_window(hwnd);
                return Err(TrayError::Create(
                    "Shell_NotifyIconW NIM_ADD failed after 5 attempts. \
                     The Windows notification area is unavailable — Explorer may \
                     not be running. Restart Explorer (Task Manager -> Restart) \
                     and launch Nitro Tray again.",
                ));
            }

            if SetTimer(hwnd, POLL_TIMER_ID, power_state::SLOW_POLL_MS, None) == 0 {
                let nid = nid_template(hwnd, ptr::null_mut());
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                destroy_icon_assets(icon, bitmap, mask);
                destroy_window(hwnd);
                return Err(TrayError::Create("SetTimer failed"));
            }

            let power_notify = RegisterPowerSettingNotification(
                hwnd as HANDLE,
                &GUID_ACDC_POWER_SOURCE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            );
            if power_notify == 0 {
                log::warn("RegisterPowerSettingNotification failed; relying on WM_POWERBROADCAST + poll");
            }

            Ok(Tray {
                hwnd,
                icon,
                bitmap,
                mask,
                power_notify,
                state,
            })
        }
    }

    /// The hidden window handle (used for RegisterHotKey / timers).
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Arm a Windows timer on the tray window (e.g. the reapply loop).
    pub fn start_timer(&self, id: usize, interval_ms: u32) -> Result<(), TrayError> {
        if unsafe { SetTimer(self.hwnd, id, interval_ms, None) } == 0 {
            return Err(TrayError::Create("SetTimer failed"));
        }
        Ok(())
    }

    /// Kill a Windows timer armed by `start_timer`.
    pub fn stop_timer(&self, id: usize) {
        unsafe {
            KillTimer(self.hwnd, id);
        }
    }

    /// Push a new view: rebuild menu items (applied on next open) and the
    /// tooltip. Tooltip shows power state, battery %, active profile, plan,
    /// and smart-charge state; degraded state when `view.degraded`.
    pub fn update(&self, view: &TrayView) -> Result<(), TrayError> {
        *self.state.view.borrow_mut() = view.clone();
        unsafe {
            let mut nid = nid_template(self.hwnd, self.icon);
            set_tip(&mut nid, &tooltip_text(view));
            if Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                return Err(TrayError::Update("Shell_NotifyIconW NIM_MODIFY failed"));
            }
        }
        Ok(())
    }

    /// Show a transient balloon notification (hotkey feedback only; automatic
    /// switching stays silent).
    pub fn notify(&self, title: &str, body: &str) {
        unsafe {
            let mut nid = nid_template(self.hwnd, ptr::null_mut());
            nid.uFlags = NIF_INFO;
            set_wide(&mut nid.szInfoTitle, title);
            set_wide(&mut nid.szInfo, body);
            nid.dwInfoFlags = NIIF_INFO;
            if Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                log::warn("Shell_NotifyIconW NIM_MODIFY (balloon) failed");
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let nid = nid_template(self.hwnd, ptr::null_mut());
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            if self.power_notify != 0 {
                UnregisterPowerSettingNotification(self.power_notify);
            }
            KillTimer(self.hwnd, POLL_TIMER_ID);
            destroy_window(self.hwnd);
            destroy_icon_assets(self.icon, self.bitmap, self.mask);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let userdata = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    let Some(state) = (unsafe { (userdata as *const TrayState).as_ref() }) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        TRAY_MSG => match lparam as u32 {
            WM_LBUTTONUP | WM_RBUTTONUP => open_menu(hwnd, state),
            _ => {}
        },
        WAKE_MSG => {}
        WM_TIMER => match wparam {
            POLL_TIMER_ID => poll_power(state),
            timers::REAPPLY_TIMER_ID => send_event(state, TrayEvent::ReapplyTick),
            timers::RECOVERY_TIMER_ID => send_event(state, TrayEvent::RecoveryTick),
            timers::READBACK_TIMER_ID => send_event(state, TrayEvent::ReadbackTick),
            _ => {}
        },
        WM_HOTKEY => {
            if wparam as i32 == HOTKEY_ID {
                send_event(state, TrayEvent::HotkeyPressed);
            }
        }
        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMPOWERSTATUSCHANGE | PBT_POWERSETTINGCHANGE => poll_power(state),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => send_event(state, TrayEvent::Resume),
                _ => {}
            }
            return 1;
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}

fn register_class(hinstance: windows_sys::Win32::Foundation::HINSTANCE) -> Result<(), TrayError> {
    if CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        lpszClassName: w!("NitroTrayTrayWnd"),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
        CLASS_REGISTERED.store(false, Ordering::SeqCst);
        return Err(TrayError::Create("RegisterClassW failed"));
    }
    Ok(())
}

fn nid_template(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: TRAY_MSG,
        hIcon: icon,
        ..Default::default()
    }
}

/// Wait up to `SHELL_WAIT_MS` for Explorer's taskbar window to exist. At
/// logon the app can start before Explorer, in which case every `NIM_ADD`
/// attempt is guaranteed to fail — gating the first attempt on this window
/// avoids burning retries on that known case. Returns true when the window
/// was found; the `NIM_ADD` retry loop still covers every other failure.
fn wait_for_shell_tray() -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(SHELL_WAIT_MS);
    while std::time::Instant::now() < deadline {
        if !unsafe { FindWindowW(SHELL_TRAY_WINDOW, ptr::null()) }.is_null() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(SHELL_POLL_MS));
    }
    false
}

/// Copy a string into a fixed-size wide buffer, truncating and NUL-terminating.
fn set_wide(dst: &mut [u16], s: &str) {
    let limit = dst.len().saturating_sub(1);
    for (i, ch) in s.encode_utf16().take(limit).enumerate() {
        dst[i] = ch;
    }
}

fn set_tip(nid: &mut NOTIFYICONDATAW, tip: &str) {
    set_wide(&mut nid.szTip, tip);
}

/// Send an event to the main loop and wake the pump. Once the channel is
/// closed, stop posting.
fn send_event(state: &TrayState, ev: TrayEvent) {
    if state.channel_closed.get() {
        return;
    }
    if state.event_tx.send(ev).is_err() {
        state.channel_closed.set(true);
        log::warn("tray event channel closed; stopping event dispatch");
        return;
    }
    unsafe {
        PostMessageW(state.hwnd, WAKE_MSG, 0, 0);
    }
}

/// Slow-poll fallback and PBT_POWERSETTINGCHANGE: compare the fresh read to
/// the last snapshot and raise `PowerChanged` only when the AC/battery STATE
/// changed (battery-% drift must not re-assert anything).
fn poll_power(state: &TrayState) {
    let now = power_state::read();
    let changed = state.last_snapshot.borrow().state != now.state;
    *state.last_snapshot.borrow_mut() = now;
    if changed {
        send_event(state, TrayEvent::PowerChanged);
    }
}

/// One derived menu entry: the raw `AppendMenuW` flags, the item id, and the
/// label. Separators are `MF_SEPARATOR` entries with an empty label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// Menu item id (`AppendMenuW` `uID`); 0 for non-routable items.
    pub id: usize,
    /// `MF_*` / `MFT_*` flag bits passed to `AppendMenuW`.
    pub flags: u32,
    /// Item text; empty for separators.
    pub label: String,
}

/// The degraded-mode "Windows plan" section: the power state's profiles when
/// the WMI interface is unavailable (the firmware profile section is
/// unusable), else empty. Single derivation — the menu model and the id
/// routing both go through this, so the plan ids cannot drift apart.
fn plans_for(view: &TrayView) -> &'static [Profile] {
    if view.degraded {
        profiles_for(view.power)
    } else {
        &[]
    }
}

/// Derive the full popup-menu model from the effective-state facts: the
/// logon checkbox, the degraded "Hardware unavailable" banner, the profile
/// items for the current power state (radio, checked for the effective
/// profile, greyed when degraded or the eco entry is firmware-rejected), the
/// smart-charge intent-fallback status line, the plan line, the degraded-mode
/// "Windows plan" section, Quit, and the ephemeral status line. Pure — no
/// window, no Win32 — so the menu is unit-testable directly.
pub fn menu_items(view: &TrayView) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let logon_label = if view.start_at_logon {
        "\u{2611} Start at logon" // ☑
    } else {
        "\u{2610} Start at logon" // ☐
    };
    items.push(menu_entry(MF_STRING, MENU_LOGON_TASK, logon_label));
    items.push(separator_entry());
    if view.degraded {
        items.push(menu_entry(MF_GRAYED | MF_DISABLED, 0, "Hardware unavailable"));
        items.push(separator_entry());
    }
    let profile_flags = if view.degraded {
        MF_GRAYED | MF_DISABLED
    } else {
        0
    };
    for (i, profile) in profiles_for(view.power).iter().enumerate() {
        let mut flags = profile_flags
            | MFT_RADIOCHECK
            | if Some(*profile) == view.profile {
                MF_CHECKED
            } else {
                0
            };
        if view.eco_disabled && *profile == Profile::Eco {
            flags |= MF_GRAYED | MF_DISABLED;
        }
        items.push(menu_entry(flags, MENU_PROFILE_BASE + i, profile_label(*profile)));
    }
    items.push(separator_entry());
    // Smart charge is always intended on and cannot be disabled in the app;
    // the item is a static status line (checked unless the readback says the
    // cap is not in effect — a `None` readback means the intent holds).
    let smart_flags = MF_DISABLED
        | if view.smart_charge != Some(false) {
            MF_CHECKED
        } else {
            0
        };
    items.push(menu_entry(smart_flags, 0, "Smart charge (80% cap)"));
    if let Some(plan) = &view.plan {
        items.push(menu_entry(MF_GRAYED | MF_DISABLED, 0, &format!("Plan: {plan}")));
    }
    // Degraded mode: the firmware profile section is unusable, so the
    // "Windows plan" section offers the same profiles through OS plans.
    let plans: &'static [Profile] = plans_for(view);
    if !plans.is_empty() {
        items.push(separator_entry());
        items.push(menu_entry(MF_GRAYED | MF_DISABLED, 0, "Windows plan"));
        for (i, profile) in plans.iter().enumerate() {
            let flags = MFT_RADIOCHECK
                | if Some(*profile) == view.profile {
                    MF_CHECKED
                } else {
                    0
                };
            items.push(menu_entry(flags, MENU_PLAN_BASE + i, profile_label(*profile)));
        }
    }
    items.push(separator_entry());
    items.push(menu_entry(MF_STRING, MENU_QUIT, "Quit"));
    // Ephemeral status line: the last apply outcome; cleared on dismissal
    // (no history is kept).
    if let Some(status) = &view.status {
        items.push(separator_entry());
        items.push(menu_entry(MF_GRAYED | MF_DISABLED, 0, status));
    }
    items
}

fn separator_entry() -> MenuItem {
    menu_entry(MF_SEPARATOR, 0, "")
}

fn menu_entry(flags: u32, id: usize, label: &str) -> MenuItem {
    MenuItem {
        id,
        flags,
        label: label.to_string(),
    }
}
/// Build the popup menu from the stored view (via the derived `menu_items`
/// model) and route the picked id.
fn open_menu(hwnd: HWND, state: &TrayState) {
    let view = state.view.borrow().clone();
    let items = menu_items(&view);
    // The profile/plan vectors back the id ranges in the model; the routing
    // ids below must match the model ids exactly (same derivation).
    let profiles = profiles_for(view.power);
    let plans: &'static [Profile] = plans_for(&view);
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        log::warn("CreatePopupMenu failed");
        return;
    }
    for item in &items {
        if item.flags & MF_SEPARATOR != 0 {
            append_separator(menu);
        } else {
            append_item(menu, item.flags, item.id, &item.label);
        }
    }

    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt) };
    unsafe {
        SetForegroundWindow(hwnd);
    }
    let cmd = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            ptr::null(),
        ) as u32 as usize
    };
    unsafe {
        DestroyMenu(menu);
    }
    match cmd {
        MENU_QUIT => send_event(state, TrayEvent::Quit),
        MENU_LOGON_TASK => send_event(state, TrayEvent::ToggleLogonTask),
        id if id >= MENU_PROFILE_BASE && id < MENU_PROFILE_BASE + profiles.len() => {
            if let Some(&profile) = profiles.get(id - MENU_PROFILE_BASE) {
                send_event(state, TrayEvent::SelectProfile(profile));
            }
        }
        id if id >= MENU_PLAN_BASE && id < MENU_PLAN_BASE + plans.len() => {
            if let Some(&profile) = plans.get(id - MENU_PLAN_BASE) {
                send_event(state, TrayEvent::SelectPlan(profile));
            }
        }
        0 => {
            // Dismissed without a pick (unfocus): the status line is
            // ephemeral — no history is kept.
            state.view.borrow_mut().status = None;
        }
        _ => {}
    }
}

fn append_separator(menu: HMENU) {
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    }
}

fn append_item(menu: HMENU, flags: u32, id: usize, label: &str) {
    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
    unsafe {
        AppendMenuW(menu, flags, id, wide.as_ptr());
    }
}

fn profile_label(profile: Profile) -> &'static str {
    match profile {
        Profile::Quiet => "Quiet",
        Profile::Balanced => "Balanced",
        Profile::Performance => "Performance",
        Profile::Eco => "Eco",
    }
}

/// The tooltip text: first line is the state summary, second line the plan and
/// smart-charge state. `\n\n\n` is the standard two-line tooltip trick.
pub fn tooltip_text(view: &TrayView) -> String {
    let status = if view.degraded {
        "Hardware unavailable"
    } else {
        "Nitro Tray"
    };
    let power = match view.power {
        PowerState::Ac => "AC",
        PowerState::Battery => "Battery",
    };
    let profile = view.profile.map(|p| p.as_str()).unwrap_or("unknown");
    let smart = match view.smart_charge {
        Some(true) => "on",
        Some(false) => "off",
        None => "unknown",
    };
    let plan = view.plan.as_deref().unwrap_or("unknown");
    format!(
        "{status} — {percent}% | {profile} | {power}\n\n\n{plan} | Smart charge {smart}",
        percent = view.percent,
    )
}

/// 16x16 battery glyph drawn into a 32bpp top-down DIB, plus a monochrome
/// mask bitmap: `CreateIconIndirect` on this Windows version rejects a NULL
/// mask even for 32bpp+alpha color icons. Both bitmap handles are kept alive
/// by the caller for the icon's lifetime.
fn make_battery_icon() -> Option<(HICON, HBITMAP, HBITMAP)> {
    unsafe {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = ICON_SIZE;
        bmi.bmiHeader.biHeight = -ICON_SIZE;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = ptr::null_mut();
        let bitmap = CreateDIBSection(
            ptr::null_mut(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            ptr::null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            return None;
        }
        let pixels =
            std::slice::from_raw_parts_mut(bits as *mut u32, (ICON_SIZE * ICON_SIZE) as usize);
        draw_battery_pixels(pixels, ICON_SIZE as usize);

        let mask = CreateBitmap(ICON_SIZE, ICON_SIZE, 1, 1, ptr::null());
        if mask.is_null() {
            DeleteObject(bitmap as _);
            return None;
        }

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: bitmap,
        };
        let icon = CreateIconIndirect(&info);
        if icon.is_null() {
            DeleteObject(bitmap as _);
            DeleteObject(mask as _);
            return None;
        }
        Some((icon, bitmap, mask))
    }
}

/// Fill the pixel buffer (BGRA, top-down) with the battery glyph.
fn draw_battery_pixels(pixels: &mut [u32], size: usize) {
    const TRANSPARENT: u32 = 0x0000_0000;
    const BORDER: u32 = 0xFF_2E_35_3C; // RGB(0x2E, 0x35, 0x3C)
    const FILL: u32 = 0xFF_3E_B0_4E; // RGB(0x3E, 0xB0, 0x4E)
    const CHARGE: u32 = 0xFF_66_CE_6B; // RGB(0x66, 0xCE, 0x6B)
    const NUB: u32 = 0xFF_B8_C0_C8; // RGB(0xB8, 0xC0, 0xC8)
    for y in 0..size {
        for x in 0..size {
            let color = if (13..=14).contains(&x) && (6..=9).contains(&y) {
                NUB
            } else if (1..=12).contains(&x) && (3..=12).contains(&y) {
                if x == 1 || x == 12 || y == 3 || y == 12 {
                    BORDER
                } else if x <= 7 {
                    CHARGE
                } else {
                    FILL
                }
            } else {
                TRANSPARENT
            };
            pixels[y * size + x] = color;
        }
    }
}

fn destroy_window(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        DestroyWindow(hwnd);
    }
}

fn destroy_icon_assets(icon: HICON, bitmap: HBITMAP, mask: HBITMAP) {
    unsafe {
        DestroyIcon(icon);
        if !bitmap.is_null() {
            DeleteObject(bitmap as _);
        }
        if !mask.is_null() {
            DeleteObject(mask as _);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Profile;

    fn view(power: PowerState, percent: u8, profile: Option<Profile>, plan: Option<&str>) -> TrayView {
        TrayView {
            power,
            percent,
            profile,
            degraded: false,
            eco_disabled: false,
            smart_charge: None,
            plan: plan.map(String::from),
            status: None,
            start_at_logon: false,
        }
    }

    #[test]
    fn tooltip_contains_all_read_back_values() {
        let v = TrayView {
            power: PowerState::Ac,
            percent: 87,
            profile: Some(Profile::Balanced),
            degraded: false,
            eco_disabled: false,
            smart_charge: Some(true),
            plan: Some("Nitro-Balanced".to_string()),
            status: None,
            start_at_logon: false,
        };
        let text = tooltip_text(&v);
        assert!(text.contains("Nitro Tray"));
        assert!(text.contains("87%"));
        assert!(text.contains("balanced"));
        assert!(text.contains("AC"));
        assert!(text.contains("Nitro-Balanced"));
        assert!(text.contains("Smart charge on"));
        assert!(text.contains("\n\n\n"));
    }

    #[test]
    fn tooltip_battery_power_state() {
        let v = view(PowerState::Battery, 42, Some(Profile::Eco), Some("Nitro-Eco"));
        let text = tooltip_text(&v);
        assert!(text.contains("Battery"));
        assert!(text.contains("eco"));
        assert!(text.contains("Nitro-Eco"));
    }

    #[test]
    fn tooltip_smart_charge_off() {
        let mut v = view(PowerState::Ac, 50, None, None);
        v.smart_charge = Some(false);
        assert!(tooltip_text(&v).contains("Smart charge off"));
    }

    #[test]
    fn tooltip_unknowns_and_degraded() {
        let mut v = view(PowerState::Battery, 255, None, None);
        v.degraded = true;
        let text = tooltip_text(&v);
        assert!(text.starts_with("Hardware unavailable"));
        assert!(text.contains("unknown"));
    }

    #[test]
    fn battery_glyph_is_transparent_outside_body() {
        let mut pixels = vec![0u32; 16 * 16];
        draw_battery_pixels(&mut pixels, 16);
        assert_eq!(pixels[0], 0x0000_0000);
        assert_eq!(pixels[15], 0x0000_0000);
        assert_eq!(pixels[15 * 16], 0x0000_0000);
    }

    #[test]
    fn battery_glyph_has_border_fill_and_nub() {
        let mut pixels = vec![0u32; 16 * 16];
        draw_battery_pixels(&mut pixels, 16);
        assert_eq!(pixels[3 * 16 + 1], 0xFF_2E_35_3C, "top-left border");
        assert_eq!(pixels[8 * 16 + 13], 0xFF_B8_C0_C8, "terminal nub");
        assert_eq!(pixels[8 * 16 + 5], 0xFF_66_CE_6B, "charge half");
        assert_eq!(pixels[8 * 16 + 10], 0xFF_3E_B0_4E, "fill half");
    }

    fn entry(flags: u32, id: usize, label: &str) -> MenuItem {
        MenuItem {
            id,
            flags,
            label: label.to_string(),
        }
    }

    fn separator() -> MenuItem {
        entry(MF_SEPARATOR, 0, "")
    }

    #[test]
    fn menu_exact_normal_ac_view() {
        let v = TrayView {
            power: PowerState::Ac,
            percent: 87,
            profile: Some(Profile::Balanced),
            degraded: false,
            eco_disabled: false,
            smart_charge: Some(true),
            plan: Some("Nitro-Balanced".to_string()),
            status: None,
            start_at_logon: false,
        };
        assert_eq!(
            menu_items(&v),
            vec![
                entry(MF_STRING, MENU_LOGON_TASK, "\u{2610} Start at logon"),
                separator(),
                entry(MFT_RADIOCHECK, MENU_PROFILE_BASE, "Quiet"),
                entry(MFT_RADIOCHECK | MF_CHECKED, MENU_PROFILE_BASE + 1, "Balanced"),
                entry(MFT_RADIOCHECK, MENU_PROFILE_BASE + 2, "Performance"),
                separator(),
                entry(MF_DISABLED | MF_CHECKED, 0, "Smart charge (80% cap)"),
                entry(MF_GRAYED | MF_DISABLED, 0, "Plan: Nitro-Balanced"),
                separator(),
                entry(MF_STRING, MENU_QUIT, "Quit"),
            ]
        );
    }

    #[test]
    fn menu_exact_degraded_battery_view() {
        let v = TrayView {
            power: PowerState::Battery,
            percent: 42,
            profile: Some(Profile::Eco),
            degraded: true,
            eco_disabled: false,
            smart_charge: None,
            plan: Some("Nitro-Eco".to_string()),
            status: Some("Applied".to_string()),
            start_at_logon: true,
        };
        assert_eq!(
            menu_items(&v),
            vec![
                entry(MF_STRING, MENU_LOGON_TASK, "\u{2611} Start at logon"),
                separator(),
                entry(MF_GRAYED | MF_DISABLED, 0, "Hardware unavailable"),
                separator(),
                entry(
                    MF_GRAYED | MF_DISABLED | MFT_RADIOCHECK | MF_CHECKED,
                    MENU_PROFILE_BASE,
                    "Eco"
                ),
                entry(MF_GRAYED | MF_DISABLED | MFT_RADIOCHECK, MENU_PROFILE_BASE + 1, "Balanced"),
                separator(),
                entry(MF_DISABLED | MF_CHECKED, 0, "Smart charge (80% cap)"),
                entry(MF_GRAYED | MF_DISABLED, 0, "Plan: Nitro-Eco"),
                separator(),
                entry(MF_GRAYED | MF_DISABLED, 0, "Windows plan"),
                entry(MFT_RADIOCHECK | MF_CHECKED, MENU_PLAN_BASE, "Eco"),
                entry(MFT_RADIOCHECK, MENU_PLAN_BASE + 1, "Balanced"),
                separator(),
                entry(MF_STRING, MENU_QUIT, "Quit"),
                separator(),
                entry(MF_GRAYED | MF_DISABLED, 0, "Applied"),
            ]
        );
    }

    #[test]
    fn menu_smart_charge_checked_unless_readback_says_off() {
        for (readback, expect_checked) in [(None, true), (Some(true), true), (Some(false), false)] {
            let mut v = view(PowerState::Ac, 50, None, None);
            v.smart_charge = readback;
            let item = menu_items(&v)
                .into_iter()
                .find(|i| i.label == "Smart charge (80% cap)")
                .expect("smart-charge line present");
            assert_eq!(item.id, 0, "readback {readback:?}");
            assert!(item.flags & MF_DISABLED != 0, "readback {readback:?}");
            assert_eq!(
                item.flags & MF_CHECKED != 0,
                expect_checked,
                "readback {readback:?}"
            );
        }
    }

    #[test]
    fn menu_eco_disabled_greys_eco_entry_only_when_present() {
        let mut battery = view(PowerState::Battery, 40, Some(Profile::Eco), None);
        battery.eco_disabled = true;
        let items = menu_items(&battery);
        let eco = items.iter().find(|i| i.label == "Eco").expect("eco offered on battery");
        assert!(eco.flags & MF_GRAYED != 0);
        assert!(eco.flags & MF_DISABLED != 0);
        let balanced = items.iter().find(|i| i.label == "Balanced").expect("balanced offered");
        assert_eq!(balanced.flags & MF_GRAYED, 0, "only eco is greyed");

        let mut ac = view(PowerState::Ac, 60, Some(Profile::Quiet), None);
        ac.eco_disabled = true;
        let items = menu_items(&ac);
        assert!(
            items.iter().all(|i| i.label != "Eco"),
            "eco is not offered on AC"
        );
        for item in items {
            assert_eq!(
                item.flags & MF_GRAYED,
                0,
                "no greyed item on AC with eco_disabled: {item:?}"
            );
        }
    }

    #[test]
    fn menu_plan_line_only_when_plan_known() {
        let mut v = view(PowerState::Ac, 50, Some(Profile::Quiet), Some("Nitro-Quiet"));
        assert!(menu_items(&v).iter().any(|i| i.label == "Plan: Nitro-Quiet"));
        v.plan = None;
        assert!(!menu_items(&v).iter().any(|i| i.label.starts_with("Plan: ")));
    }

    #[test]
    fn menu_status_line_preceded_by_separator() {
        let mut v = view(PowerState::Ac, 50, None, None);
        assert!(!menu_items(&v).iter().any(|i| i.label == "Applied"));
        v.status = Some("Applied".to_string());
        let items = menu_items(&v);
        let idx = items.iter().position(|i| i.label == "Applied").expect("status line present");
        assert_eq!(items[idx - 1], separator());
        assert_eq!(items[idx].id, 0);
        assert!(items[idx].flags & MF_GRAYED != 0);
        assert!(items[idx].flags & MF_DISABLED != 0);
    }

    #[test]
    fn menu_logon_glyph_follows_flag() {
        let mut v = view(PowerState::Ac, 50, None, None);
        v.start_at_logon = false;
        assert_eq!(
            menu_items(&v)[0],
            entry(MF_STRING, MENU_LOGON_TASK, "\u{2610} Start at logon")
        );
        v.start_at_logon = true;
        assert_eq!(
            menu_items(&v)[0],
            entry(MF_STRING, MENU_LOGON_TASK, "\u{2611} Start at logon")
        );
    }

    #[test]
    fn menu_quit_is_last_item() {
        let v = view(PowerState::Ac, 50, None, None);
        let items = menu_items(&v);
        assert_eq!(
            items[items.len() - 1],
            entry(MF_STRING, MENU_QUIT, "Quit")
        );
    }

    #[test]
    fn menu_unknown_profile_checks_no_profile_item() {
        let v = view(PowerState::Ac, 50, None, None);
        for item in menu_items(&v) {
            let is_profile = ["Quiet", "Balanced", "Performance", "Eco"].contains(&item.label.as_str());
            if is_profile {
                assert_eq!(
                    item.flags & MF_CHECKED,
                    0,
                    "no checkmark expected in {item:?}"
                );
            }
        }
    }
}
