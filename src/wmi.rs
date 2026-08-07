//! In-process raw COM/WMI control of the Acer gaming firmware
//! (`AcerGamingFunction` in `ROOT\WMI`, instance `ACPI\PNP0C14\APGe_0`).
//! Opcode/method encodings match the proven AeroForge tables (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No PowerShell/CIM fallback
//! exists — everything is raw COM `ExecMethod` via the shared `comwbem`
//! module.

use std::cell::Cell;

use crate::comwbem::{self, Bstr, ClassObject, ComApartment, ComRef, Variant, CIM_UINT32, CIM_UINT64};
use crate::log;
use windows_sys::Win32::Foundation::REGDB_E_CLASSNOTREG;
use windows_sys::Win32::System::Wmi::{WBEM_E_INVALID_CLASS, WBEM_E_NOT_FOUND};

/// Consecutive WMI failures after which the adapter disables itself: a
/// flapping/starving provider can destabilize in-proc WbemCore to the point
/// of access violations, so repeated failures must stop all further calls.
const MAX_ADAPTER_FAILURES: u32 = 5;

/// Acer firmware platform profile values (prior art, spec-confirmed).
pub const PROFILE_QUIET: u32 = 0;
pub const PROFILE_BALANCED: u32 = 1;
pub const PROFILE_PERFORMANCE: u32 = 4;
pub const PROFILE_TURBO: u32 = 5;
pub const PROFILE_ECO: u32 = 6;

/// `SetGamingMiscSetting` setting id for the platform profile (0x0B).
pub const SETTING_PLATFORM_PROFILE: u32 = 0x0B;

/// `SetGamingFanBehavior` value for fan mode AUTO.
pub const FAN_AUTO: u32 = 0x0041_0009;

/// Errors from the WMI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WmiError {
    /// A COM/WMI call failed.
    Com { hr: i32, op: &'static str },
    /// The Acer WMI instance/class was not found (interface unavailable).
    NotAvailable,
    /// An unexpected response shape.
    Unexpected(String),
}

/// Encodes a `SetGamingMiscSetting` request as `(setting, value)` — pure
/// encoding helper, unit-tested against the prior-art table.
pub fn misc_setting_request(setting: u32, value: u32) -> (u32, u32) {
    (setting, setting | (value << 8))
}

/// Encodes a `SetGamingFanBehavior` request value — pure encoding helper,
/// unit-tested (e.g. auto = 0x00410009). Non-auto maps to the max-cooling
/// behavior input (prior-art §1.5).
pub fn fan_behavior_request(auto: bool) -> u32 {
    if auto {
        FAN_AUTO
    } else {
        0x0082_0009
    }
}

/// Decodes a `GetGamingMiscSetting` gmOutput value: the second byte wins when
/// it is nonzero or the value exceeds one byte, else the low byte (AMD-shifted
/// decode, prior-art §1.6).
pub fn decode_gm_output_byte(value: u64) -> u8 {
    let shifted = ((value >> 8) & 0xFF) as u8;
    if shifted != 0 || value > 0xFF {
        shifted
    } else {
        (value & 0xFF) as u8
    }
}

const NAMESPACE: &str = "ROOT\\WMI";
const CLASS_NAME: &str = "AcerGamingFunction";
const INSTANCE_PATH: &str = "AcerGamingFunction.InstanceName=\"ACPI\\PNP0C14\\APGe_0\"";
const IN_PARAM: &str = "gmInput";
const OUT_PARAM: &str = "gmOutput";

pub struct WmiAdapter {
    services: ComRef,
    class: ClassObject,
    /// Dropped LAST: CoUninitialize must follow the Release of every COM
    /// object, so this field is declared after the interface handles.
    _com: ComApartment,
    /// Consecutive failed calls; disables the adapter at `MAX_ADAPTER_FAILURES`.
    failures: Cell<u32>,
    /// When set, every call short-circuits to `NotAvailable`.
    dead: Cell<bool>,
}

// COM objects are apartment-bound; the adapter is only ever used from the
// thread that created it (the UI thread), and the markers only relax
// thread-safety claims for the single-threaded core that holds it.
unsafe impl Send for WmiAdapter {}
unsafe impl Sync for WmiAdapter {}

