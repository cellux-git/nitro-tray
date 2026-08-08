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

use super::{BoostMode, CpuTuning, NITRO_PLANS, PlanApi, PowerApi, PowerError, cpu_tuning, plan_name_matches};

/// Profiles in the same order as `NITRO_PLANS`.
const PROFILES: [Profile; 4] = [Profile::Quiet, Profile::Balanced, Profile::Performance, Profile::Eco];

/// Registry-representation GUIDs of the processor-boost-mode value indexes.
/// windows-sys 0.61.2 does not expose them (only the index constants), so
/// they are hardcoded here. The write/read path uses the index (0/1/2) via
/// `PowerWriteACValueIndex` / `PowerReadACValueIndex`, not these GUIDs.
pub const BOOST_MODE_DISABLED_VALUE_GUID: GUID = GUID::from_u128(0x3b04d4fd_1cc7_4f23_ab1c_d1337819c4e2);
pub const BOOST_MODE_ENABLED_VALUE_GUID: GUID = GUID::from_u128(0x893dee8e_2bef_41e0_89c6_b55d0929964c);
pub const BOOST_MODE_AGGRESSIVE_VALUE_GUID: GUID = GUID::from_u128(0x36687f9e_e3a5_4dbf_b1dc_15eb381c6863);

/// Boost mode -> `PowerWriteACValueIndex` value (0 disabled, 1 enabled,
/// 2 aggressive). Pure encoding, unit-tested.
pub fn boost_mode_index(boost: BoostMode) -> u32 {
    match boost {
        BoostMode::Disabled => PROCESSOR_PERF_BOOST_MODE_DISABLED,
        BoostMode::Enabled => PROCESSOR_PERF_BOOST_MODE_ENABLED,
        BoostMode::Aggressive => PROCESSOR_PERF_BOOST_MODE_AGGRESSIVE,
    }
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

impl PlanApi for PowerApi {
    fn ensure_support(&self) -> Result<(), PowerError> {
        PowerApi::ensure_support()
    }

    fn set_profile(&self, profile: Profile) -> Result<(), PowerError> {
        PowerApi::set_profile(profile)
    }

    fn active_profile(&self) -> Result<Option<Profile>, PowerError> {
        PowerApi::active_profile()
    }
}

impl PowerApi {
    /// Ensure the four Nitro plans exist: for each missing plan, duplicate
    /// the Windows Balanced plan, rename it, and apply the spec's processor
    /// tuning (creation only — never re-tuned afterwards). Detected by name;
    /// no state is stored outside Windows.
    pub fn ensure_support() -> Result<(), PowerError> {
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

    /// Activate the profile's Nitro plan via the in-process power APIs.
    pub fn set_profile(profile: Profile) -> Result<(), PowerError> {
        let name = profile.plan_name();
        let guid = Self::find_plan(name)?.ok_or_else(|| PowerError::NotFound(name.to_string()))?;
        let hr = unsafe { PowerSetActiveScheme(ptr::null_mut(), &guid) };
        if hr == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(api_err(hr, "PowerSetActiveScheme"))
        }
    }

    /// Read back the currently active profile: the active scheme's friendly
    /// name mapped through the plan table. `Ok(None)` when the active scheme
    /// is not one of the four Nitro plans.
    pub fn active_profile() -> Result<Option<Profile>, PowerError> {
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
        Ok(profile_from_active_name(&name?))
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

/// The profile whose Nitro plan matches `name` case-insensitively; `None`
/// when the name matches no Nitro plan. Pure, unit-tested.
fn profile_from_active_name(name: &str) -> Option<Profile> {
    NITRO_PLANS
        .iter()
        .find(|plan| plan_name_matches(name, plan))
        .and_then(|plan| Profile::from_plan_name(plan))
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

    #[test]
    fn profile_from_active_name_maps_nitro_plan_names() {
        assert_eq!(profile_from_active_name("Nitro-Performance"), Some(Profile::Performance));
        assert_eq!(profile_from_active_name("nitro-performance"), Some(Profile::Performance));
        assert_eq!(profile_from_active_name("Nitro-Balanced"), Some(Profile::Balanced));
        assert_eq!(profile_from_active_name("Nitro-Eco"), Some(Profile::Eco));
        assert_eq!(profile_from_active_name("Nitro-Quiet"), Some(Profile::Quiet));
        assert_eq!(profile_from_active_name("Nitro-Turbo"), None);
        assert_eq!(profile_from_active_name("Balanced"), None);
        assert_eq!(profile_from_active_name(""), None);
    }
}
