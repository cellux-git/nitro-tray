//! Application core: wires config + policy engine + adapters. Owns the apply
//! path (firmware profile, HID usage mode, fan auto, smart charge, plan),
//! runtime eco acceptance detection, per-power-state pick persistence
//! (`nitro-tray.state.toml` beside the exe), and the read-back effective
//! state. Degrades gracefully when the Acer WMI interface is unreachable.

use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::charge::SmartChargeAdapter;
use crate::config::Config;
use crate::hid::HidAdapter;
use crate::log;
use crate::policy::{IntendedState, PolicyEngine, PowerState, Profile};
use crate::power::PowerApi;
use crate::power_state::{self, PowerStateSnapshot};
use crate::wmi::{self, WmiAdapter};

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

/// Log each read-back failure kind at most once per process run.
static LOGGED_WMI_PROFILE_READ: Once = Once::new();
static LOGGED_PLAN_READ: Once = Once::new();
static LOGGED_CHARGE_READ: Once = Once::new();

pub struct AppCore {
    engine: PolicyEngine,
    auto_switch: bool,
    wmi: Option<WmiAdapter>,
    charge: Option<SmartChargeAdapter>,
    hid: Option<HidAdapter>,
    eco_accepted: Option<bool>,
    last_power: PowerStateSnapshot,
    state_path: PathBuf,
}