impl WmiAdapter {
    /// Connect to `ROOT\WMI` in-process (CoInitializeEx + CoCreateInstance
    /// CLSID_WbemLocator + ConnectServer). Fails with `NotAvailable` when the
    /// Acer WMI interface is unreachable.
    pub fn connect() -> Result<Self, WmiError> {
        let _com = ComApartment::init().map_err(|hr| WmiError::Com { hr, op: "CoInitializeEx" })?;
        let locator =
            unsafe { comwbem::create_locator() }.map_err(|hr| {
                if hr == REGDB_E_CLASSNOTREG {
                    WmiError::NotAvailable
                } else {
                    WmiError::Com { hr, op: "CoCreateInstance(CLSID_WbemLocator)" }
                }
            })?;
        let services = unsafe { comwbem::connect_server(&locator, NAMESPACE) }
            .map_err(|hr| WmiError::Com { hr, op: "ConnectServer(ROOT\\WMI)" })?;
        let class = unsafe { comwbem::get_class(&services, CLASS_NAME) }.map_err(|hr| {
            if hr == WBEM_E_INVALID_CLASS || hr == WBEM_E_NOT_FOUND {
                WmiError::NotAvailable
            } else {
                WmiError::Com { hr, op: "GetObject(AcerGamingFunction)" }
            }
        })?;
        Ok(Self {
            _com,
            services,
            class,
            failures: Cell::new(0),
            dead: Cell::new(false),
        })
    }

    /// Adapter still usable (not disabled by a failure streak)?
    pub fn is_available(&self) -> bool {
        !self.dead.get()
    }

    /// Set the firmware platform profile (write via `SetGamingMiscSetting`).
    pub fn set_platform_profile(&self, value: u32) -> Result<(), WmiError> {
        let (_, input) = misc_setting_request(SETTING_PLATFORM_PROFILE, value);
        self.exec_method("SetGamingMiscSetting", u64::from(input))?;
        Ok(())
    }

    /// Read back the platform profile (`GetGamingMiscSetting`).
    pub fn get_platform_profile(&self) -> Result<u32, WmiError> {
        let output = self
            .exec_method("GetGamingMiscSetting", u64::from(SETTING_PLATFORM_PROFILE))?
            .ok_or_else(|| WmiError::Unexpected("GetGamingMiscSetting: no gmOutput".into()))?;
        Ok(u32::from(decode_gm_output_byte(output)))
    }

    /// Set fan behavior to auto (`SetGamingFanBehavior`).
    pub fn set_fan_auto(&self) -> Result<(), WmiError> {
        self.exec_method("SetGamingFanBehavior", u64::from(FAN_AUTO))?;
        Ok(())
    }

    /// Read back the fan behavior value.
    pub fn get_fan_behavior(&self) -> Result<u32, WmiError> {
        let output = self
            .exec_method("GetGamingFanBehavior", 0)?
            .ok_or_else(|| WmiError::Unexpected("GetGamingFanBehavior: no gmOutput".into()))?;
        Ok(output as u32)
    }

    fn exec_method(&self, method: &'static str, input: u64) -> Result<Option<u64>, WmiError> {
        if self.dead.get() {
            return Err(WmiError::NotAvailable);
        }
        let result = self.exec_method_inner(method, input);
        match &result {
            Ok(_) => self.failures.set(0),
            Err(_) => {
                let count = self.failures.get() + 1;
                self.failures.set(count);
                if count >= MAX_ADAPTER_FAILURES {
                    self.dead.set(true);
                    log::warn("wmi: adapter disabled after repeated failures; running degraded");
                }
            }
        }
        result
    }

