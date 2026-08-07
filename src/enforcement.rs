//! Enforcement triggers: startup, AC <-> battery transitions, resume/wake.
//! Maps tray/power events to `AppCore` enforcement calls. All automatic
//! switching is silent (no notifications). Works with Acer's user-mode
//! services stopped or uninstalled (verify on device).

use crate::app::AppCore;
use crate::log;

/// Startup: ensure the four Nitro plans exist, then enforce the intended
/// state for the current power state.
pub fn on_startup(app: &mut AppCore) {
    log::info("enforcement: startup");
    app.ensure_nitro_plans();
    app.enforce_now();
}

/// AC <-> battery transition (only when `auto_switch` is enabled in config):
/// silently enforce the intended state for the new power state.
pub fn on_power_changed(app: &mut AppCore) {
    if !app.auto_switch() {
        log::info("enforcement: power change ignored (auto_switch disabled)");
        return;
    }
    log::info("enforcement: power change; enforcing intended state");
    app.re_evaluate_eco();
    app.enforce_now();
}

/// Resume/wake: re-enforce after firmware or OS resets.
pub fn on_resume(app: &mut AppCore) {
    log::info("enforcement: resume; re-enforcing intended state");
    app.re_evaluate_eco();
    app.enforce_now();
}

#[cfg(test)]
mod tests {
    // All enforcement paths call through `AppCore` into OS/hardware APIs
    // (power plans, WMI, HID, charge); there is no pure logic here to unit
    // test. Coverage for this module is on-device verification (see the
    // ticket's Comments section).
}
