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
/// the app core (state file); the engine only keeps them in memory. The
/// smart-charge intent is remembered here so the tray toggle (ticket 10) can
/// mutate it in place.
#[derive(Clone, Debug)]
pub struct PolicyEngine {
    ac_pick: Profile,
    battery_pick: Profile,
    smart_charge: bool,
}

impl PolicyEngine {
    /// Start from config defaults (validated; invalid names fall back to the
    /// spec defaults: AC balanced, battery eco).
    pub fn new(config: &Config) -> Self {
        PolicyEngine {
            ac_pick: Profile::from_config_str(&config.ac_profile).unwrap_or(Profile::Balanced),
            battery_pick: Profile::from_config_str(&config.battery_profile).unwrap_or(Profile::Eco),
            smart_charge: config.smart_charge,
        }
    }

    /// The current pick for a power state (config default until overridden).
    pub fn profile_for(&self, state: PowerState) -> Profile {
        match state {
            PowerState::Ac => self.ac_pick,
            PowerState::Battery => self.battery_pick,
        }
    }

    /// Record a manual pick for a power state (persisted by the app core).
    pub fn set_profile(&mut self, state: PowerState, profile: Profile) {
        match state {
            PowerState::Ac => self.ac_pick = profile,
            PowerState::Battery => self.battery_pick = profile,
        }
    }

