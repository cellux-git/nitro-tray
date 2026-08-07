//! In-process MI control of the Acer gaming firmware
//! (`AcerGamingFunction` in `ROOT\WMI`, instance `ACPI\PNP0C14\APGe_0`).
//! Opcode/method encodings match the proven AeroForge tables (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No PowerShell/CIM fallback
//! exists — everything is raw in-process MI (`mi.dll`) via the shared `mi`
//! module, bound to the provider-enumerated instance (the `-InputObject`
//! shape; class-level invocation is rejected by this provider, ticket 16).

use std::cell::Cell;

use crate::log;
use crate::mi::{MiConnection, MiInstance, MiError, MI_RESULT_ACCESS_DENIED, MI_RESULT_INVALID_CLASS, MI_RESULT_NOT_FOUND};

/// Consecutive WMI failures after which the adapter disables itself: a
/// flapping/starving provider must stop all further calls rather than keep
/// hammering a broken transport (the recovery loop reconnects the adapter).
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
    /// An MI/WMI call failed (hr carries the `MI_RESULT` code).
    Com { hr: i32, op: &'static str },
    /// The Acer WMI instance/class was not found or is not accessible
    /// (interface unavailable).
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
const IN_PARAM: &str = "gmInput";
const OUT_PARAM: &str = "gmOutput";

pub struct WmiAdapter {
    connection: MiConnection,
    /// Consecutive failed calls; disables the adapter at `MAX_ADAPTER_FAILURES`.
    failures: Cell<u32>,
    /// When set, every call short-circuits to `NotAvailable`.
    dead: Cell<bool>,
}

// MI is thread-safe, and the markers relax thread-safety claims for the
// single-threaded core that holds the adapter (COM no longer involved).
unsafe impl Send for WmiAdapter {}
unsafe impl Sync for WmiAdapter {}

impl WmiAdapter {
    /// Connect to `ROOT\WMI` via in-process MI (`mi.dll`): initializes the MI
    /// client and a local session. Session creation does not talk to the
    /// provider, so reachability is proven by the first operation; failures
    /// trip the circuit breaker and the recovery loop reconnects.
    pub fn connect() -> Result<Self, WmiError> {
        let connection = MiConnection::connect().map_err(map_mi)?;
        Ok(Self {
            connection,
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

    /// One instance-bound MI invocation: enumerate the provider's first
    /// `AcerGamingFunction` instance (the binding target — the same shape
    /// PowerShell's `Invoke-CimMethod -InputObject` uses), build the input
    /// bag (`gmInput`, typed as declared by the MOF: `MI_UINT64` on Set*
    /// methods, `MI_UINT32` on Get* methods), invoke, read `gmOutput`.
    fn exec_method_inner(&self, method: &'static str, input: u64) -> Result<Option<u64>, WmiError> {
        let instance = self.enumerate_instance()?;
        let mut input_bag = self.connection.new_instance(CLASS_NAME).map_err(map_mi)?;
        if method.starts_with("Set") {
            input_bag.add_u64(IN_PARAM, input).map_err(map_mi)?;
        } else {
            input_bag.add_u32(IN_PARAM, input as u32).map_err(map_mi)?;
        }
        let out = self.connection.invoke(NAMESPACE, &instance, method, &input_bag).map_err(map_mi)?;
        match out {
            None => Ok(None),
            Some(result) => {
                let value = result
                    .get_u64(OUT_PARAM)
                    .map_err(map_mi)?
                    .ok_or_else(|| WmiError::Unexpected(format!("{method}: no gmOutput")))?;
                Ok(Some(value))
            }
        }
    }

    /// The provider's first `AcerGamingFunction` instance. Re-enumerated per
    /// call so a provider registration hiccup self-heals on the next call
    /// (MI is not subject to the WBEM-COM bad windows, ticket 16).
    fn enumerate_instance(&self) -> Result<MiInstance, WmiError> {
        self.connection.enumerate_first_instance(NAMESPACE, CLASS_NAME).map_err(map_mi)
    }
}

/// Maps an `MiError` to `WmiError`: interface-unavailable codes become
/// `NotAvailable` (the caller degrades), everything else is `Com`.
fn map_mi(err: MiError) -> WmiError {
    match err.result {
        MI_RESULT_INVALID_CLASS | MI_RESULT_NOT_FOUND | MI_RESULT_ACCESS_DENIED => WmiError::NotAvailable,
        _ => WmiError::Com { hr: err.result, op: err.op },
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

    #[test]
    fn mi_interface_unavailable_codes_map_to_not_available() {
        let err = MiError { result: MI_RESULT_INVALID_CLASS, op: "t", message: None };
        assert_eq!(map_mi(err), WmiError::NotAvailable);
        let err = MiError { result: MI_RESULT_NOT_FOUND, op: "t", message: None };
        assert_eq!(map_mi(err), WmiError::NotAvailable);
        let err = MiError { result: MI_RESULT_ACCESS_DENIED, op: "t", message: None };
        assert_eq!(map_mi(err), WmiError::NotAvailable);
    }

    #[test]
    fn other_mi_codes_map_to_com() {
        let err = MiError { result: crate::mi::MI_RESULT_TYPE_MISMATCH, op: "MI_Instance_SetElement", message: None };
        assert_eq!(map_mi(err), WmiError::Com { hr: 13, op: "MI_Instance_SetElement" });
        let err = MiError { result: crate::mi::MI_RESULT_INVALID_NAMESPACE, op: "t", message: None };
        assert_eq!(map_mi(err), WmiError::Com { hr: 3, op: "t" });
    }
}
