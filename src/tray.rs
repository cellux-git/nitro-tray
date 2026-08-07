//! Tray UI: static icon in the system tray, left-click opens the menu, menu
//! and tooltip show the *read-back* effective state, balloon notifications,
//! and a degraded "Hardware unavailable" state when the Acer WMI interface is
//! unreachable. The tray window also owns the power-notification and timer
//! plumbing that feeds the app core.

use std::sync::mpsc::Sender;

use windows_sys::Win32::Foundation::HWND;

use crate::policy::{PowerState, Profile};

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

pub struct Tray {
    // opaque: hidden window, NOTIFYICONDATA, view state
}

impl Tray {
    /// Create the hidden window + tray icon. Window messages (tray clicks,
    /// WM_POWERBROADCAST, timers) raise `TrayEvent`s on `event_tx`.
    pub fn create(event_tx: Sender<TrayEvent>) -> Result<Self, TrayError> {
        let _ = event_tx;
        todo!("ticket 08: implement")
    }

    /// The hidden window handle (used for RegisterHotKey / timers).
    pub fn hwnd(&self) -> HWND {
        todo!("ticket 08: implement")
    }

    /// Push a new view: rebuild menu items (applied on next open) and the
    /// tooltip. Tooltip shows power state, battery %, active profile, plan,
    /// and smart-charge state; degraded state when `view.degraded`.
    pub fn update(&self, view: &TrayView) -> Result<(), TrayError> {
        let _ = view;
        todo!("ticket 08: implement")
    }

    /// Show a transient balloon notification (hotkey feedback only; automatic
    /// switching stays silent).
    pub fn notify(&self, title: &str, body: &str) {
        let _ = (title, body);
        todo!("ticket 08: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 08: view/display formatting helpers if any.
}
