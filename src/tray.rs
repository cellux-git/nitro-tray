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
    CreateDIBSection, DeleteObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP,
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
    DestroyMenu, DestroyWindow, GetCursorPos, GetWindowLongPtrW, KillTimer, PostMessageW,
    RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, TrackPopupMenu,
    DEVICE_NOTIFY_WINDOW_HANDLE, GWLP_USERDATA, HICON, HMENU, ICONINFO, MF_CHECKED, MF_DISABLED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_TOPALIGN, WNDCLASSW, WM_APP, WM_HOTKEY, WM_LBUTTONUP, WM_POWERBROADCAST, WM_RBUTTONUP,
    WM_TIMER, WS_OVERLAPPEDWINDOW,
};

use crate::hotkey::HOTKEY_ID;
use crate::log;
use crate::policy::{PowerState, Profile};
use crate::power_state::{self, PowerStateSnapshot};
use crate::reapply;

/// Hidden window class (tray icon owner, power notifications, poll timer).
/// The class name is a `w!` literal (the macro takes literals only).
const ICON_ID: u32 = 1;
/// uCallbackMessage: the shell posts mouse messages here.
const TRAY_MSG: u32 = WM_APP + 1;
/// Posted after every channel send to wake the GetMessageW pump.
const WAKE_MSG: u32 = WM_APP + 2;
const POLL_TIMER_ID: usize = 1;
const MENU_PROFILE_BASE: usize = 1;
const MENU_SMART_CHARGE: usize = 100;
const MENU_QUIT: usize = 200;
const ICON_SIZE: i32 = 16;

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
    /// The user picked a profile in the menu.
    SelectProfile(Profile),
    /// The user toggled smart charge.
    ToggleSmartCharge,
    /// Power status changed (WM_POWERBROADCAST or slow poll fallback).
    PowerChanged,
    /// The machine resumed from sleep (WM_POWERBROADCAST).
    Resume,
    /// The global profile-cycle hotkey was pressed (WM_HOTKEY).
    HotkeyPressed,
    /// The reapply timer ticked (WM_TIMER, reapply::TIMER_ID).
    ReapplyTick,
}

/// The view the main loop pushes into the tray after each state change: the
/// read-back effective state plus which menu items to offer/grey out.
#[derive(Clone, Debug)]
pub struct TrayView {
    pub power: PowerState,
    pub percent: u8,
    /// Current effective profile (checked in the menu); `None` when unknown.
    pub profile: Option<Profile>,
    /// Profiles offered for the current power state, in menu order.
    pub profiles: Vec<Profile>,
    /// Grey out the profile items (degraded: WMI unavailable).
    pub profiles_greyed: bool,
    /// Read-back smart-charge state; `None` when unavailable.
    pub smart_charge: Option<bool>,
    /// Grey out the smart-charge item (degraded).
    pub smart_charge_greyed: bool,
    /// Active Windows plan name; `None` when unknown.
    pub plan: Option<String>,
    /// Show the degraded "Hardware unavailable" state.
    pub degraded: bool,
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
                    profiles: Vec::new(),
                    profiles_greyed: false,
                    smart_charge: None,
                    smart_charge_greyed: false,
                    plan: None,
                    degraded: false,
                }),
                last_snapshot: RefCell::new(power_state::read()),
                event_tx,
                channel_closed: Cell::new(false),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*state as *const TrayState as isize);

            let Some((icon, bitmap)) = make_battery_icon() else {
                destroy_window(hwnd);
                return Err(TrayError::Create("icon creation failed"));
            };
            let nid = nid_template(hwnd, icon);
            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                destroy_icon_assets(icon, bitmap);
                destroy_window(hwnd);
                return Err(TrayError::Create("Shell_NotifyIconW NIM_ADD failed"));
            }

            if SetTimer(hwnd, POLL_TIMER_ID, power_state::SLOW_POLL_MS, None) == 0 {
                let nid = nid_template(hwnd, ptr::null_mut());
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                destroy_icon_assets(icon, bitmap);
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
            destroy_icon_assets(self.icon, self.bitmap);
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
            reapply::TIMER_ID => send_event(state, TrayEvent::ReapplyTick),
            _ => {}
        },
        WM_HOTKEY => {
            if wparam as i32 == HOTKEY_ID {
                send_event(state, TrayEvent::HotkeyPressed);
            }
        }
        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMPOWERSTATUSCHANGE => send_event(state, TrayEvent::PowerChanged),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => send_event(state, TrayEvent::Resume),
                PBT_POWERSETTINGCHANGE => poll_power(state),
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
/// the last snapshot and raise `PowerChanged` on any difference.
fn poll_power(state: &TrayState) {
    let now = power_state::read();
    let changed = *state.last_snapshot.borrow() != now;
    *state.last_snapshot.borrow_mut() = now;
    if changed {
        send_event(state, TrayEvent::PowerChanged);
    }
}

