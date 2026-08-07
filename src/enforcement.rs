//! Enforcement triggers: startup, AC <-> battery transitions, resume/wake.
//! Maps tray/power events to `AppCore` enforcement calls. All automatic
//! switching is silent (no notifications). Works with Acer's user-mode
//! services stopped or uninstalled (verify on device).

use crate::app::AppCore;

/// Startup: ensure the four Nitro plans exist, then enforce the intended
/// state for the current power state.
pub fn on_startup(app: &mut AppCore) {
    let _ = app;
    todo!("ticket 12: implement")
}

/// AC <-> battery transition (only when `auto_switch` is enabled in config):
/// silently enforce the intended state for the new power state.
pub fn on_power_changed(app: &mut AppCore) {
    let _ = app;
    todo!("ticket 12: implement")
}

/// Resume/wake: re-enforce after firmware or OS resets.
pub fn on_resume(app: &mut AppCore) {
    let _ = app;
    todo!("ticket 12: implement")
}

#[cfg(test)]
mod tests {
    // ticket 12: whatever is testable without the OS layer.
}
