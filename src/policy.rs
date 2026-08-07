//! Pure policy engine — the primary tested seam of the app.
//!
//! Given the current power state, the config, and the user's persisted
//! profile picks, it computes the exact intended target state. It owns the
//! AC/battery profile lists, forward-wrap cycling, eco acceptance/fallback,
//! and per-power-state persistence of manual picks. No OS/hardware calls.

use crate::config::Config;

/// Whether the machine is on AC power or battery.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PowerState {
    Ac,
    Battery,
}

/// The four user-visible profiles. Eco is battery-only; turbo is unused.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Profile {
    Quiet,
    Balanced,
    Performance,
    Eco,
}

impl Profile {
    /// Stable string form, used by config, state file, and display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::Quiet => "quiet",
            Profile::Balanced => "balanced",
            Profile::Performance => "performance",
            Profile::Eco => "eco",
        }
    }

    /// Parse a config/state-file value; unknown names return `None`.
    pub fn from_config_str(s: &str) -> Option<Profile> {
        match s {
            "quiet" => Some(Profile::Quiet),
            "balanced" => Some(Profile::Balanced),
            "performance" => Some(Profile::Performance),
            "eco" => Some(Profile::Eco),
            _ => None,
        }
    }

    /// The Nitro Windows power plan for this profile.
    pub fn plan_name(&self) -> &'static str {
        match self {
            Profile::Quiet => "Nitro-Quiet",
            Profile::Balanced => "Nitro-Balanced",
            Profile::Performance => "Nitro-Performance",
            Profile::Eco => "Nitro-Eco",
        }
    }

    /// Acer firmware platform profile value (spec: quiet 0, balanced 1,
    /// performance 4, eco 6).
    pub fn firmware_value(&self) -> u32 {
        match self {
            Profile::Quiet => 0,
            Profile::Balanced => 1,
            Profile::Performance => 4,
            Profile::Eco => 6,
        }
    }

    /// Acer HID usage mode for this profile (eco drives Quiet mode).
    pub fn hid_mode(&self) -> HidMode {
        match self {
            Profile::Quiet | Profile::Eco => HidMode::Quiet,
            Profile::Balanced => HidMode::Normal,
            Profile::Performance => HidMode::Performance,
        }
    }
}

/// Acer HID system-usage mode (prior art values: Performance=1, Normal=2, Quiet=3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HidMode {
    Quiet,
    Normal,
    Performance,
}

/// Profiles offered per power state, position-bound for the menu.
pub const AC_PROFILES: [Profile; 3] = [Profile::Quiet, Profile::Balanced, Profile::Performance];
pub const BATTERY_PROFILES: [Profile; 2] = [Profile::Eco, Profile::Balanced];

/// The exact intended target state for one enforcement action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntendedState {
    /// Acer firmware platform profile value; `None` when the profile's
    /// firmware entry is unavailable (eco rejected on this machine).
    pub firmware_profile: Option<u32>,
    /// Acer HID usage mode.
    pub hid_mode: HidMode,
    /// Fan behavior is always auto.
    pub fan_auto: bool,
    /// Smart-charge (80% cap) intent.
    pub smart_charge: bool,
    /// Target Nitro plan; `None` for firmware-only (reapply) intents.
    pub plan: Option<&'static str>,
}

/// Pure decision engine. Persistence of picks beyond the session is owned by
/// the app core (state file); the engine only keeps them in memory.
#[derive(Clone, Debug)]
pub struct PolicyEngine {
    ac_pick: Profile,
    battery_pick: Profile,
}

impl PolicyEngine {
    /// Start from config defaults (validated; invalid names fall back to the
    /// spec defaults: AC balanced, battery eco).
    pub fn new(config: &Config) -> Self {
        let _ = config;
        todo!("ticket 03: implement")
    }

    /// The current pick for a power state (config default until overridden).
    pub fn profile_for(&self, state: PowerState) -> Profile {
        let _ = state;
        todo!("ticket 03: implement")
    }

    /// Record a manual pick for a power state (persisted by the app core).
    pub fn set_profile(&mut self, state: PowerState, profile: Profile) {
        let _ = (state, profile);
        todo!("ticket 03: implement")
    }

    /// Profiles offered for the current power state, in menu order.
    pub fn profile_list(&self, state: PowerState) -> &'static [Profile] {
        let _ = state;
        todo!("ticket 03: implement")
    }

    /// Forward-wrap cycle through the current power state's list; applies and
    /// returns the resulting profile.
    pub fn cycle(&mut self, state: PowerState) -> Profile {
        let _ = state;
        todo!("ticket 03: implement")
    }

    /// Current picks, `(ac, battery)` — used by the app core to persist.
    pub fn picks(&self) -> (Profile, Profile) {
        todo!("ticket 03: implement")
    }

    /// Full intended target for a power state: firmware profile (eco -> 6
    /// when accepted, else `None`), HID mode, fan auto, smart charge, and the
    /// matching Nitro plan.
    pub fn intended(&self, state: PowerState, eco_accepted: bool) -> IntendedState {
        let _ = (state, eco_accepted);
        todo!("ticket 03: implement")
    }

    /// Firmware-only intended target for the reapply loop: identical to
    /// `intended` but `plan` is always `None`.
    pub fn reapply_intended(&self, state: PowerState, eco_accepted: bool) -> IntendedState {
        let _ = (state, eco_accepted);
        todo!("ticket 03: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 03: exact intended targets for representative inputs; defaults;
    // per-power-state persistence; cycling order and wrap; eco acceptance vs
    // disabled; reapply never includes the plan.
}
