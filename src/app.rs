//! Application core: wires config + policy engine + adapters (accepted
//! across the transport/plan seams — the binary entry point builds the
//! production instances, tests build fakes, ticket 05). Owns the apply
//! path (firmware profile, HID usage mode, fan auto, keyboard backlight off,
//! plan), the startup + minute-tick smart-charge occasions, runtime eco
//! acceptance detection, per-power-state pick persistence
//! (`nitro-tray.state.toml` beside the exe), and the read-back effective
//! state. The per-occasion entry points (`on_startup`, `on_power_changed`,
//! `on_resume`, `on_recovery_tick`, `on_reapply_tick`) fold the former
//! trigger modules and dispatch the tray/power events; each occasion's
//! trigger knowledge — which occasions re-evaluate eco, which run quiet —
//! lives here, in one place. Degrades gracefully when the Acer WMI interface
//! is unreachable; unavailable adapters are reconnected by the recovery
//! loop, never terminal.

use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::adapter::map_mi;
use crate::charge::SmartChargeAdapter;
use crate::config::Config;
use crate::hid::{HidAdapter, HidTransport, RealHidTransport};
use crate::log;
use crate::mi::MiConnection;
use crate::policy::{IntendedState, PolicyEngine, PowerState, Profile};
use crate::power::{PlanApi, PowerApi, PowerError};
use crate::power_state::{self, PowerStateSnapshot};
use crate::transport::MiTransport;
use crate::wmi::WmiAdapter;

/// Filename of the persisted user state, beside the exe.
pub const STATE_FILE_NAME: &str = "nitro-tray.state.toml";

