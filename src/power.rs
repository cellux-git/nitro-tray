//! In-process Windows power-management wrapper. Creates the four Nitro plans
//! once from the Windows Balanced plan, activates a target plan, reads the
//! active plan, and detects plans by name. Never spawns `powercfg` or any
//! other external process. Plans are never re-tuned after creation.

use crate::policy::Profile;
use std::ptr;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, LocalFree,
};
use windows_sys::Win32::System::Power::{
    ACCESS_SCHEME, PowerDuplicateScheme, PowerEnumerate, PowerGetActiveScheme,
    PowerReadACValueIndex, PowerReadFriendlyName, PowerSetActiveScheme,
    PowerWriteACValueIndex, PowerWriteDCValueIndex, PowerWriteFriendlyName,
};
use windows_sys::Win32::System::SystemServices::{
    GUID_PROCESSOR_PERF_BOOST_MODE, GUID_PROCESSOR_SETTINGS_SUBGROUP,
    GUID_PROCESSOR_THROTTLE_MAXIMUM, GUID_PROCESSOR_THROTTLE_MINIMUM,
    GUID_TYPICAL_POWER_SAVINGS, PROCESSOR_PERF_BOOST_MODE_AGGRESSIVE,
    PROCESSOR_PERF_BOOST_MODE_DISABLED, PROCESSOR_PERF_BOOST_MODE_ENABLED,
};

/// The four Nitro plan names, in profile order (quiet, balanced, performance,
/// eco) — derived from the policy mapping so the two tables cannot drift.
pub const NITRO_PLANS: [&str; 4] = [
    Profile::Quiet.plan_name(),
    Profile::Balanced.plan_name(),
    Profile::Performance.plan_name(),
    Profile::Eco.plan_name(),
];

/// Profiles in the same order as `NITRO_PLANS`.
const PROFILES: [Profile; 4] = [Profile::Quiet, Profile::Balanced, Profile::Performance, Profile::Eco];

/// Registry-representation GUIDs of the processor-boost-mode value indexes.
/// windows-sys 0.61.2 does not expose them (only the index constants), so
/// they are hardcoded here. The write/read path uses the index (0/1/2) via
/// `PowerWriteACValueIndex` / `PowerReadACValueIndex`, not these GUIDs.
pub const BOOST_MODE_DISABLED_VALUE_GUID: GUID = GUID::from_u128(0x3b04d4fd_1cc7_4f23_ab1c_d1337819c4e2);
pub const BOOST_MODE_ENABLED_VALUE_GUID: GUID = GUID::from_u128(0x893dee8e_2bef_41e0_89c6_b55d0929964c);
pub const BOOST_MODE_AGGRESSIVE_VALUE_GUID: GUID = GUID::from_u128(0x36687f9e_e3a5_4dbf_b1dc_15eb381c6863);

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

/// Boost mode -> `PowerWriteACValueIndex` value (0 disabled, 1 enabled,
/// 2 aggressive). Pure encoding, unit-tested.
pub fn boost_mode_index(boost: BoostMode) -> u32 {
    match boost {
        BoostMode::Disabled => PROCESSOR_PERF_BOOST_MODE_DISABLED,
        BoostMode::Enabled => PROCESSOR_PERF_BOOST_MODE_ENABLED,
        BoostMode::Aggressive => PROCESSOR_PERF_BOOST_MODE_AGGRESSIVE,
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
}

fn api_err(hr: u32, op: &'static str) -> PowerError {
    PowerError::Api { hr: hr as i32, op }
}

/// Decode the 16-byte scheme GUID returned by `PowerEnumerate`.
fn read_guid(buf: &[u8]) -> GUID {
    let data1 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let data2 = u16::from_le_bytes(buf[4..6].try_into().unwrap());
    let data3 = u16::from_le_bytes(buf[6..8].try_into().unwrap());
    let mut data4 = [0u8; 8];
    data4.copy_from_slice(&buf[8..16]);
    GUID { data1, data2, data3, data4 }
}

/// Read the friendly name of a scheme via `PowerReadFriendlyName` (two-call
/// size/buffer pattern).
fn read_friendly_name(scheme: &GUID) -> Result<String, PowerError> {
    let mut size: u32 = 0;
    let hr = unsafe {
        PowerReadFriendlyName(
            ptr::null_mut(),
            scheme,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            &mut size,
        )
    };
    if hr != ERROR_SUCCESS && hr != ERROR_MORE_DATA {
        return Err(api_err(hr, "PowerReadFriendlyName"));
    }
    let mut buf = vec![0u8; size.max(64) as usize];
    loop {
        let mut used = buf.len() as u32;
        let hr = unsafe {
            PowerReadFriendlyName(
                ptr::null_mut(),
                scheme,
                ptr::null(),
                ptr::null(),
                buf.as_mut_ptr(),
                &mut used,
            )
        };
        match hr {
            ERROR_SUCCESS => {
                let units = ((used as usize) / 2).min(buf.len() / 2);
                let wide = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u16, units) };
                let end = wide.iter().position(|&c| c == 0).unwrap_or(units);
                return Ok(String::from_utf16_lossy(&wide[..end]));
            }
            ERROR_MORE_DATA if used > buf.len() as u32 => buf.resize(used as usize, 0),
            _ => return Err(api_err(hr, "PowerReadFriendlyName")),
        }
    }
}

