//! In-process Windows power-management wrapper. Creates the four Nitro plans
//! once from the Windows Balanced plan, activates a target plan, reads the
//! active plan, and detects plans by name. Never spawns `powercfg` or any
//! other external process. Plans are never re-tuned after creation.

use crate::policy::Profile;
use windows_sys::core::GUID;

/// The four Nitro plan names, in profile order (quiet, balanced, performance, eco).
pub const NITRO_PLANS: [&str; 4] = ["Nitro-Quiet", "Nitro-Balanced", "Nitro-Performance", "Nitro-Eco"];

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
    let _ = profile;
    todo!("ticket 04: implement")
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
}

pub struct PowerApi;

impl PowerApi {
    /// Ensure the four Nitro plans exist: for each missing plan, duplicate
    /// the Windows Balanced plan, rename it, and apply the spec's processor
    /// tuning (creation only — never re-tuned afterwards). Detected by name;
    /// no state is stored outside Windows.
    pub fn ensure_nitro_plans() -> Result<(), PowerError> {
        todo!("ticket 04: implement")
    }

    /// Activate the named plan via the in-process power APIs.
    pub fn set_active_plan(name: &str) -> Result<(), PowerError> {
        let _ = name;
        todo!("ticket 04: implement")
    }

    /// Read back the friendly name of the currently active plan.
    pub fn active_plan_name() -> Result<String, PowerError> {
        todo!("ticket 04: implement")
    }

    /// Find a plan by name; `Ok(None)` when it does not exist.
    pub fn find_plan(name: &str) -> Result<Option<GUID>, PowerError> {
        let _ = name;
        todo!("ticket 04: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 04: cpu_tuning encoding per plan table; plan-name helpers.
}