    fn exec_method_inner(&self, method: &'static str, input: u64) -> Result<Option<u64>, WmiError> {
        unsafe {
            let method_wide = comwbem::wide(method);
            let mut in_signature: *mut core::ffi::c_void = core::ptr::null_mut();
            let mut out_signature: *mut core::ffi::c_void = core::ptr::null_mut();
            let hr = self
                .class
                .get_method(method_wide.as_ptr(), &mut in_signature, &mut out_signature);
            if hr != 0 {
                return Err(self.hr_error(hr, "GetMethod"));
            }
            drop(ComRef::from_raw(out_signature));
            if in_signature.is_null() {
                return Err(WmiError::Unexpected(format!("{method}: no input signature")));
            }
            let in_signature = ClassObject::from_raw(in_signature);

            let mut in_params: *mut core::ffi::c_void = core::ptr::null_mut();
            let hr = in_signature.spawn_instance(&mut in_params);
            if hr != 0 {
                return Err(self.hr_error(hr, "SpawnInstance"));
            }
            if in_params.is_null() {
                return Err(WmiError::Unexpected(format!("{method}: null input instance")));
            }
            let in_params = ClassObject::from_raw(in_params);

            let in_param_wide = comwbem::wide(IN_PARAM);
            // The Acer MOF declares gmInput as UInt64 on Set* methods and
            // UInt32 on Get* methods, and WMI rejects a type mismatch without
            // coercion, so try both in order (verified against the class
            // definition; the BSTR form stays as a last resort for odd SKUs).
            let ui8 = Variant::ui8(input);
            let hr = in_params.put(in_param_wide.as_ptr(), &ui8, CIM_UINT64);
            if hr != 0 {
                let ui4 = Variant::ui4(input as u32);
                let hr = in_params.put(in_param_wide.as_ptr(), &ui4, CIM_UINT32);
                if hr != 0 {
                    let text = input.to_string();
                    let bstr = Bstr::new(&text)
                        .ok_or_else(|| WmiError::Unexpected("SysAllocString(gmInput) failed".into()))?;
                    let u64_bstr = Variant::from_bstr(bstr.into_raw());
                    let hr = in_params.put(in_param_wide.as_ptr(), &u64_bstr, CIM_UINT64);
                    if hr != 0 {
                        return Err(self.hr_error(hr, "Put(gmInput)"));
                    }
                }
            }

            let path = Bstr::new(INSTANCE_PATH)
                .ok_or_else(|| WmiError::Unexpected("SysAllocString(instance path) failed".into()))?;
            let method_bstr =
                Bstr::new(method).ok_or_else(|| WmiError::Unexpected("SysAllocString(method) failed".into()))?;
            let out_params = comwbem::exec_method(&self.services, &path, &method_bstr, in_params.raw())
                .map_err(|hr| self.hr_error(hr, method))?;
            match out_params {
                None => Ok(None),
                Some(out) => {
                    let out_param_wide = comwbem::wide(OUT_PARAM);
                    let value = out.get(out_param_wide.as_ptr()).map_err(|hr| self.hr_error(hr, "Get(gmOutput)"))?;
                    Ok(value.as_u64())
                }
            }
        }
    }

    fn hr_error(&self, hr: i32, op: &'static str) -> WmiError {
        if hr == WBEM_E_NOT_FOUND {
            WmiError::Unexpected(format!("{op}: WBEM_E_NOT_FOUND"))
        } else {
            WmiError::Com { hr, op }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_platform_profile_misc_setting() {
        let (setting, input) = misc_setting_request(SETTING_PLATFORM_PROFILE, PROFILE_PERFORMANCE);
        assert_eq!(setting, 0x0B);
        assert_eq!(input, 0x40B);
        let (_, quiet) = misc_setting_request(SETTING_PLATFORM_PROFILE, PROFILE_QUIET);
        assert_eq!(quiet, 0x0B);
        let (_, eco) = misc_setting_request(SETTING_PLATFORM_PROFILE, PROFILE_ECO);
        assert_eq!(eco, 0x60B);
    }

    #[test]
    fn fan_behavior_request_matches_prior_art() {
        assert_eq!(fan_behavior_request(true), FAN_AUTO);
        assert_eq!(fan_behavior_request(false), 0x0082_0009);
    }

    #[test]
    fn decodes_amd_shifted_gm_output_bytes() {
        assert_eq!(decode_gm_output_byte(0x7300), 0x73);
        assert_eq!(decode_gm_output_byte(0x0100), 0x01);
        assert_eq!(decode_gm_output_byte(0x0400), 0x04);
        assert_eq!(decode_gm_output_byte(0x0500), 0x05);
    }

    #[test]
    fn keeps_legacy_low_byte_gm_outputs() {
        assert_eq!(decode_gm_output_byte(0x00), 0x00);
        assert_eq!(decode_gm_output_byte(0x01), 0x01);
        assert_eq!(decode_gm_output_byte(0x64), 0x64);
    }

    #[test]
    fn profile_and_fan_constants_match_prior_art() {
        assert_eq!(PROFILE_QUIET, 0);
        assert_eq!(PROFILE_BALANCED, 1);
        assert_eq!(PROFILE_PERFORMANCE, 4);
        assert_eq!(PROFILE_TURBO, 5);
        assert_eq!(PROFILE_ECO, 6);
        assert_eq!(FAN_AUTO, 0x0041_0009);
        assert_eq!(SETTING_PLATFORM_PROFILE, 0x0B);
    }
}