/// Build the popup menu from the stored view and route the picked id.
fn open_menu(hwnd: HWND, state: &TrayState) {
    let view = state.view.borrow().clone();
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        log::warn("CreatePopupMenu failed");
        return;
    }
    if view.degraded {
        append_item(menu, MF_GRAYED | MF_DISABLED, 0, "Hardware unavailable");
        append_separator(menu);
    }
    let profile_flags = if view.profiles_greyed {
        MF_GRAYED | MF_DISABLED
    } else {
        0
    };
    for (i, profile) in view.profiles.iter().enumerate() {
        let flags = profile_flags
            | if Some(*profile) == view.profile {
                MF_CHECKED
            } else {
                0
            };
        append_item(menu, flags, MENU_PROFILE_BASE + i, profile_label(*profile));
    }
    append_separator(menu);
    let mut smart_flags = if view.smart_charge_greyed {
        MF_GRAYED | MF_DISABLED
    } else {
        0
    };
    if view.smart_charge == Some(true) {
        smart_flags |= MF_CHECKED;
    }
    append_item(menu, smart_flags, MENU_SMART_CHARGE, "Smart charge (80% cap)");
    if let Some(plan) = &view.plan {
        append_item(menu, MF_GRAYED | MF_DISABLED, 0, &format!("Plan: {plan}"));
    }
    append_separator(menu);
    append_item(menu, MF_STRING, MENU_QUIT, "Quit");

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
        MENU_SMART_CHARGE => send_event(state, TrayEvent::ToggleSmartCharge),
        id if id >= MENU_PROFILE_BASE && id < MENU_PROFILE_BASE + view.profiles.len() => {
            if let Some(&profile) = view.profiles.get(id - MENU_PROFILE_BASE) {
                send_event(state, TrayEvent::SelectProfile(profile));
            }
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

/// 16x16 battery glyph drawn into a 32bpp top-down DIB: dark outline, green
/// fill with a brighter charge half, and a terminal nub. The bitmap handle is
/// kept alive by the caller for the icon's lifetime.
fn make_battery_icon() -> Option<(HICON, HBITMAP)> {
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

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: ptr::null_mut(),
            hbmColor: bitmap,
        };
        let icon = CreateIconIndirect(&info);
        if icon.is_null() {
            DeleteObject(bitmap as _);
            return None;
        }
        Some((icon, bitmap))
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

fn destroy_icon_assets(icon: HICON, bitmap: HBITMAP) {
    unsafe {
        DestroyIcon(icon);
        if !bitmap.is_null() {
            DeleteObject(bitmap as _);
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
            profiles: Vec::new(),
            profiles_greyed: false,
            smart_charge: None,
            smart_charge_greyed: false,
            plan: plan.map(String::from),
            degraded: false,
        }
    }

    #[test]
    fn tooltip_contains_all_read_back_values() {
        let v = TrayView {
            power: PowerState::Ac,
            percent: 87,
            profile: Some(Profile::Balanced),
            profiles: Vec::new(),
            profiles_greyed: false,
            smart_charge: Some(true),
            smart_charge_greyed: false,
            plan: Some("Nitro-Balanced".to_string()),
            degraded: false,
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
}
