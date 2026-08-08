//! Power-plan seam, split by platform (linux-port ticket 02). The Win32 body
//! lives in `win.rs`; the Linux stub in `linux.rs`.
//!
//! Shared, OS-independent surface: the Nitro plan table (`NITRO_PLANS`),
//! the per-profile CPU tuning encodings (`cpu_tuning`), the case-insensitive
//! plan-name match, the `PowerError` type, and the `PlanApi` seam `AppCore`
//! consumes.
//!
//! Windows: the in-process power-management wrapper — creates the four Nitro
//! plans once from the Windows Balanced plan, activates a target plan, reads
//! the active plan, and detects plans by name. Never spawns `powercfg` or any
//! other external process. Plans are never re-tuned after creation.
//!
//! Linux: `PowerApi` is a stub whose `PlanApi` implementation runs quiet —
//! every operation succeeds without touching the OS, so no plan failure
//! appears on any enforce occasion and `effective()` shows no plan line.
//! The sysfs governor/EPP/boost backend (plan table 1:1, no external
//! processes) lands in ticket 05.

use crate::policy::Profile;

#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
pub use win::{
    BOOST_MODE_AGGRESSIVE_VALUE_GUID, BOOST_MODE_DISABLED_VALUE_GUID, BOOST_MODE_ENABLED_VALUE_GUID,
    boost_mode_index, read_ac_index,
};

/// The four Nitro plan names, in profile order (quiet, balanced, performance,
/// eco) — derived from the policy mapping so the two tables cannot drift.
pub const NITRO_PLANS: [&str; 4] = [
    Profile::Quiet.plan_name(),
    Profile::Balanced.plan_name(),
    Profile::Performance.plan_name(),
    Profile::Eco.plan_name(),
];

/// Processor boost mode (spec plan table: off / default / aggressive).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoostMode {
    Disabled,
    Enabled,
    Aggressive,
}

/// CPU processor-state tuning per the spec plan table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuTuning {
    pub min_percent: u32,
    pub max_percent: u32,
    pub boost: BoostMode,
}

/// Spec plan table: Quiet 5/45 off, Balanced 5/99 default, Performance 5/100
/// aggressive, Eco 5/40 off. Pure encoding function, unit-tested.
pub fn cpu_tuning(profile: Profile) -> CpuTuning {
    match profile {
        Profile::Quiet => CpuTuning { min_percent: 5, max_percent: 45, boost: BoostMode::Disabled },
        Profile::Balanced => CpuTuning { min_percent: 5, max_percent: 99, boost: BoostMode::Enabled },
        Profile::Performance => CpuTuning { min_percent: 5, max_percent: 100, boost: BoostMode::Aggressive },
        Profile::Eco => CpuTuning { min_percent: 5, max_percent: 40, boost: BoostMode::Disabled },
    }
}

/// Case-insensitive plan-name comparison (Windows friendly names may differ
/// in case from the documented `NITRO_PLANS`). Pure helper, unit-tested.
pub fn plan_name_matches(actual: &str, wanted: &str) -> bool {
    actual.trim().eq_ignore_ascii_case(wanted.trim())
}

/// Errors from the in-process power APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PowerError {
    /// A Win32 power API call failed.
    Api { hr: i32, op: &'static str },
    /// A plan with the requested name was not found.
    NotFound(String),
    /// No active scheme could be read back.
    NotActive,
    /// A partial write applied: the remaining per-item failures, when some
    /// writes succeeded and the rest failed (ticket 05's sysfs EACCES path —
    /// "warning logged, remaining writes applied"). The items map verbatim
    /// into `ApplyReport.failed` for the tray's granular status.
    Partial { failed: Vec<&'static str> },
    /// The plan API is not available on this platform. Not constructed today
    /// (the Linux stub runs quiet since ticket 09); retained as the reserved
    /// vocabulary for ticket 05's backend when the sysfs path itself is
    /// unusable.
    Unavailable,
}

/// The production plan API, split by platform (linux-port ticket 02):
/// Windows is the in-process Win32 power API; Linux is a stub that runs
/// quiet (`Ok`/`Ok`/`Ok(None)`) until the ticket-05 sysfs backend lands.
pub struct PowerApi;

/// The power-plan surface the app core consumes (ticket 05): behind this
/// seam, `AppCore` tests run without touching the Win32 power APIs, which
/// cannot run under `cargo test` and would manipulate the live system.
/// Method shapes match `PowerApi`'s statics exactly, but the seam is
/// profile-typed — "plan" is Windows vocabulary, so the seam speaks
/// `Profile` and the plan-name mapping lives behind the Windows adapter.
/// The Linux stub runs quiet: no plan failures on any enforce occasion.
pub trait PlanApi {
    /// Ensure plan enforcement is possible (Windows: recreate deleted Nitro
    /// plans; Linux: no-op).
    fn ensure_support(&self) -> Result<(), PowerError>;
    /// Activate the profile's plan (Windows: activate its Nitro plan;
    /// Linux: no-op).
    fn set_profile(&self, profile: Profile) -> Result<(), PowerError>;
    /// Read back the currently active profile; `Ok(None)` when no Nitro
    /// plan is active (Linux stub: always `Ok(None)`).
    fn active_profile(&self) -> Result<Option<Profile>, PowerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_tuning_matches_spec_plan_table() {
        assert_eq!(
            cpu_tuning(Profile::Quiet),
            CpuTuning { min_percent: 5, max_percent: 45, boost: BoostMode::Disabled }
        );
        assert_eq!(
            cpu_tuning(Profile::Balanced),
            CpuTuning { min_percent: 5, max_percent: 99, boost: BoostMode::Enabled }
        );
        assert_eq!(
            cpu_tuning(Profile::Performance),
            CpuTuning { min_percent: 5, max_percent: 100, boost: BoostMode::Aggressive }
        );
        assert_eq!(
            cpu_tuning(Profile::Eco),
            CpuTuning { min_percent: 5, max_percent: 40, boost: BoostMode::Disabled }
        );
    }

    #[test]
    fn plan_name_matches_is_case_insensitive() {
        assert!(plan_name_matches("Nitro-Quiet", "nitro-quiet"));
        assert!(plan_name_matches("nitro-quiet", "Nitro-Quiet"));
        assert!(plan_name_matches("Nitro-Balanced", "Nitro-Balanced"));
        assert!(plan_name_matches(" NITRO-PERFORMANCE ", "nitro-performance"));
        assert!(!plan_name_matches("Nitro-Quiet", "Nitro-Balanced"));
        assert!(!plan_name_matches("Nitro-Quiet", "nitro-quiet-extra"));
    }

    #[test]
    fn partial_error_carries_the_per_item_failures() {
        let err = PowerError::Partial { failed: vec!["governor", "energy_perf_policy"] };
        let PowerError::Partial { failed } = &err else {
            panic!("expected Partial");
        };
        assert_eq!(failed, &vec!["governor", "energy_perf_policy"]);
    }
}