    /// Profiles offered for the current power state, in menu order.
    pub fn profile_list(&self, state: PowerState) -> &'static [Profile] {
        match state {
            PowerState::Ac => &AC_PROFILES,
            PowerState::Battery => &BATTERY_PROFILES,
        }
    }

    /// Forward-wrap cycle through the current power state's list; applies and
    /// returns the resulting profile.
    pub fn cycle(&mut self, state: PowerState) -> Profile {
        let list = self.profile_list(state);
        let current = self.profile_for(state);
        let idx = list.iter().position(|&p| p == current).unwrap_or(0);
        let next = list[(idx + 1) % list.len()];
        self.set_profile(state, next);
        next
    }

    /// Current picks, `(ac, battery)` — used by the app core to persist.
    pub fn picks(&self) -> (Profile, Profile) {
        (self.ac_pick, self.battery_pick)
    }

    /// Full intended target for a power state: firmware profile (eco -> 6
    /// when accepted, else `None`), HID mode, fan auto, smart charge, and the
    /// matching Nitro plan.
    pub fn intended(&self, state: PowerState, eco_accepted: bool) -> IntendedState {
        self.intended_inner(state, eco_accepted, true)
    }

    /// Firmware-only intended target for the reapply loop: identical to
    /// `intended` but `plan` is always `None`.
    pub fn reapply_intended(&self, state: PowerState, eco_accepted: bool) -> IntendedState {
        self.intended_inner(state, eco_accepted, false)
    }

    fn intended_inner(
        &self,
        state: PowerState,
        eco_accepted: bool,
        include_plan: bool,
    ) -> IntendedState {
        let profile = self.profile_for(state);
        IntendedState {
            firmware_profile: match profile {
                Profile::Eco if !eco_accepted => None,
                _ => Some(profile.firmware_value()),
            },
            hid_mode: profile.hid_mode(),
            fan_auto: true,
            smart_charge: self.smart_charge,
            plan: if include_plan { Some(profile.plan_name()) } else { None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn engine() -> PolicyEngine {
        PolicyEngine::new(&Config::default())
    }

    fn with_pick(state: PowerState, profile: Profile) -> PolicyEngine {
        let mut e = engine();
        e.set_profile(state, profile);
        e
    }

    #[test]
    fn defaults_from_config() {
        let e = engine();
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Balanced);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Eco);
        assert_eq!(e.picks(), (Profile::Balanced, Profile::Eco));
    }

    #[test]
    fn config_overrides_default_picks() {
        let cfg = Config {
            ac_profile: "quiet".to_string(),
            battery_profile: "balanced".to_string(),
            ..Config::default()
        };
        let e = PolicyEngine::new(&cfg);
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Quiet);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.picks(), (Profile::Quiet, Profile::Balanced));
    }

    #[test]
    fn invalid_profile_names_fall_back_to_defaults() {
        for (ac, battery) in [
            ("turbo", "unknown"),
            ("performancex", "eco"),
            ("balanced", "quiet"),
            ("", ""),
        ] {
            let cfg = Config {
                ac_profile: ac.to_string(),
                battery_profile: battery.to_string(),
                ..Config::default()
            };
            let e = PolicyEngine::new(&cfg);
            assert_eq!(
                e.profile_for(PowerState::Ac),
                Profile::from_config_str(ac).unwrap_or(Profile::Balanced)
            );
            assert_eq!(
                e.profile_for(PowerState::Battery),
                Profile::from_config_str(battery).unwrap_or(Profile::Eco)
            );
        }
    }

    #[test]
    fn manual_picks_are_per_power_state() {
        let mut e = engine();
        e.set_profile(PowerState::Battery, Profile::Balanced);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Balanced);
        e.set_profile(PowerState::Ac, Profile::Performance);
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Performance);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.picks(), (Profile::Performance, Profile::Balanced));
        e.set_profile(PowerState::Battery, Profile::Eco);
        assert_eq!(e.picks(), (Profile::Performance, Profile::Eco));
    }

    #[test]
    fn profile_lists_are_position_bound() {
        assert_eq!(AC_PROFILES, [Profile::Quiet, Profile::Balanced, Profile::Performance]);
        assert_eq!(BATTERY_PROFILES, [Profile::Eco, Profile::Balanced]);
        assert_eq!(engine().profile_list(PowerState::Ac), &AC_PROFILES[..]);
        assert_eq!(engine().profile_list(PowerState::Battery), &BATTERY_PROFILES[..]);
        assert_eq!(engine().profile_list(PowerState::Ac).len(), 3);
        assert_eq!(engine().profile_list(PowerState::Battery).len(), 2);
    }

    #[test]
    fn intended_exact_targets_all_profiles() {
        let cases = [
            (Profile::Quiet, Some(0), HidMode::Quiet, "Nitro-Quiet"),
            (Profile::Balanced, Some(1), HidMode::Normal, "Nitro-Balanced"),
            (Profile::Performance, Some(4), HidMode::Performance, "Nitro-Performance"),
            (Profile::Eco, Some(6), HidMode::Quiet, "Nitro-Eco"),
        ];
        for (profile, fw, hid, plan) in cases {
            for state in [PowerState::Ac, PowerState::Battery] {
                for eco_accepted in [true, false] {
                    let want = IntendedState {
                        firmware_profile: if profile == Profile::Eco && !eco_accepted {
                            None
                        } else {
                            fw
                        },
                        hid_mode: hid,
                        fan_auto: true,
                        smart_charge: true,
                        plan: Some(plan),
                    };
                    assert_eq!(
                        with_pick(state, profile).intended(state, eco_accepted),
                        want,
                        "{profile:?} on {state:?}, eco_accepted={eco_accepted}"
                    );
                }
            }
        }
    }

    #[test]
    fn eco_accepted_uses_firmware_6_else_none_but_plan_holds() {
        let e = with_pick(PowerState::Battery, Profile::Eco);
        let accepted = e.intended(PowerState::Battery, true);
        assert_eq!(accepted.firmware_profile, Some(6));
        assert_eq!(accepted.hid_mode, HidMode::Quiet);
        assert!(accepted.fan_auto);
        assert!(accepted.smart_charge);
        assert_eq!(accepted.plan, Some("Nitro-Eco"));

        let rejected = e.intended(PowerState::Battery, false);
        assert_eq!(rejected.firmware_profile, None);
        assert_eq!(rejected.hid_mode, HidMode::Quiet);
        assert!(rejected.fan_auto);
        assert!(rejected.smart_charge);
        assert_eq!(rejected.plan, Some("Nitro-Eco"));
    }

    #[test]
    fn smart_charge_intent_flows_from_config() {
        for smart_charge in [true, false] {
            let cfg = Config {
                smart_charge,
                ac_profile: "performance".to_string(),
                ..Config::default()
            };
            let e = PolicyEngine::new(&cfg);
            assert_eq!(e.intended(PowerState::Ac, true).smart_charge, smart_charge);
            assert_eq!(e.intended(PowerState::Battery, false).smart_charge, smart_charge);
            assert_eq!(e.reapply_intended(PowerState::Ac, true).smart_charge, smart_charge);
        }
    }

    #[test]
    fn cycle_ac_forward_wraps() {
        let cfg = Config {
            ac_profile: "quiet".to_string(),
            ..Config::default()
        };
        let mut e = PolicyEngine::new(&cfg);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Balanced);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Performance);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Quiet);
        assert_eq!(e.picks(), (Profile::Quiet, Profile::Eco));
    }

    #[test]
    fn cycle_ac_from_middle_pick() {
        let mut e = with_pick(PowerState::Ac, Profile::Balanced);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Performance);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Quiet);
        assert_eq!(e.cycle(PowerState::Ac), Profile::Balanced);
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Balanced);
    }

    #[test]
    fn cycle_battery_forward_wraps() {
        let mut e = engine();
        assert_eq!(e.cycle(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.cycle(PowerState::Battery), Profile::Eco);
        assert_eq!(e.cycle(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.picks(), (Profile::Balanced, Profile::Balanced));
    }

    #[test]
    fn cycle_battery_from_balanced_pick() {
        let mut e = with_pick(PowerState::Battery, Profile::Balanced);
        assert_eq!(e.cycle(PowerState::Battery), Profile::Eco);
        assert_eq!(e.cycle(PowerState::Battery), Profile::Balanced);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Balanced);
    }

    #[test]
    fn cycle_does_not_touch_other_power_state() {
        let mut e = engine();
        e.cycle(PowerState::Ac);
        assert_eq!(e.profile_for(PowerState::Battery), Profile::Eco);
        e.cycle(PowerState::Battery);
        assert_eq!(e.profile_for(PowerState::Ac), Profile::Performance);
    }

    #[test]
    fn reapply_intended_matches_intended_except_plan() {
        for state in [PowerState::Ac, PowerState::Battery] {
            for profile in [
                Profile::Quiet,
                Profile::Balanced,
                Profile::Performance,
                Profile::Eco,
            ] {
                for eco_accepted in [true, false] {
                    let e = with_pick(state, profile);
                    let mut want = e.intended(state, eco_accepted);
                    want.plan = None;
                    assert_eq!(
                        e.reapply_intended(state, eco_accepted),
                        want,
                        "{profile:?} on {state:?}, eco_accepted={eco_accepted}"
                    );
                    assert_eq!(e.reapply_intended(state, eco_accepted).plan, None);
                }
            }
        }
    }

    #[test]
    fn reapply_eco_rejected_keeps_firmware_none_and_no_plan() {
        let e = with_pick(PowerState::Battery, Profile::Eco);
        let want = IntendedState {
            firmware_profile: None,
            hid_mode: HidMode::Quiet,
            fan_auto: true,
            smart_charge: true,
            plan: None,
        };
        assert_eq!(e.reapply_intended(PowerState::Battery, false), want);
    }
}