/// Read-back effective state for the tray (hardware/OS truth, not intent).
#[derive(Clone, Debug)]
pub struct EffectiveState {
    pub power: PowerState,
    pub percent: u8,
    /// Active firmware profile; `None` when unreadable.
    pub profile: Option<Profile>,
    /// Active Nitro plan (canonical name); `None` when no Nitro plan is
    /// active or the readback failed.
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

/// Application core over the transport seams: `M` is the MI transport
/// (production `MiConnection`, tests `FakeTransport`), `T` the HID transport
/// (production `RealHidTransport`, tests `FakeHidTransport`), `P` the
/// power-plan API (production `PowerApi`, tests `FakePlanApi`). All three
/// default to production, so the binary entry point stays
/// `AppCore::new(config, exe_dir, wmi, charge, hid, PowerApi)`.
pub struct AppCore<
    M: MiTransport = MiConnection,
    T: HidTransport = RealHidTransport,
    P: PlanApi = PowerApi,
> {
    engine: PolicyEngine,
    auto_switch: bool,
    wmi: Option<WmiAdapter<M>>,
    charge: Option<SmartChargeAdapter<M>>,
    hid: Option<HidAdapter<T>>,
    power: P,
    eco_accepted: Option<bool>,
    last_power: PowerStateSnapshot,
    state_path: PathBuf,
    /// Logged a failed wmi reconnect at least once in the current degradation
    /// episode (reset on recovery), so the 30 s retry loop never log-spams.
    wmi_retry_logged: bool,
    /// Same as `wmi_retry_logged`, for the smart-charge adapter.
    charge_retry_logged: bool,
    /// Turn the keyboard backlight off on every apply (config, default true).
    keyboard_led_off: bool,
    /// "Start at logon" checkbox: installs/removes the logon scheduled task
    /// (persisted in the state file; default false — new installs have no
    /// task).
    start_at_logon: bool,
}

impl<M: MiTransport, T: HidTransport, P: PlanApi> AppCore<M, T, P> {
    /// Construct the core: parse config-backed picks, load the state file.
    /// The adapters and the plan API arrive ACROSS the seam — the binary
    /// entry point connects them (degradation warnings live there), tests
    /// hand over fakes. Connecting nothing here means the core's behavior
    /// runs under `cargo test` without hardware or Win32.
    pub fn new(
        config: Config,
        exe_dir: &Path,
        wmi: Option<WmiAdapter<M>>,
        charge: Option<SmartChargeAdapter<M>>,
        hid: Option<HidAdapter<T>>,
        power: P,
    ) -> Self {
        let mut engine = PolicyEngine::new(&config);

        let state_path = exe_dir.join(STATE_FILE_NAME);
        let start_at_logon = match std::fs::read_to_string(&state_path) {
            Ok(contents) => {
                let state = load_state(&contents);
                if let Some((ac, battery)) = state.picks {
                    match Profile::from_config_str(&ac) {
                        Some(profile) => engine.set_profile(PowerState::Ac, profile),
                        None => log::warn(format!("state: ignoring invalid ac pick {ac:?}")),
                    }
                    match Profile::from_config_str(&battery) {
                        Some(profile) => engine.set_profile(PowerState::Battery, profile),
                        None => log::warn(format!("state: ignoring invalid battery pick {battery:?}")),
                    }
                }
                state.start_at_logon
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => {
                log::warn(format!("state: cannot read {}: {err}", state_path.display()));
                false
            }
        };

        AppCore {
            engine,
            auto_switch: config.auto_switch,
            keyboard_led_off: config.keyboard_led_off,
            wmi,
            charge,
            hid,
            power,
            eco_accepted: None,
            last_power: power_state::read(),
            state_path,
            wmi_retry_logged: false,
            charge_retry_logged: false,
            start_at_logon,
        }
    }

    /// Read the current effective state back from hardware/OS.
    pub fn effective(&self) -> EffectiveState {
        let snapshot = power_state::read();
        let profile = match self.wmi.as_ref() {
            Some(wmi) if wmi.is_available() => match wmi.get_platform_profile() {
                Ok(value) => Profile::from_firmware_value(value),
                Err(err) => {
                    LOGGED_WMI_PROFILE_READ.call_once(|| {
                        log::warn(format!("wmi: platform profile readback failed: {err:?}"));
                    });
                    None
                }
            },
            Some(_) | None => None,
        };
        let plan = match self.power.active_profile() {
            Ok(Some(profile)) => Some(profile.plan_name().to_string()),
            Ok(None) => None,
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
    /// pick, run eco runtime detection when the pick is eco. Returns the
    /// apply outcome (failed + skipped items), for the tray's status line.
    pub fn apply_profile(&mut self, profile: Profile) -> ApplyReport {
        let snapshot = self.refresh_power();
        self.engine.set_profile(snapshot.state, profile);
        self.write_state_file();
        if profile == Profile::Eco {
            self.detect_eco(false);
        }
        self.apply_full(snapshot.state)
    }

    /// Cycle forward through the current power state's list, apply, persist;
    /// returns the new profile. A disabled eco entry is skipped so the hotkey
    /// can never select a profile the firmware rejects. The apply result is
    /// reported like `apply_profile` (for the tray's status line).
    pub fn cycle_profile(&mut self) -> (Profile, ApplyReport) {
        let snapshot = self.refresh_power();
        let mut next = self.engine.cycle(snapshot.state);
        if next == Profile::Eco && self.eco_disabled() {
            next = self.engine.cycle(snapshot.state);
        }
        self.write_state_file();
        if next == Profile::Eco {
            self.detect_eco(false);
        }
        let report = self.apply_full(snapshot.state);
        (next, report)
    }

    /// Ensure plan support: Windows recreates deleted Nitro plans, Linux
    /// runs quiet (no-op).
    pub fn ensure_support(&mut self) {
        if let Err(err) = self.power.ensure_support() {
            log::error(format!("power: failed to ensure plan support: {err:?}"));
        }
    }

    /// Full enforcement for the current power state, silently: ensure plan
    /// support, then apply the intended state (profile, HID, fan auto,
    /// keyboard backlight, plan — never smart charge). Used at startup, on
    /// power transitions, and on resume.
    pub fn enforce_now(&mut self) {
        self.ensure_support();
        let snapshot = self.refresh_power();
        self.apply_full(snapshot.state);
    }

    /// Enforcement against the cached snapshot (no read — the caller owns
    /// the single power-state read of the occasion).
    fn enforce_now_for(&mut self) {
        self.ensure_support();
        self.apply_full(self.last_power.state);
    }

    /// Firmware-only re-assertion (reapply loop): WMI profile, HID mode, fan
    /// auto, keyboard backlight (config-gated) — never smart charge (that is
    /// the startup apply + minute tick's job) and never the active plan.
    /// Runs quietly: the loop ticks every `reapply_interval_secs`, so its
    /// success lines are DEBUG, never INFO (only failures surface as WARN).
    pub fn reapply_firmware(&mut self) {
        let snapshot = self.refresh_power();
        let intent = self.engine.reapply_intended(snapshot.state, self.eco_ok());
        self.apply_intended(&intent, true);
    }

    /// Firmware-only re-assertion against the cached snapshot (no read).
    fn reapply_firmware_for(&mut self) {
        let intent = self.engine.reapply_intended(self.last_power.state, self.eco_ok());
        self.apply_intended(&intent, true);
    }

    /// Once-a-minute smart-charge re-assertion (readback tick): when the cap
    /// reads back off, re-enable it. The readback loop is always armed, so a
    /// silent external disable is corrected within a minute even when the
    /// reapply loop is off. Read errors are ignored (the recovery loop owns
    /// reconnects).
    pub fn reassert_smart_charge(&mut self) {
        let Some(charge) = self.charge.as_ref() else { return };
        if !charge.is_available() {
            return;
        }
        match charge.is_enabled() {
            Ok(true) => {}
            Ok(false) => match charge.set_enabled(true) {
                Ok(()) => log::info("charge: smart charge read back off; re-enabled"),
                Err(err) => log::warn(format!("charge: failed to re-enable after readback off: {err:?}")),
            },
            Err(err) => log::warn(format!("charge: readback failed on re-assert tick: {err:?}")),
        }
    }

    /// Smart charge at application start: one write enabling the 80% cap
    /// (the other smart-charge occasion is the once-a-minute
    /// `reassert_smart_charge`; everything else leaves it untouched).
    pub fn apply_smart_charge(&mut self) {
        match self.charge.as_ref() {
            Some(charge) => match charge.set_enabled(true) {
                Ok(()) => log::info("charge: smart charge enabled"),
                Err(err) => log::warn(format!("charge: failed to enable smart charge: {err:?}")),
            },
            None => log::warn("charge: adapter unavailable at startup; smart charge not applied"),
        }
    }

    /// Re-run eco acceptance detection (called on power transitions, on
    /// reapply ticks, and on each eco selection attempt): re-tests when eco
    /// is currently rejected, or when acceptance is still unknown and the
    /// current pick wants eco. Loop-driven runs (reapply ticks) are quiet.
    pub fn re_evaluate_eco(&mut self) {
        let state = self.refresh_power().state;
        self.re_evaluate_eco_for(state);
    }

    /// Eco re-check against a KNOWN power state (no read — the caller owns
    /// the snapshot). Loop-driven runs are quiet.
    fn re_evaluate_eco_for(&mut self, state: PowerState) {
        let wants_eco = self.engine.profile_for(state) == Profile::Eco;
        if self.eco_accepted == Some(true) || (self.eco_accepted.is_none() && !wants_eco) {
            return;
        }
        self.detect_eco(true);
    }

    /// Apply only the profile's plan (degraded mode: still offered when the
    /// Acer WMI interface is unavailable). Returns the apply outcome, for
    /// the tray's status line. A partial backend failure surfaces its
    /// per-item failures verbatim (granular tray status); any other error
    /// is reported as the single "plan" item.
    pub fn apply_plan(&self, profile: Profile) -> ApplyReport {
        let plan = profile.plan_name();
        match self.power.set_profile(profile) {
            Ok(()) => {
                log::info(format!("power: active plan set to {plan}"));
                ApplyReport::default()
            }
            Err(err) => {
                log::warn(format!("power: failed to set active plan {plan}: {err:?}"));
                let failed = match err {
                    PowerError::Partial { failed } => failed,
                    _ => vec!["plan"],
                };
                ApplyReport {
                    failed,
                    skipped: Vec::new(),
                }
            }
        }
    }

    /// Acer WMI interface reachable?
    pub fn wmi_available(&self) -> bool {
        self.wmi.as_ref().is_some_and(|wmi| wmi.is_available())
    }

    /// Smart-charge adapter usable (connected and not breaker-tripped)?
    pub fn charge_available(&self) -> bool {
        self.charge.as_ref().is_some_and(|charge| charge.is_available())
    }

    /// Try to reconnect adapters that are missing or tripped their circuit
    /// breaker (driven by the always-armed 30 s recovery timer). A fresh
    /// `M::connect()` replaces the old adapter only on full success — the
    /// breaker is never reset on a single successful call. On any reconnect,
    /// eco acceptance is re-evaluated and enforcement re-runs. Returns true
    /// when an adapter reconnected, so the caller refreshes the tray view
    /// ("Hardware unavailable" clears by itself). Failed attempts log at most
    /// once per degradation episode, never once per tick.
    pub fn reconnect_unavailable(&mut self) -> bool {
        let mut reconnected = false;
        if !self.wmi_available() {
            match M::connect().map_err(map_mi) {
                Ok(transport) => {
                    log::info("wmi: reconnected; leaving degraded mode");
                    self.wmi = Some(WmiAdapter::with_transport(transport));
                    self.wmi_retry_logged = false;
                    reconnected = true;
                }
                Err(err) => {
                    if !self.wmi_retry_logged {
                        log::warn(format!("wmi: reconnect failed, still degraded: {err:?}"));
                        self.wmi_retry_logged = true;
                    }
                }
            }
        }
        if !self.charge_available() {
            match M::connect().map_err(map_mi) {
                Ok(transport) => {
                    log::info("charge: smart-charge adapter reconnected");
                    self.charge = Some(SmartChargeAdapter::with_transport(transport));
                    self.charge_retry_logged = false;
                    reconnected = true;
                }
                Err(err) => {
                    if !self.charge_retry_logged {
                        log::warn(format!("charge: reconnect failed: {err:?}"));
                        self.charge_retry_logged = true;
                    }
                }
            }
        }
        if reconnected {
            self.re_evaluate_eco();
            self.enforce_now();
        }
        reconnected
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

    /// "Start at logon" flag (state file, default false).
    pub fn start_at_logon(&self) -> bool {
        self.start_at_logon
    }

    /// Enable/disable "Start at logon" and persist the flag. The caller
    /// installs or removes the scheduled task (main.rs) — the flag and the
    /// task stay in sync from the caller's side.
    pub fn set_start_at_logon(&mut self, enabled: bool) {
        self.start_at_logon = enabled;
        self.write_state_file();
    }

    /// Startup occasion (once, from the binary entry point after the tray
    /// is up): ensure plan support, enforce the intended state for the
    /// current power state, and apply smart charge (one of its two occasions
    /// — the other is the once-a-minute readback tick).
    pub fn on_startup(&mut self) {
        log::info("enforcement: startup");
        self.ensure_support();
        self.enforce_now();
        self.apply_smart_charge();
    }

    /// AC <-> battery transition (only when `auto_switch` is enabled in
    /// config): silently enforce the intended state for the new power
    /// state. Reads the power snapshot ONCE and drives the eco re-check and
    /// the enforcement from the cached snapshot, so the occasion is a single
    /// `GetSystemPowerStatus` read.
    pub fn on_power_changed(&mut self) {
        if !self.auto_switch {
            log::info("enforcement: power change ignored (auto_switch disabled)");
            return;
        }
        log::info("enforcement: power change; enforcing intended state");
        self.refresh_power();
        self.re_evaluate_eco_for(self.last_power.state);
        self.enforce_now_for();
    }

    /// Resume/wake occasion: re-enforce after firmware or OS resets. Reads
    /// the power snapshot once, like `on_power_changed`.
    pub fn on_resume(&mut self) {
        log::info("enforcement: resume; re-enforcing intended state");
        self.refresh_power();
        self.re_evaluate_eco_for(self.last_power.state);
        self.enforce_now_for();
    }

    /// Recovery tick (always-armed 30 s timer): reconnect adapters that are
    /// missing or tripped their circuit breaker; see `reconnect_unavailable`
    /// for the return semantics (true → the caller refreshes the tray view).
    pub fn on_recovery_tick(&mut self) -> bool {
        self.reconnect_unavailable()
    }

    /// Reapply tick (config-gated loop, off by default): eco re-evaluation
    /// plus firmware-only re-assertion, quietly — never the active plan,
    /// never smart charge. Reads the power snapshot once for both steps.
    pub fn on_reapply_tick(&mut self) {
        self.refresh_power();
        self.re_evaluate_eco_for(self.last_power.state);
        self.reapply_firmware_for();
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
    /// result in `eco_accepted` (design decision 2). `quiet` suppresses the
    /// outcome INFO for loop-driven runs.
    fn detect_eco(&mut self, quiet: bool) {
        let (before, accepted) = {
            let Some(wmi) = self.wmi.as_ref() else {
                return;
            };
            let before = wmi.get_platform_profile().ok();
            let accepted = match wmi.set_platform_profile(Profile::Eco.firmware_value()) {
                Err(err) => {
                    log::warn(format!("wmi: eco detection failed to set profile 6: {err:?}"));
                    None
                }
                Ok(()) => match wmi.get_platform_profile() {
                    Ok(value) => {
                        let accepted = value == Profile::Eco.firmware_value();
                        let message = format!(
                            "wmi: eco detection readback {value} (eco accepted: {accepted})"
                        );
                        if quiet {
                            log::debug(message);
                        } else {
                            log::info(message);
                        }
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
    /// (automatic enforcement paths). Returns the apply outcome.
    fn apply_full(&mut self, state: PowerState) -> ApplyReport {
        if self.engine.profile_for(state) == Profile::Eco && self.eco_accepted.is_none() {
            self.detect_eco(false);
        }
        let intent = self.engine.intended(state, self.eco_ok());
        self.apply_intended(&intent, false)
    }

    /// Apply one intended state to the hardware/OS: WMI profile, HID usage
    /// mode (log-only on failure, never fatal), fan auto, keyboard backlight
    /// off (config-gated), and the active plan. Smart charge is deliberately
    /// absent: it is written only at application start
    /// (`apply_smart_charge`) and re-enabled by the once-a-minute readback
    /// tick when it reads off (`reassert_smart_charge`) — profile changes,
    /// power transitions, resume, and reapply ticks never touch it. Every
    /// item is applied independently; failures are logged, never abort the
    /// rest, and reported so the tray can show the outcome. An item whose
    /// adapter is unavailable is reported as *not applied* (never "Applied"
    /// and never a failure — the tray already shows the degraded state). A
    /// `None` firmware target (eco rejected) is not an item at all. `quiet`
    /// demotes success lines to DEBUG (the reapply loop ticks repeatedly;
    /// failures still surface as WARN either way).
    fn apply_intended(&self, intent: &IntendedState, quiet: bool) -> ApplyReport {
        let mut report = ApplyReport::default();
        let ok = |message: String| {
            if quiet {
                log::debug(message);
            } else {
                log::info(message);
            }
        };
        if let Some(value) = intent.firmware_profile {
            match self.wmi.as_ref() {
                Some(wmi) => match wmi.set_platform_profile(value) {
                    Ok(()) => ok(format!("wmi: platform profile set to {value}")),
                    Err(err) => {
                        log::warn(format!("wmi: failed to set platform profile {value}: {err:?}"));
                        report.failed.push("platform profile");
                    }
                },
                None => report.skipped.push("platform profile"),
            }
        }
        if let Some(hid) = self.hid.as_ref() {
            match hid.set_usage_mode(intent.hid_mode) {
                Ok(()) => ok(format!("hid: usage mode set to {:?}", intent.hid_mode)),
                Err(err) => {
                    log::warn(format!("hid: failed to set usage mode {:?}: {err:?}", intent.hid_mode));
                    report.failed.push("HID mode");
                }
            }
        } else {
            report.skipped.push("HID mode");
        }
        if let Some(wmi) = self.wmi.as_ref() {
            match wmi.set_fan_auto() {
                Ok(()) => ok("wmi: fan set to auto".to_string()),
                Err(err) => {
                    log::warn(format!("wmi: failed to set fan auto: {err:?}"));
                    report.failed.push("fan");
                }
            }
        } else {
            report.skipped.push("fan");
        }
        if self.keyboard_led_off {
            match self.wmi.as_ref() {
                Some(wmi) => match wmi.set_keyboard_backlight_off() {
                    Ok(()) => ok("wmi: keyboard backlight set off".to_string()),
                    Err(err) => {
                        log::warn(format!("wmi: failed to set keyboard backlight off: {err:?}"));
                        report.failed.push("keyboard leds");
                    }
                },
                None => report.skipped.push("keyboard leds"),
            }
        }
        if let Some(profile) = intent.plan.and_then(Profile::from_plan_name) {
            let plan = profile.plan_name();
            match self.power.set_profile(profile) {
                Ok(()) => ok(format!("power: active plan set to {plan}")),
                Err(err) => {
                    log::warn(format!("power: failed to set active plan {plan}: {err:?}"));
                    // A partial backend failure (ticket 05's sysfs EACCES
                    // path) surfaces its per-item failures verbatim; any
                    // other error is the single "plan" item.
                    match err {
                        PowerError::Partial { failed } => report.failed.extend(failed),
                        _ => report.failed.push("plan"),
                    }
                }
            }
        }
        report
    }

    /// Persist the per-power-state picks and the start-at-logon flag. Smart
    /// charge is intentionally absent: it is always on and cannot be
    /// configured.
    fn write_state_file(&self) {
        let state = UserState {
            picks: Some((
                self.engine.profile_for(PowerState::Ac).as_str().to_string(),
                self.engine.profile_for(PowerState::Battery).as_str().to_string(),
            )),
            start_at_logon: self.start_at_logon,
        };
        let text = serialize_state(&state);
        if let Err(err) = std::fs::write(&self.state_path, text) {
            log::warn(format!("state: cannot write {}: {err}", self.state_path.display()));
        }
    }
}

/// Parse state-file TOML text into `UserState`. Missing or malformed content
/// yields defaults; partial entries are ignored as a whole. A legacy
/// `smart_charge` key (the app no longer lets users disable it) is ignored.
pub fn load_state(toml_text: &str) -> UserState {
    let table = toml::from_str::<toml::Value>(toml_text)
        .ok()
        .and_then(|value| value.as_table().cloned());
    let picks = table.as_ref().and_then(|table| {
        let picks = table.get("picks")?.as_table()?;
        Some((
            picks.get("ac")?.as_str()?.to_string(),
            picks.get("battery")?.as_str()?.to_string(),
        ))
    });
    let start_at_logon = table
        .as_ref()
        .and_then(|table| table.get("logon_task").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    UserState { picks, start_at_logon }
}

/// Serialize user state as TOML text, parseable by `load_state` (round-trip
/// tested).
pub fn serialize_state(state: &UserState) -> String {
    let mut root = toml::map::Map::new();
    if let Some((ac, battery)) = &state.picks {
        let mut picks = toml::map::Map::new();
        picks.insert("ac".to_string(), toml::Value::String(ac.clone()));
        picks.insert("battery".to_string(), toml::Value::String(battery.clone()));
        root.insert("picks".to_string(), toml::Value::Table(picks));
    }
    root.insert(
        "logon_task".to_string(),
        toml::Value::Boolean(state.start_at_logon),
    );
    toml::to_string(&toml::Value::Table(root)).unwrap_or_default()
}

/// Persisted user state (`nitro-tray.state.toml`), defaults for new installs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserState {
    /// Per-power-state profile picks (`ac`, `battery`), when both present.
    pub picks: Option<(String, String)>,
    /// Start-at-logon scheduled task enabled; default `false` — new installs
    /// have no scheduled task until the user checks "Start at logon".
    pub start_at_logon: bool,
}

/// Outcome of one apply, for the tray's ephemeral status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    /// Labels of apply items that errored.
    pub failed: Vec<&'static str>,
    /// Labels of apply items that were intended but could not run because
    /// their adapter is unavailable (never counted as failures — the tray
    /// already shows the degraded state).
    pub skipped: Vec<&'static str>,
}

/// Status-line text for an apply outcome: "Applied" when nothing failed or
/// was skipped, else "Failed: <labels>" / "Not applied: <labels>" (or both,
/// joined). Ephemeral feedback — no history is kept.
pub fn apply_report_text(report: &ApplyReport) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !report.failed.is_empty() {
        parts.push(format!("Failed: {}", report.failed.join(", ")));
    }
    if !report.skipped.is_empty() {
        parts.push(format!("Not applied: {}", report.skipped.join(", ")));
    }
    if parts.is_empty() {
        "Applied".to_string()
    } else {
        parts.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{self, WMI_NAMESPACE};
    use crate::charge::{CLASS_NAME as CHARGE_CLASS, METHOD_GET as CHARGE_GET, METHOD_SET as CHARGE_SET};
    use crate::testing::{FakeHidTransport, FakePlanApi, FakeTransport, no_output, some_output, transport_error};
    use crate::transport::{MiElement, MiOutput, MiValue};
    use crate::wmi::CLASS_NAME as WMI_CLASS;

    type TestCore = AppCore<FakeTransport, FakeHidTransport, FakePlanApi>;

    /// A unique scratch dir for one test's state file, removed on drop of
    /// the returned guard (`temp_dir` keeps config.rs's pattern: unique
    /// name + pid, so parallel tests never collide).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nitro-tray-app-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn remove_dir(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Script the four WMI firmware apply keys on a fake transport.
    fn script_wmi_apply(fake: &FakeTransport) {
        fake.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingMiscSetting", [no_output()]);
        fake.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingFanBehavior", [no_output()]);
        fake.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingKBBacklight", [no_output()]);
    }

    /// Script the smart-charge write (ReturnValue 1) + readback row (cap on).
    fn script_charge_ok(fake: &FakeTransport) {
        fake.script(WMI_NAMESPACE, CHARGE_CLASS, CHARGE_SET, [
            some_output(MiOutput::new().with_u32("ReturnValue", 1)),
        ]);
        fake.script(WMI_NAMESPACE, CHARGE_CLASS, CHARGE_GET, [
            some_output(MiOutput::new().with_u32("uFunctionList", 3).with_u8_array("uFunctionStatus", vec![1, 0, 0, 0, 0])),
        ]);
    }

    /// A fully scripted app over fakes: every adapter healthy, default
    /// config. Returns (core, wmi fake, hid fake, plan fake).
    fn scripted_app(
        dir: &Path,
        config: Config,
    ) -> (TestCore, FakeTransport, FakeHidTransport, FakePlanApi) {
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)),
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)),
        ]);
        script_wmi_apply(&wmi);
        let charge = FakeTransport::new();
        script_charge_ok(&charge);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let power = FakePlanApi::new();
        let app = AppCore::new(
            config,
            dir,
            Some(WmiAdapter::with_transport(wmi.clone())),
            Some(SmartChargeAdapter::with_transport(charge)),
            Some(HidAdapter::with_transport(hid.clone())),
            power.clone(),
        );
        (app, wmi, hid, power)
    }

    fn eco_picks() -> Config {
        Config {
            ac_profile: "eco".to_string(),
            battery_profile: "eco".to_string(),
            ..Config::default()
        }
    }

    #[test]
    fn eco_detection_accepts_profile_six_when_the_readback_matches() {
        let dir = temp_dir("eco-accept");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingMiscSetting", [no_output()]);
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)), // before: balanced
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)), // readback: eco
        ]);
        script_wmi_apply(&wmi);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let mut app = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi.clone())),
            None,
            Some(HidAdapter::with_transport(hid.clone())),
            FakePlanApi::new(),
        );
        let report = app.apply_profile(Profile::Eco);
        assert_eq!(report, ApplyReport::default());
        assert!(!app.eco_disabled());
        // The detect write (6) and the apply write (6) both used the eco
        // firmware value; the readback ran twice (before + after).
        let calls = wmi.calls();
        let sets = calls.iter().filter(|c| c.method == "SetGamingMiscSetting").collect::<Vec<_>>();
        assert_eq!(sets.len(), 2);
        for call in sets {
            assert_eq!(call.input.elements[0], MiElement { name: "gmInput", value: MiValue::U64(0x60B) });
        }
        assert_eq!(wmi.count("GetGamingMiscSetting"), 2);
        remove_dir(&dir);
    }

    #[test]
    fn eco_detection_rejection_restores_the_previous_firmware_profile() {
        let dir = temp_dir("eco-reject");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingMiscSetting", [no_output()]);
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)), // before: balanced
            some_output(MiOutput::new().with_u64("gmOutput", 0x500)), // readback: rejected
        ]);
        script_wmi_apply(&wmi);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let mut app = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi.clone())),
            None,
            Some(HidAdapter::with_transport(hid.clone())),
            FakePlanApi::new(),
        );
        app.apply_profile(Profile::Eco);
        assert!(app.eco_disabled());
        // detect wrote 6, then the rejected restore wrote the original 1.
        let set_inputs: Vec<u64> = wmi
            .calls()
            .iter()
            .filter(|c| c.method == "SetGamingMiscSetting")
            .map(|c| match c.input.elements[0].value {
                MiValue::U64(value) => value,
                _ => panic!("expected u64 gmInput"),
            })
            .collect();
        assert_eq!(set_inputs, vec![0x60B, 0x10B]);
        remove_dir(&dir);
    }

    #[test]
    fn apply_profile_all_items_succeed_reports_applied() {
        let dir = temp_dir("apply-ok");
        let (mut app, wmi, hid, power) = scripted_app(&dir, Config::default());
        let report = app.apply_profile(Profile::Balanced);
        assert_eq!(report, ApplyReport::default());
        assert_eq!(apply_report_text(&report), "Applied");
        assert_eq!(wmi.count("SetGamingMiscSetting"), 1);
        assert_eq!(wmi.count("SetGamingFanBehavior"), 1);
        assert_eq!(wmi.count("SetGamingKBBacklight"), 1);
        assert_eq!(hid.sent_reports().len(), 1);
        assert_eq!(power.set_calls(), vec!["Nitro-Balanced".to_string()]);
        remove_dir(&dir);
    }

    #[test]
    fn apply_profile_wmi_error_reports_the_failed_item() {
        let dir = temp_dir("apply-wmi-error");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingMiscSetting", [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingFanBehavior", [no_output()]);
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "SetGamingKBBacklight", [no_output()]);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let mut app = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi)),
            None,
            Some(HidAdapter::with_transport(hid)),
            FakePlanApi::new(),
        );
        let report = app.apply_profile(Profile::Balanced);
        assert_eq!(report.failed, vec!["platform profile"]);
        assert!(report.skipped.is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn apply_profile_without_wmi_adapter_skips_firmware_items() {
        let dir = temp_dir("apply-no-wmi");
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let mut app: TestCore = AppCore::new(
            Config::default(),
            &dir,
            None,
            None,
            Some(HidAdapter::with_transport(hid)),
            FakePlanApi::new(),
        );
        let report = app.apply_profile(Profile::Balanced);
        assert!(report.failed.is_empty());
        assert_eq!(report.skipped, vec!["platform profile", "fan", "keyboard leds"]);
        remove_dir(&dir);
    }

    #[test]
    fn apply_plan_partial_failure_reports_the_items_verbatim() {
        let dir = temp_dir("apply-plan-partial");
        let power = FakePlanApi::new();
        power.script_set(vec![Err(PowerError::Partial {
            failed: vec!["governor", "energy_perf_policy"],
        })]);
        let app: TestCore = AppCore::new(
            Config::default(),
            &dir,
            None,
            None,
            None,
            power,
        );
        let report = app.apply_plan(Profile::Quiet);
        assert_eq!(report.failed, vec!["governor", "energy_perf_policy"]);
        assert_eq!(apply_report_text(&report), "Failed: governor, energy_perf_policy");
        remove_dir(&dir);
    }

    #[test]
    fn effective_reads_back_through_the_fake_adapters() {
        let dir = temp_dir("effective");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x400)),
        ]);
        let charge = FakeTransport::new();
        script_charge_ok(&charge);
        let power = FakePlanApi::new();
        power.script_active_profile(vec![Ok(Some(Profile::Performance))]);
        let app: TestCore = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi)),
            Some(SmartChargeAdapter::with_transport(charge)),
            None,
            power,
        );
        let effective = app.effective();
        assert_eq!(effective.profile, Some(Profile::Performance));
        assert_eq!(effective.plan.as_deref(), Some("Nitro-Performance"));
        assert_eq!(effective.smart_charge, Some(true));
        assert!(effective.wmi_available);
        assert!(!effective.eco_disabled);
        // The power state comes from the real machine (read-only); the
        // engine picks drive behavior, so the snapshot value is never
        // asserted.
        remove_dir(&dir);
    }

    #[test]
    fn reconnect_replaces_a_tripped_wmi_adapter_and_reenforces() {
        let dir = temp_dir("reconnect-wmi");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let tripped = WmiAdapter::with_transport(wmi.clone());
        for _ in 0..adapter::MAX_ADAPTER_FAILURES {
            assert!(tripped.get_platform_profile().is_err());
        }
        assert!(!tripped.is_available());

        // The next connect() produces a fresh transport seeded so the
        // reconnect's eco re-check REJECTS eco (readback 5) and the apply
        // path succeeds.
        wmi.script_next_connect(WMI_NAMESPACE, WMI_CLASS, "SetGamingMiscSetting", [no_output()]);
        wmi.script_next_connect(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)),
            some_output(MiOutput::new().with_u64("gmOutput", 0x500)),
        ]);
        wmi.script_next_connect(WMI_NAMESPACE, WMI_CLASS, "SetGamingFanBehavior", [no_output()]);
        wmi.script_next_connect(WMI_NAMESPACE, WMI_CLASS, "SetGamingKBBacklight", [no_output()]);

        let charge = FakeTransport::new();
        script_charge_ok(&charge);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let power = FakePlanApi::new();
        let mut app = AppCore::new(
            eco_picks(),
            &dir,
            Some(tripped),
            Some(SmartChargeAdapter::with_transport(charge)),
            Some(HidAdapter::with_transport(hid.clone())),
            power.clone(),
        );

        assert!(app.reconnect_unavailable());
        assert!(app.wmi_available());
        // The reconnect's eco re-check ran against the fresh adapter and
        // REJECTED eco (the only way eco_accepted becomes Some(false)).
        assert!(app.eco_disabled());
        // Enforcement re-ran: plans ensured, the eco plan applied, the
        // firmware-only items re-asserted through the fresh adapter.
        assert_eq!(power.ensure_calls(), 1);
        assert_eq!(power.set_calls(), vec!["Nitro-Eco".to_string()]);
        assert_eq!(hid.sent_reports().len(), 1);
        // The tripped adapter saw nothing after its breaker trip (the
        // reconnect path never touches the dead instance).
        assert_eq!(wmi.total(), adapter::MAX_ADAPTER_FAILURES as usize);
        remove_dir(&dir);
    }

    #[test]
    fn reconnect_failure_stays_degraded_and_does_not_spam() {
        let dir = temp_dir("reconnect-fail");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let tripped = WmiAdapter::with_transport(wmi.clone());
        for _ in 0..adapter::MAX_ADAPTER_FAILURES {
            assert!(tripped.get_platform_profile().is_err());
        }
        let charge = FakeTransport::new();
        script_charge_ok(&charge);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let power = FakePlanApi::new();
        let mut app = AppCore::new(
            eco_picks(),
            &dir,
            Some(tripped),
            Some(SmartChargeAdapter::with_transport(charge)),
            Some(HidAdapter::with_transport(hid)),
            power.clone(),
        );

        // connect() keeps failing across the episode (one-shot queue
        // drained, but a second failure is scripted too).
        wmi.script_connect_outcomes([
            Err(crate::mi::MiError {
                result: crate::mi::MI_RESULT_FAILED,
                op: "FakeTransport",
                message: None,
            }),
            Err(crate::mi::MiError {
                result: crate::mi::MI_RESULT_FAILED,
                op: "FakeTransport",
                message: None,
            }),
        ]);
        let connects_before = FakeTransport::connect_count();
        assert!(!app.reconnect_unavailable());
        assert!(!app.wmi_available());
        assert!(!app.reconnect_unavailable());
        assert!(!app.wmi_available());
        // The degradation episode produced no transport calls beyond the
        // breaker trip and no enforcement re-run (the retry-logged guard
        // swallowed the second WARN — call counts stand in for the log).
        assert_eq!(wmi.total(), adapter::MAX_ADAPTER_FAILURES as usize);
        assert_eq!(FakeTransport::connect_count() - connects_before, 2);
        assert_eq!(power.ensure_calls(), 0);
        assert!(power.set_calls().is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn reconnect_replaces_a_tripped_charge_adapter_too() {
        let dir = temp_dir("reconnect-charge");
        let wmi = FakeTransport::new();
        script_wmi_apply(&wmi);
        let charge = FakeTransport::new();
        charge.script(WMI_NAMESPACE, CHARGE_CLASS, CHARGE_GET, [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let tripped = SmartChargeAdapter::with_transport(charge.clone());
        for _ in 0..adapter::MAX_ADAPTER_FAILURES {
            assert!(tripped.is_enabled().is_err());
        }
        assert!(!tripped.is_available());
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let power = FakePlanApi::new();
        let mut app = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi.clone())),
            Some(tripped),
            Some(HidAdapter::with_transport(hid)),
            power.clone(),
        );

        assert!(app.reconnect_unavailable());
        assert!(app.wmi_available());
        assert!(app.charge_available());
        assert_eq!(power.ensure_calls(), 1);
        assert_eq!(power.set_calls(), vec!["Nitro-Balanced".to_string()]);
        assert_eq!(charge.total(), adapter::MAX_ADAPTER_FAILURES as usize);
        remove_dir(&dir);
    }

    #[test]
    fn on_power_changed_is_a_noop_when_auto_switch_is_disabled() {
        let dir = temp_dir("power-noop");
        let config = Config {
            auto_switch: false,
            ..Config::default()
        };
        let (mut app, wmi, _hid, power) = scripted_app(&dir, config);
        app.on_power_changed();
        assert_eq!(wmi.total(), 0);
        assert_eq!(power.ensure_calls(), 0);
        assert!(power.set_calls().is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn on_power_changed_enforces_the_intended_state_quietly() {
        let dir = temp_dir("power-changed");
        let (mut app, wmi, hid, power) = scripted_app(&dir, eco_picks());
        app.on_power_changed();
        assert!(!app.eco_disabled());
        assert_eq!(power.ensure_calls(), 1);
        assert_eq!(power.set_calls(), vec!["Nitro-Eco".to_string()]);
        // Eco detect wrote 6, then the apply wrote 6 again.
        assert_eq!(wmi.count("SetGamingMiscSetting"), 2);
        assert_eq!(wmi.count("GetGamingMiscSetting"), 2);
        assert_eq!(wmi.count("SetGamingFanBehavior"), 1);
        assert_eq!(wmi.count("SetGamingKBBacklight"), 1);
        assert_eq!(hid.sent_reports().len(), 1);
        remove_dir(&dir);
    }

    #[test]
    fn on_reapply_tick_reasserts_firmware_without_plan_or_ensure() {
        let dir = temp_dir("reapply-tick");
        let (mut app, wmi, hid, power) = scripted_app(&dir, eco_picks());
        app.on_reapply_tick();
        // The tick re-evaluated eco (detect: set 6 + readback accepted) and
        // re-asserted the firmware items — but never the plan.
        assert_eq!(wmi.count("SetGamingMiscSetting"), 2);
        assert_eq!(wmi.count("SetGamingFanBehavior"), 1);
        assert_eq!(wmi.count("SetGamingKBBacklight"), 1);
        assert_eq!(hid.sent_reports().len(), 1);
        assert_eq!(power.ensure_calls(), 0);
        assert!(power.set_calls().is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn on_startup_enforces_and_enables_smart_charge() {
        let dir = temp_dir("startup");
        let wmi = FakeTransport::new();
        wmi.script(WMI_NAMESPACE, WMI_CLASS, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)),
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)),
        ]);
        script_wmi_apply(&wmi);
        let charge = FakeTransport::new();
        script_charge_ok(&charge);
        let hid = FakeHidTransport::new();
        hid.script_set(vec![Ok(())]);
        let power = FakePlanApi::new();
        let mut app = AppCore::new(
            Config::default(),
            &dir,
            Some(WmiAdapter::with_transport(wmi)),
            Some(SmartChargeAdapter::with_transport(charge.clone())),
            Some(HidAdapter::with_transport(hid)),
            power.clone(),
        );
        app.on_startup();
        // Smart charge was written once (write + readback pair).
        assert_eq!(charge.count(CHARGE_SET), 1);
        assert_eq!(charge.count(CHARGE_GET), 1);
        assert_eq!(power.ensure_calls(), 2);
        assert_eq!(power.set_calls().len(), 1);
        remove_dir(&dir);
    }

    #[test]
    fn state_file_round_trip_picks() {
        let state = UserState {
            picks: Some(("quiet".to_string(), "eco".to_string())),
            ..UserState::default()
        };
        let text = serialize_state(&state);
        let loaded = load_state(&text);
        assert_eq!(loaded, state);
    }

    #[test]
    fn state_file_round_trip_all_profiles() {
        for (ac, battery) in [
            ("quiet", "balanced"),
            ("balanced", "eco"),
            ("performance", "eco"),
        ] {
            let state = UserState {
                picks: Some((ac.to_string(), battery.to_string())),
                ..UserState::default()
            };
            let text = serialize_state(&state);
            let loaded = load_state(&text);
            assert_eq!(loaded, state);
        }
    }

    #[test]
    fn state_file_serialized_text_is_toml_parseable() {
        let state = UserState {
            picks: Some(("performance".to_string(), "balanced".to_string())),
            start_at_logon: true,
        };
        let text = serialize_state(&state);
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let table = parsed.as_table().unwrap();
        assert!(table.get("smart_charge").is_none());
        assert_eq!(table.get("logon_task").unwrap().as_bool(), Some(true));
        let picks = table.get("picks").unwrap().as_table().unwrap();
        assert_eq!(picks.get("ac").unwrap().as_str(), Some("performance"));
        assert_eq!(picks.get("battery").unwrap().as_str(), Some("balanced"));
    }

    #[test]
    fn load_state_empty_and_malformed_give_defaults() {
        assert_eq!(load_state(""), UserState::default());
        assert_eq!(load_state("smart_charge = = true"), UserState::default());
        assert_eq!(load_state("[picks"), UserState::default());
    }

    #[test]
    fn load_state_partial_picks_are_ignored() {
        assert_eq!(load_state("[picks]\nac = \"quiet\""), UserState::default());
        assert_eq!(load_state("[picks]\nbattery = \"eco\""), UserState::default());
        assert_eq!(load_state("smart_charge = true"), UserState::default());
    }

    #[test]
    fn load_state_wrong_types_are_ignored() {
        assert_eq!(load_state("[picks]\nac = 5\nbattery = true"), UserState::default());
        assert_eq!(load_state("smart_charge = \"yes\""), UserState::default());
        assert_eq!(load_state("logon_task = \"yes\""), UserState::default());
    }

    #[test]
    fn load_state_ignores_unknown_keys_and_legacy_smart_charge() {
        let text = "reapply = true\nsmart_charge = false\n[other]\nkey = 1\n[picks]\nac = \"balanced\"\nbattery = \"eco\"";
        let state = load_state(text);
        assert_eq!(
            state,
            UserState {
                picks: Some(("balanced".to_string(), "eco".to_string())),
                start_at_logon: false,
            }
        );
    }

    #[test]
    fn load_state_reads_logon_task_flag() {
        assert!(load_state("logon_task = true").start_at_logon);
        assert!(!load_state("logon_task = false").start_at_logon);
        let state = load_state("logon_task = true\n[picks]\nac = \"quiet\"\nbattery = \"eco\"");
        assert_eq!(
            state,
            UserState {
                picks: Some(("quiet".to_string(), "eco".to_string())),
                start_at_logon: true,
            }
        );
    }

    #[test]
    fn apply_report_empty_means_applied() {
        assert_eq!(apply_report_text(&ApplyReport::default()), "Applied");
    }

    #[test]
    fn apply_report_lists_failed_items() {
        let report = ApplyReport { failed: vec!["plan"], skipped: Vec::new() };
        assert_eq!(apply_report_text(&report), "Failed: plan");
        let report = ApplyReport {
            failed: vec!["platform profile", "fan", "smart charge"],
            skipped: Vec::new(),
        };
        assert_eq!(
            apply_report_text(&report),
            "Failed: platform profile, fan, smart charge"
        );
    }

    #[test]
    fn apply_report_never_says_applied_when_items_were_skipped() {
        let report = ApplyReport {
            failed: Vec::new(),
            skipped: vec!["platform profile", "fan", "smart charge"],
        };
        assert_eq!(
            apply_report_text(&report),
            "Not applied: platform profile, fan, smart charge"
        );
    }

    #[test]
    fn apply_report_joins_failed_and_skipped() {
        let report = ApplyReport {
            failed: vec!["plan"],
            skipped: vec!["platform profile"],
        };
        assert_eq!(apply_report_text(&report), "Failed: plan; Not applied: platform profile");
    }
}