/// Enumerate one power scheme by index (`ACCESS_SCHEME`); `Ok(None)` when the
/// enumeration is exhausted.
fn enumerate_scheme(index: u32) -> Result<Option<GUID>, PowerError> {
    let mut size: u32 = 0;
    let hr = unsafe {
        PowerEnumerate(
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ACCESS_SCHEME,
            index,
            ptr::null_mut(),
            &mut size,
        )
    };
    if hr == ERROR_NO_MORE_ITEMS {
        return Ok(None);
    }
    if hr != ERROR_SUCCESS && hr != ERROR_MORE_DATA {
        return Err(api_err(hr, "PowerEnumerate"));
    }
    let mut buf = vec![0u8; size.max(16) as usize];
    loop {
        let mut used = buf.len() as u32;
        let hr = unsafe {
            PowerEnumerate(
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ACCESS_SCHEME,
                index,
                buf.as_mut_ptr(),
                &mut used,
            )
        };
        match hr {
            ERROR_SUCCESS => return Ok(Some(read_guid(&buf))),
            ERROR_NO_MORE_ITEMS => return Ok(None),
            ERROR_MORE_DATA if used > buf.len() as u32 => buf.resize(used as usize, 0),
            _ => return Err(api_err(hr, "PowerEnumerate")),
        }
    }
}

/// Write one setting for both AC and DC into a scheme (creation-time tuning).
fn write_ac_dc_index(scheme: &GUID, setting: &GUID, value: u32) -> Result<(), PowerError> {
    let hr = unsafe {
        PowerWriteACValueIndex(ptr::null_mut(), scheme, &GUID_PROCESSOR_SETTINGS_SUBGROUP, setting, value)
    };
    if hr != ERROR_SUCCESS {
        return Err(api_err(hr, "PowerWriteACValueIndex"));
    }
    let hr = unsafe {
        PowerWriteDCValueIndex(ptr::null_mut(), scheme, &GUID_PROCESSOR_SETTINGS_SUBGROUP, setting, value)
    };
    if hr != ERROR_SUCCESS {
        return Err(api_err(hr, "PowerWriteDCValueIndex"));
    }
    Ok(())
}

/// Apply the spec processor tuning to a freshly duplicated scheme.
fn tune_scheme(scheme: &GUID, tuning: CpuTuning) -> Result<(), PowerError> {
    let settings = [
        (GUID_PROCESSOR_THROTTLE_MINIMUM, tuning.min_percent),
        (GUID_PROCESSOR_THROTTLE_MAXIMUM, tuning.max_percent),
        (GUID_PROCESSOR_PERF_BOOST_MODE, boost_mode_index(tuning.boost)),
    ];
    for (setting, value) in settings {
        write_ac_dc_index(scheme, &setting, value)?;
    }
    Ok(())
}

pub struct PowerApi;

/// The power-plan surface the app core consumes (ticket 05): behind this
/// seam, `AppCore` tests run without touching the Win32 power APIs, which
/// cannot run under `cargo test` and would manipulate the live system.
/// Method shapes match `PowerApi`'s statics exactly.
pub trait PlanApi {
    /// Ensure the four Nitro plans exist (recreate deleted ones).
    fn ensure_nitro_plans(&self) -> Result<(), PowerError>;
    /// Activate the named plan.
    fn set_active_plan(&self, plan: &str) -> Result<(), PowerError>;
    /// Read back the friendly name of the currently active plan.
    fn active_plan_name(&self) -> Result<String, PowerError>;
}

impl PlanApi for PowerApi {
    fn ensure_nitro_plans(&self) -> Result<(), PowerError> {
        PowerApi::ensure_nitro_plans()
    }

    fn set_active_plan(&self, plan: &str) -> Result<(), PowerError> {
        PowerApi::set_active_plan(plan)
    }

