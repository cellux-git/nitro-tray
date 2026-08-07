//! Application core: wires config + policy engine + adapters. Owns the apply
//! path (firmware profile, HID usage mode, fan auto, smart charge, plan),
//! runtime eco acceptance detection, per-power-state pick persistence
//! (`nitro-tray.state.toml` beside the exe), and the read-back effective
//! state. Degrades gracefully when the Acer WMI interface is unreachable.

use std::path::Path;

use crate::config::Config;
use crate::policy::{PowerState, Profile};

/// Filename of the persisted user state, beside the exe.
pub const STATE_FILE_NAME: &str = "nitro-tray.state.toml";

/// Read-back effective state for the tray (hardware/OS truth, not intent).
#[derive(Clone, Debug)]
pub struct EffectiveState {
    pub power: PowerState,
    pub percent: u8,
    /// Active firmware profile; `None` when unreadable.
    pub profile: Option<Profile>,
    /// Active Windows plan name.
    pub plan: Option<String>,
    /// Read-back smart-charge state.
    pub smart_charge: Option<bool>,
    /// Acer WMI interface reachable?
    pub wmi_available: bool,
    /// Eco entry disabled (firmware rejected profile 6).
    pub eco_disabled: bool,
}

pub struct AppCore {
    // opaque
}

impl AppCore {
    /// Construct the core: parse config-backed picks, load the state file,
    /// connect adapters (WMI/HID/charge failures degrade, never crash).
    pub fn new(config: Config, exe_dir: &Path) -> Self {
        let _ = (config, exe_dir);
        todo!("ticket 09: implement")
    }

    /// Read the current effective state back from hardware/OS.
    pub fn effective(&self) -> EffectiveState {
        todo!("ticket 09: implement")
    }

    /// Apply a profile immediately (full apply), persist the per-power-state
    /// pick, run eco runtime detection when the pick is eco.
    pub fn apply_profile(&mut self, profile: Profile) {
        let _ = profile;
        todo!("ticket 09: implement")
    }

    /// Cycle forward through the current power state's list, apply, persist;
    /// returns the new profile.
    pub fn cycle_profile(&mut self) -> Profile {
        todo!("ticket 09: implement")
    }

    /// Toggle smart charge: apply, update intent (persisted so startup
    /// enforcement keeps the choice).
    pub fn toggle_smart_charge(&mut self) {
        todo!("ticket 09: implement")
    }

    /// Ensure the four Nitro plans exist (recreate deleted ones).
    pub fn ensure_nitro_plans(&mut self) {
        todo!("ticket 09: implement")
    }

    /// Full enforcement for the current power state, silently: ensure plans,
    /// then apply the intended state (profile, HID, fan auto, smart charge,
    /// plan). Used at startup, on power transitions, and on resume.
    pub fn enforce_now(&mut self) {
        todo!("ticket 09: implement")
    }

    /// Firmware-only re-assertion (reapply loop): WMI profile, HID mode, fan
    /// auto, smart-charge state — never the active plan.
    pub fn reapply_firmware(&mut self) {
        todo!("ticket 09: implement")
    }

    /// Re-run eco acceptance detection (called on power transitions, on
    /// reapply ticks, and after the first eco attempt).
    pub fn re_evaluate_eco(&mut self) {
        todo!("ticket 09: implement")
    }

    /// Acer WMI interface reachable?
    pub fn wmi_available(&self) -> bool {
        todo!("ticket 09: implement")
    }

    /// Eco entry disabled (firmware rejected profile 6)?
    pub fn eco_disabled(&self) -> bool {
        todo!("ticket 09: implement")
    }

    /// The current pick for a power state (for menu check marks).
    pub fn profile_for(&self, state: PowerState) -> Profile {
        let _ = state;
        todo!("ticket 09: implement")
    }

    /// Current power state (last known snapshot).
    pub fn current_power(&self) -> PowerState {
        todo!("ticket 09: implement")
    }

    /// Intended smart-charge state (config default, mutated by the toggle).
    pub fn smart_charge_intent(&self) -> bool {
        todo!("ticket 09: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 09: state-file round trips, degraded wiring if testable.
}
