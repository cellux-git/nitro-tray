//! Win32 tray plumbing: static icon in the system tray, left-click opens the
//! menu, menu and tooltip show the *read-back* effective state, balloon
//! notifications, and a degraded "Hardware unavailable" state when the Acer
//! WMI interface is unreachable. The tray window also owns the
//! power-notification and timer plumbing that feeds the app core. The pure
//! menu model (view, neutral items, tooltip text, id constants) lives in
//! `tray_model.rs` — shared, both platforms.

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
use crate::tray_model::{
    menu_items, plans_for, tooltip_text, MenuItem, TrayEvent, TrayView, MENU_LOGON_TASK,
    MENU_PLAN_BASE, MENU_PROFILE_BASE, MENU_QUIT, SMART_CHARGE_LABEL,
};

/// Hidden window class (tray icon owner, power notifications, poll timer).
/// The class name is a `w!` literal (the macro takes literals only).
const ICON_ID: u32 = 1;
/// uCallbackMessage: the shell posts mouse messages here.
const TRAY_MSG: u32 = WM_APP + 1;
/// Posted after every channel send to wake the GetMessageW pump.
const WAKE_MSG: u32 = WM_APP + 2;
const POLL_TIMER_ID: usize = 1;
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
                    profiles_greyed: false,
                    plan_section: false,
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
        let flags = append_flags(item, profiles.len(), plans.len());
        append_item(menu, flags, item.id, &item.label);
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

/// Win32 `AppendMenuW` flags for one neutral menu item: separators map to
/// `MF_SEPARATOR`; everything else is `MF_STRING` plus `MF_DISABLED` when
/// disabled, `MF_CHECKED` when checked, `MFT_RADIOCHECK` when the id falls
/// in the profile or plan range, and `MF_GRAYED` for disabled items that are
/// neither radio nor checked — except the smart-charge status line, which is
/// disabled but stays black (`MF_DISABLED` only, exactly as before) even
/// when its readback says off and the checkmark is absent. The label keys
/// that one special case (the neutral model cannot distinguish the line).
/// Pure — the unit test pins the mapping byte-identical to the pre-ticket
/// menu.
fn append_flags(item: &MenuItem, profile_count: usize, plan_count: usize) -> u32 {
    if item.separator {
        return MF_SEPARATOR;
    }
    let radio = (MENU_PROFILE_BASE..MENU_PROFILE_BASE + profile_count).contains(&item.id)
        || (MENU_PLAN_BASE..MENU_PLAN_BASE + plan_count).contains(&item.id);
    let greyed = !item.enabled
        && (radio || !item.checked)
        && item.label != SMART_CHARGE_LABEL;
    MF_STRING
        | if item.enabled { 0 } else { MF_DISABLED }
        | if greyed { MF_GRAYED } else { 0 }
        | if item.checked { MF_CHECKED } else { 0 }
        | if radio { MFT_RADIOCHECK } else { 0 }
}

fn append_item(menu: HMENU, flags: u32, id: usize, label: &str) {
    let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
    unsafe {
        AppendMenuW(menu, flags, id, wide.as_ptr());
    }
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

    /// The append-time `MF_*`/`MFT_*` mapping, pinned byte-identical to the
    /// pre-ticket Windows menu: the degraded battery view (every item kind
    /// present) with the exact `AppendMenuW` flag bits per item.
    #[test]
    fn append_flags_byte_identical_to_pre_ticket_menu() {
        let v = TrayView {
            power: PowerState::Battery,
            percent: 42,
            profile: Some(Profile::Eco),
            degraded: true,
            profiles_greyed: true,
            plan_section: true,
            eco_disabled: false,
            smart_charge: None,
            plan: Some("Nitro-Eco".to_string()),
            status: Some("Applied".to_string()),
            start_at_logon: true,
        };
        let items = menu_items(&v);
        let flags: Vec<u32> = items
            .iter()
            .map(|item| append_flags(item, profiles_for(v.power).len(), plans_for(&v).len()))
            .collect();
        assert_eq!(
            flags,
            vec![
                MF_STRING, // logon checkbox
                MF_SEPARATOR, //
                MF_GRAYED | MF_DISABLED, // "Hardware unavailable" banner
                MF_SEPARATOR, //
                MFT_RADIOCHECK | MF_CHECKED | MF_GRAYED | MF_DISABLED, // eco (greyed + checked)
                MFT_RADIOCHECK | MF_GRAYED | MF_DISABLED, // balanced (greyed)
                MF_SEPARATOR, //
                MF_DISABLED | MF_CHECKED, // smart charge (checked)
                MF_GRAYED | MF_DISABLED, // plan line
                MF_SEPARATOR, //
                MF_GRAYED | MF_DISABLED, // "Windows plan" header
                MFT_RADIOCHECK | MF_CHECKED, // plan eco (checked)
                MFT_RADIOCHECK, // plan balanced
                MF_SEPARATOR, //
                MF_STRING, // quit
                MF_SEPARATOR, //
                MF_GRAYED | MF_DISABLED, // status line
            ]
        );
    }

    #[test]
    fn append_flags_smart_charge_off_stays_disabled_not_greyed() {
        // Pre-ticket: the smart-charge line is `MF_DISABLED` only (black,
        // unclickable, no checkmark) when the readback says off — greyed
        // would be a visual change, and every other disabled line (banner,
        // plan line, "Windows plan" header, status) greys.
        let v = TrayView {
            power: PowerState::Ac,
            percent: 50,
            profile: None,
            degraded: true,
            profiles_greyed: true,
            plan_section: true,
            eco_disabled: false,
            smart_charge: Some(false),
            plan: Some("Nitro-Quiet".to_string()),
            status: Some("Applied".to_string()),
            start_at_logon: false,
        };
        let items = menu_items(&v);
        let smart = items
            .iter()
            .find(|item| item.label == SMART_CHARGE_LABEL)
            .expect("smart-charge line present");
        assert!(!smart.checked);
        assert_eq!(
            append_flags(smart, profiles_for(v.power).len(), plans_for(&v).len()),
            MF_DISABLED,
            "smart-charge off: disabled but not greyed (pre-ticket 0x2)"
        );
        for item in items {
            let flags = append_flags(&item, profiles_for(v.power).len(), plans_for(&v).len());
            if item.label != SMART_CHARGE_LABEL && flags & MF_DISABLED != 0 && flags & MFT_RADIOCHECK == 0 {
                assert!(
                    flags & MF_GRAYED != 0,
                    "every other disabled non-radio item greys: {item:?} -> {flags:#x}"
                );
            }
        }
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