impl AppCore {
    /// Construct the core: parse config-backed picks, load the state file,
    /// connect adapters (WMI/HID/charge failures degrade, never crash).
    pub fn new(config: Config, exe_dir: &Path) -> Self {
        let mut engine = PolicyEngine::new(&config);

        let wmi = match WmiAdapter::connect() {
            Ok(adapter) => Some(adapter),
            Err(err) => {
                log::warn(format!("wmi: adapter unavailable; running degraded: {err:?}"));
                None
            }
        };
        let charge = match SmartChargeAdapter::connect() {
            Ok(adapter) => Some(adapter),
            Err(err) => {
                log::warn(format!("charge: smart-charge adapter unavailable: {err:?}"));
                None
            }
        };
        let hid = match HidAdapter::open() {
            Ok(adapter) => Some(adapter),
            Err(err) => {
                log::warn(format!("hid: usage-mode adapter unavailable: {err:?}"));
                None
            }
        };

        let state_path = exe_dir.join(STATE_FILE_NAME);
        match std::fs::read_to_string(&state_path) {
            Ok(contents) => {
                let picks = load_state(&contents);
                if let Some((ac, battery)) = picks {
                    match Profile::from_config_str(&ac) {
                        Some(profile) => engine.set_profile(PowerState::Ac, profile),
                        None => log::warn(format!("state: ignoring invalid ac pick {ac:?}")),
                    }
                    match Profile::from_config_str(&battery) {
                        Some(profile) => engine.set_profile(PowerState::Battery, profile),
                        None => log::warn(format!("state: ignoring invalid battery pick {battery:?}")),
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => log::warn(format!("state: cannot read {}: {err}", state_path.display())),
        }

        AppCore {
            engine,
            auto_switch: config.auto_switch,
            wmi,
            charge,
            hid,
            eco_accepted: None,
            last_power: power_state::read(),
            state_path,
        }
    }

    /// Read the current effective state back from hardware/OS.
    pub fn effective(&self) -> EffectiveState {
        let snapshot = power_state::read();
        let profile = match self.wmi.as_ref() {
            Some(wmi) if wmi.is_available() => match wmi.get_platform_profile() {
                Ok(value) => profile_from_firmware(value),
                Err(err) => {
                    LOGGED_WMI_PROFILE_READ.call_once(|| {
                        log::warn(format!("wmi: platform profile readback failed: {err:?}"));
                    });
                    None
                }
            },
            Some(_) | None => None,
        };
        let plan = match PowerApi::active_plan_name() {
            Ok(name) => Some(name),
            Err(err) => {
                LOGGED_PLAN_READ.call_once(|| {
                    log::warn(format!("power: active plan readback failed: {err:?}"));
                });
                None
            }
        };
        let smart_charge = match self.charge.as_ref() {
            Some(charge) if charge.is_available() => match charge.is_enabled() {
                Ok(enabled) => Some(enabled),
                Err(err) => {
                    LOGGED_CHARGE_READ.call_once(|| {
                        log::warn(format!("charge: smart-charge readback failed: {err:?}"));
                    });
                    None
                }
            },
            Some(_) | None => None,
        };
        EffectiveState {
            power: snapshot.state,
            percent: snapshot.percent,
            profile,
            plan,
            smart_charge,
            wmi_available: self.wmi.as_ref().is_some_and(|wmi| wmi.is_available()),
            eco_disabled: self.eco_accepted == Some(false),
        }
    }

    /// Apply a profile immediately (full apply), persist the per-power-state
    /// pick, run eco runtime detection when the pick is eco.
    pub fn apply_profile(&mut self, profile: Profile) {
        let snapshot = self.refresh_power();
        self.engine.set_profile(snapshot.state, profile);
        self.write_state_file();
        if profile == Profile::Eco {
            self.detect_eco();
        }
        self.apply_full(snapshot.state);
    }

    /// Cycle forward through the current power state's list, apply, persist;
    /// returns the new profile. A disabled eco entry is skipped so the hotkey
    /// can never select a profile the firmware rejects.
    pub fn cycle_profile(&mut self) -> Profile {
        let snapshot = self.refresh_power();
        let mut next = self.engine.cycle(snapshot.state);
        if next == Profile::Eco && self.eco_disabled() {
            next = self.engine.cycle(snapshot.state);
        }
        self.write_state_file();
        if next == Profile::Eco {
            self.detect_eco();
        }
        self.apply_full(snapshot.state);
        next
    }

    /// Ensure the four Nitro plans exist (recreate deleted ones).
    pub fn ensure_nitro_plans(&mut self) {
        if let Err(err) = PowerApi::ensure_nitro_plans() {
            log::error(format!("power: failed to ensure Nitro plans: {err:?}"));
        }
    }

    /// Full enforcement for the current power state, silently: ensure plans,
    /// then apply the intended state (profile, HID, fan auto, smart charge,
    /// plan). Used at startup, on power transitions, and on resume.
    pub fn enforce_now(&mut self) {
        self.ensure_nitro_plans();
        let snapshot = self.refresh_power();
        self.apply_full(snapshot.state);
    }

    /// Firmware-only re-assertion (reapply loop): WMI profile, HID mode, fan
    /// auto, smart-charge state — never the active plan.
    pub fn reapply_firmware(&mut self) {
        let snapshot = self.refresh_power();
        let intent = self.engine.reapply_intended(snapshot.state, self.eco_ok());
        self.apply_intended(&intent);
    }

    /// Re-run eco acceptance detection (called on power transitions, on
    /// reapply ticks, and on each eco selection attempt): re-tests when eco
    /// is currently rejected, or when acceptance is still unknown and the
    /// current pick wants eco.
    pub fn re_evaluate_eco(&mut self) {
        let state = self.refresh_power().state;
        let wants_eco = self.engine.profile_for(state) == Profile::Eco;
        if self.eco_accepted == Some(true) || (self.eco_accepted.is_none() && !wants_eco) {
            return;
        }
        self.detect_eco();
    }

    /// Apply only the Windows plan for a profile (degraded mode: still
    /// offered when the Acer WMI interface is unavailable).
    pub fn apply_plan(&self, profile: Profile) {
        let plan = profile.plan_name();
        match PowerApi::set_active_plan(plan) {
            Ok(()) => log::info(format!("power: active plan set to {plan}")),
            Err(err) => log::warn(format!("power: failed to set active plan {plan}: {err:?}")),
        }
    }

    /// Acer WMI interface reachable?
    pub fn wmi_available(&self) -> bool {
        self.wmi.as_ref().is_some_and(|wmi| wmi.is_available())
    }

    /// Eco entry disabled (firmware rejected profile 6)?
    pub fn eco_disabled(&self) -> bool {
        self.eco_accepted == Some(false)
    }

    /// The current pick for a power state (for menu check marks).
    pub fn profile_for(&self, state: PowerState) -> Profile {
        self.engine.profile_for(state)
    }

    /// Current power state (last known snapshot).
    pub fn current_power(&self) -> PowerState {
        self.last_power.state
    }

    /// Auto-switch on AC <-> battery transitions (config, immutable at runtime).
    pub fn auto_switch(&self) -> bool {
        self.auto_switch
    }

    /// Read power from the OS and cache the snapshot for `current_power()`.
    fn refresh_power(&mut self) -> PowerStateSnapshot {
        let snapshot = power_state::read();
        self.last_power = snapshot;
        snapshot
    }

    /// Eco acceptance in `bool` form for the policy engine.
    fn eco_ok(&self) -> bool {
        self.eco_accepted == Some(true)
    }

    /// Eco runtime detection: write firmware profile 6, then read back; the
    /// machine accepts eco only when the readback equals 6. A set error or a
    /// failed readback while WMI is available means rejected. On rejection the
    /// previously active firmware profile is restored (best effort), so the
    /// machine is never left in an unspecified firmware state. Caches the
    /// result in `eco_accepted` (design decision 2).
    fn detect_eco(&mut self) {
        let (before, accepted) = {
            let Some(wmi) = self.wmi.as_ref() else {
                return;
            };
            let before = wmi.get_platform_profile().ok();
            let accepted = match wmi.set_platform_profile(wmi::PROFILE_ECO) {
                Err(err) => {
                    log::warn(format!("wmi: eco detection failed to set profile 6: {err:?}"));
                    None
                }
                Ok(()) => match wmi.get_platform_profile() {
                    Ok(value) => {
                        let accepted = value == wmi::PROFILE_ECO;
                        log::info(format!(
                            "wmi: eco detection readback {value} (eco accepted: {accepted})"
                        ));
                        Some(accepted)
                    }
                    Err(err) => {
                        log::warn(format!("wmi: eco detection readback failed: {err:?}"));
                        None
                    }
                },
            };
            (before, accepted)
        };
        match accepted {
            Some(true) => self.eco_accepted = Some(true),
            Some(false) | None => {
                self.eco_accepted = Some(false);
                if let (Some(wmi), Some(before)) = (self.wmi.as_ref(), before) {
                    if let Err(err) = wmi.set_platform_profile(before) {
                        log::warn(format!("wmi: failed to restore profile {before} after eco rejection: {err:?}"));
                    }
                }
            }
        }
    }

    /// Full intended apply for a power state. Runs first-time eco detection
    /// when the current pick is eco and acceptance is still unknown
    /// (automatic enforcement paths).
    fn apply_full(&mut self, state: PowerState) {
        if self.engine.profile_for(state) == Profile::Eco && self.eco_accepted.is_none() {
            self.detect_eco();
        }
        let intent = self.engine.intended(state, self.eco_ok());
        self.apply_intended(&intent);
    }

    /// Apply one intended state to the hardware/OS: WMI profile, HID usage
    /// mode (log-only on failure, never fatal), fan auto, smart charge, and
    /// the active plan. Every item is applied independently; failures are
    /// logged and never abort the rest. Smart charge is always targeted on —
    /// it cannot be disabled in the app — so every apply is a best-effort
    /// enable attempt (writes are retried by the reapply loop when enabled).
    fn apply_intended(&self, intent: &IntendedState) {
        if let (Some(wmi), Some(value)) = (self.wmi.as_ref(), intent.firmware_profile) {
            match wmi.set_platform_profile(value) {
                Ok(()) => log::info(format!("wmi: platform profile set to {value}")),
                Err(err) => log::warn(format!("wmi: failed to set platform profile {value}: {err:?}")),
            }
        }
        if let Some(hid) = self.hid.as_ref() {
            match hid.set_usage_mode(intent.hid_mode) {
                Ok(()) => log::info(format!("hid: usage mode set to {:?}", intent.hid_mode)),
                Err(err) => {
                    log::warn(format!("hid: failed to set usage mode {:?}: {err:?}", intent.hid_mode))
                }
            }
        }
        if let Some(wmi) = self.wmi.as_ref() {
            match wmi.set_fan_auto() {
                Ok(()) => log::info("wmi: fan set to auto"),
                Err(err) => log::warn(format!("wmi: failed to set fan auto: {err:?}")),
            }
        }
        if let Some(charge) = self.charge.as_ref() {
            match charge.set_enabled(true) {
                Ok(()) => log::info("charge: smart charge enabled"),
                Err(err) => log::warn(format!("charge: failed to enable smart charge: {err:?}")),
            }
        }
        if let Some(plan) = intent.plan {
            match PowerApi::set_active_plan(plan) {
                Ok(()) => log::info(format!("power: active plan set to {plan}")),
                Err(err) => log::warn(format!("power: failed to set active plan {plan}: {err:?}")),
            }
        }
    }

    /// Persist the per-power-state picks to the state file. Smart charge is
    /// intentionally absent: it is always on and cannot be configured.
    fn write_state_file(&self) {
        let text = serialize_state(self.engine.picks());
        if let Err(err) = std::fs::write(&self.state_path, text) {
            log::warn(format!("state: cannot write {}: {err}", self.state_path.display()));
        }
    }
}

/// Parse state-file TOML text into `(picks)` overrides. Missing or malformed
/// content yields `None`; partial entries are ignored as a whole. A legacy
/// `smart_charge` key (the app no longer lets users disable it) is ignored.
fn load_state(toml_text: &str) -> Option<(String, String)> {
    let value: toml::Value = toml::from_str(toml_text).ok()?;
    let table = value.as_table()?;
    table.get("picks").and_then(|picks| {
        let table = picks.as_table()?;
        Some((
            table.get("ac")?.as_str()?.to_string(),
            table.get("battery")?.as_str()?.to_string(),
        ))
    })
}

/// Serialize picks as TOML text, parseable by `load_state` (round-trip
/// tested).
fn serialize_state(picks: (Profile, Profile)) -> String {
    let mut root = toml::map::Map::new();
    let mut picks_table = toml::map::Map::new();
    picks_table.insert("ac".to_string(), toml::Value::String(picks.0.as_str().to_string()));
    picks_table.insert(
        "battery".to_string(),
        toml::Value::String(picks.1.as_str().to_string()),
    );
    root.insert("picks".to_string(), toml::Value::Table(picks_table));
    toml::to_string(&toml::Value::Table(root)).unwrap_or_default()
}

/// Map a firmware platform-profile readback to a `Profile` (0 quiet, 1
/// balanced, 4 performance, 6 eco); `None` for unknown values (e.g. turbo 5).
fn profile_from_firmware(value: u32) -> Option<Profile> {
    match value {
        wmi::PROFILE_QUIET => Some(Profile::Quiet),
        wmi::PROFILE_BALANCED => Some(Profile::Balanced),
        wmi::PROFILE_PERFORMANCE => Some(Profile::Performance),
        wmi::PROFILE_ECO => Some(Profile::Eco),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file_round_trip_picks() {
        let text = serialize_state((Profile::Quiet, Profile::Eco));
        let picks = load_state(&text);
        assert_eq!(picks, Some(("quiet".to_string(), "eco".to_string())));
    }

    #[test]
    fn state_file_round_trip_all_profiles() {
        for (ac, battery) in [
            ("quiet", "balanced"),
            ("balanced", "eco"),
            ("performance", "eco"),
        ] {
            let text = serialize_state((
                Profile::from_config_str(ac).unwrap(),
                Profile::from_config_str(battery).unwrap(),
            ));
            let picks = load_state(&text);
            assert_eq!(picks, Some((ac.to_string(), battery.to_string())));
        }
    }

    #[test]
    fn state_file_serialized_text_is_toml_parseable() {
        let text = serialize_state((Profile::Performance, Profile::Balanced));
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let table = parsed.as_table().unwrap();
        assert!(table.get("smart_charge").is_none());
        let picks = table.get("picks").unwrap().as_table().unwrap();
        assert_eq!(picks.get("ac").unwrap().as_str(), Some("performance"));
        assert_eq!(picks.get("battery").unwrap().as_str(), Some("balanced"));
    }

    #[test]
    fn load_state_empty_and_malformed_give_none() {
        assert_eq!(load_state(""), None);
        assert_eq!(load_state("smart_charge = = true"), None);
        assert_eq!(load_state("[picks"), None);
    }

    #[test]
    fn load_state_partial_picks_are_ignored() {
        assert_eq!(load_state("[picks]\nac = \"quiet\""), None);
        assert_eq!(load_state("[picks]\nbattery = \"eco\""), None);
        assert_eq!(load_state("smart_charge = true"), None);
    }

    #[test]
    fn load_state_wrong_types_are_ignored() {
        assert_eq!(load_state("[picks]\nac = 5\nbattery = true"), None);
        assert_eq!(load_state("smart_charge = \"yes\""), None);
    }

    #[test]
    fn load_state_ignores_unknown_keys_and_legacy_smart_charge() {
        let text = "reapply = true\nsmart_charge = false\n[other]\nkey = 1\n[picks]\nac = \"balanced\"\nbattery = \"eco\"";
        let picks = load_state(text);
        assert_eq!(picks, Some(("balanced".to_string(), "eco".to_string())));
    }

    #[test]
    fn firmware_readback_maps_spec_values() {
        assert_eq!(profile_from_firmware(0), Some(Profile::Quiet));
        assert_eq!(profile_from_firmware(1), Some(Profile::Balanced));
        assert_eq!(profile_from_firmware(4), Some(Profile::Performance));
        assert_eq!(profile_from_firmware(6), Some(Profile::Eco));
    }

    #[test]
    fn firmware_readback_unknown_values_give_none() {
        for value in [2, 3, 5, 7, 0xFF] {
            assert_eq!(profile_from_firmware(value), None, "value {value}");
        }
    }
}