    fn active_plan_name(&self) -> Result<String, PowerError> {
        PowerApi::active_plan_name()
    }
}

impl PowerApi {
    /// Ensure the four Nitro plans exist: for each missing plan, duplicate
    /// the Windows Balanced plan, rename it, and apply the spec's processor
    /// tuning (creation only — never re-tuned afterwards). Detected by name;
    /// no state is stored outside Windows.
    pub fn ensure_nitro_plans() -> Result<(), PowerError> {
        for (profile, name) in PROFILES.iter().zip(NITRO_PLANS) {
            if Self::find_plan(name)?.is_some() {
                continue;
            }
            let mut created: *mut GUID = ptr::null_mut();
            let hr = unsafe { PowerDuplicateScheme(ptr::null_mut(), &GUID_TYPICAL_POWER_SAVINGS, &mut created) };
            if hr != ERROR_SUCCESS {
                return Err(api_err(hr, "PowerDuplicateScheme"));
            }
            let scheme = unsafe { *created };
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let hr = unsafe {
                PowerWriteFriendlyName(
                    ptr::null_mut(),
                    &scheme,
                    ptr::null(),
                    ptr::null(),
                    wide.as_ptr() as *const u8,
                    (wide.len() * 2) as u32,
                )
            };
            if hr != ERROR_SUCCESS {
                unsafe { LocalFree(created as *mut core::ffi::c_void) };
                return Err(api_err(hr, "PowerWriteFriendlyName"));
            }
            let tuned = tune_scheme(&scheme, cpu_tuning(*profile));
            unsafe { LocalFree(created as *mut core::ffi::c_void) };
            tuned?;
        }
        Ok(())
    }

    /// Activate the named plan via the in-process power APIs.
    pub fn set_active_plan(name: &str) -> Result<(), PowerError> {
        let guid = Self::find_plan(name)?.ok_or_else(|| PowerError::NotFound(name.to_string()))?;
        let hr = unsafe { PowerSetActiveScheme(ptr::null_mut(), &guid) };
        if hr == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(api_err(hr, "PowerSetActiveScheme"))
        }
    }

    /// Read back the friendly name of the currently active plan.
    pub fn active_plan_name() -> Result<String, PowerError> {
        let mut guid: *mut GUID = ptr::null_mut();
        let hr = unsafe { PowerGetActiveScheme(ptr::null_mut(), &mut guid) };
        if hr != ERROR_SUCCESS {
            return Err(api_err(hr, "PowerGetActiveScheme"));
        }
        if guid.is_null() {
            return Err(PowerError::NotActive);
        }
        let name = read_friendly_name(unsafe { &*guid });
        unsafe { LocalFree(guid as *mut core::ffi::c_void) };
        name
    }

    /// Find a plan by name; `Ok(None)` when it does not exist.
    pub fn find_plan(name: &str) -> Result<Option<GUID>, PowerError> {
        let mut index: u32 = 0;
        loop {
            match enumerate_scheme(index)? {
                Some(guid) => {
                    let actual = read_friendly_name(&guid)?;
                    if plan_name_matches(&actual, name) {
                        return Ok(Some(guid));
                    }
                }
                None => return Ok(None),
            }
            index += 1;
        }
    }
}

/// Read one AC value index from a scheme (used by the probe for readback).
pub fn read_ac_index(scheme: &GUID, setting: &GUID) -> Result<u32, PowerError> {
    let mut value: u32 = 0;
    let hr = unsafe {
        PowerReadACValueIndex(ptr::null_mut(), scheme, &GUID_PROCESSOR_SETTINGS_SUBGROUP, setting, &mut value)
    };
    if hr == ERROR_SUCCESS {
        Ok(value)
    } else {
        Err(api_err(hr, "PowerReadACValueIndex"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

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
    fn boost_mode_index_encodes_spec_indexes() {
        assert_eq!(boost_mode_index(BoostMode::Disabled), 0);
        assert_eq!(boost_mode_index(BoostMode::Enabled), 1);
        assert_eq!(boost_mode_index(BoostMode::Aggressive), 2);
    }

    #[test]
    fn read_guid_decodes_little_endian_guid_bytes() {
        let bytes: [u8; 16] = [
            0x22, 0x42, 0x1b, 0x38, 0x94, 0xf6, 0xf0, 0x41, 0x96, 0x85, 0xff, 0x5b, 0xb2, 0x60, 0xdf, 0x2e,
        ];
        let expected = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
        assert!(guid_eq(&read_guid(&bytes), &expected));
    }
}
