//! In-process MI control of the Acer gaming firmware
//! (`AcerGamingFunction` in `ROOT\WMI`, instance `ACPI\PNP0C14\APGe_0`).
//! Opcode/method encodings match the proven AeroForge tables (see
//! `docs/firmware-notes.md`). No PowerShell/CIM fallback
//! exists — everything is raw in-process MI (`mi.dll`) via the shared `mi`
//! module, bound to the provider-enumerated instance (the `-InputObject`
//! shape; class-level invocation is rejected by this provider, ticket 16).
//!
//! The failure-streak circuit breaker, the adapter error type, and the
//! MI→adapter error mapping live in the shared `adapter` module; every
//! public operation here runs through `CircuitBreaker::guarded`
//! (ticket 04).

use crate::adapter::{map_mi, AdapterError, CircuitBreaker, WMI_NAMESPACE};
use crate::mi::MiConnection;
use crate::transport::{MiInput, MiTransport};

/// `SetGamingMiscSetting` setting id for the platform profile (0x0B).
pub const SETTING_PLATFORM_PROFILE: u32 = 0x0B;

/// `SetGamingFanBehavior` value for fan mode AUTO.
pub const FAN_AUTO: u32 = 0x0041_0009;

/// 16-byte `SetGamingKBBacklight` config that turns the keyboard backlight
/// off: mode 0 (static), brightness 0, byte 9 (apply flag) 1, rest zero
/// (see `docs/firmware-notes.md`).
pub const KEYBOARD_BACKLIGHT_OFF: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];

/// Encodes a `SetGamingMiscSetting` request as `(setting, value)` — pure
/// encoding helper, unit-tested against the documented encoding.
pub fn misc_setting_request(setting: u32, value: u32) -> (u32, u32) {
    (setting, setting | (value << 8))
}

/// Encodes a `SetGamingFanBehavior` request value — pure encoding helper,
/// unit-tested (e.g. auto = 0x00410009). Non-auto maps to the max-cooling
/// behavior input (unused encoding, see `docs/firmware-notes.md`).
pub fn fan_behavior_request(auto: bool) -> u32 {
    if auto {
        FAN_AUTO
    } else {
        0x0082_0009
    }
}

/// Decodes a `GetGamingMiscSetting` gmOutput value: the second byte wins when
/// it is nonzero or the value exceeds one byte, else the low byte (AMD-shifted
/// decode, see `docs/firmware-notes.md`).
pub fn decode_gm_output_byte(value: u64) -> u8 {
    let shifted = ((value >> 8) & 0xFF) as u8;
    if shifted != 0 || value > 0xFF {
        shifted
    } else {
        (value & 0xFF) as u8
    }
}

/// `AcerGamingFunction` class and parameter names for the gaming-firmware
/// protocol — public so the diagnostic probes print the same strings the
/// adapter sends.
pub const CLASS_NAME: &str = "AcerGamingFunction";
pub const IN_PARAM: &str = "gmInput";
pub const OUT_PARAM: &str = "gmOutput";

/// Acer gaming-firmware WMI adapter over a `MiTransport` seam: production
/// uses `WmiAdapter::connect()` (a real `MiConnection`), tests use
/// `WmiAdapter::with_transport(fake)`. The shared circuit breaker and every
/// typed invoke helper are exercised through the seam.
pub struct WmiAdapter<M: MiTransport = MiConnection> {
    transport: M,
    /// Shared failure-streak circuit breaker: trips at
    /// `adapter::MAX_ADAPTER_FAILURES` and short-circuits every call.
    breaker: CircuitBreaker,
}

// MI is thread-safe, and the markers relax thread-safety claims for the
// single-threaded core that holds the adapter (COM no longer involved).
// Unsafe: the adapter serializes all transport access on its owning thread,
// and every `MiTransport` implementor in this crate is Send+Sync.
unsafe impl<M: MiTransport> Send for WmiAdapter<M> {}
unsafe impl<M: MiTransport> Sync for WmiAdapter<M> {}

impl<M: MiTransport> WmiAdapter<M> {
    /// Connect through the per-platform transport seam (the transport is the
    /// per-platform module now): Windows `MiConnection` initiates the MI
    /// client and a local session; the Linux `MiConnection` stub returns
    /// `NOT_FOUND`, mapped to `NotAvailable` by `map_mi` — the entry point
    /// degrades exactly like the old hardcoded stub. Session creation does
    /// not talk to the provider, so reachability is proven by the first
    /// operation; failures trip the circuit breaker and the recovery loop
    /// reconnects.
    pub fn connect() -> Result<Self, AdapterError> {
        <M as MiTransport>::connect().map_err(map_mi).map(Self::with_transport)
    }

    /// Wrap any `MiTransport` (the test seam).
    pub fn with_transport(transport: M) -> Self {
        Self {
            transport,
            breaker: CircuitBreaker::new(
                "wmi: adapter disabled after repeated failures; running degraded",
            ),
        }
    }

    /// Adapter still usable (not disabled by a failure streak)?
    pub fn is_available(&self) -> bool {
        self.breaker.is_available()
    }

    /// Set the firmware platform profile (write via `SetGamingMiscSetting`;
    /// `gmInput` is declared `UInt64` in the MOF).
    pub fn set_platform_profile(&self, value: u32) -> Result<(), AdapterError> {
        let (_, input) = misc_setting_request(SETTING_PLATFORM_PROFILE, value);
        self.exec_method(
            "SetGamingMiscSetting",
            MiInput::new(CLASS_NAME).u64(IN_PARAM, u64::from(input)),
        )?;
        Ok(())
    }

    /// Read back the platform profile (`GetGamingMiscSetting`; `gmInput` is
    /// declared `UInt32` in the MOF).
    pub fn get_platform_profile(&self) -> Result<u32, AdapterError> {
        let output = self
            .exec_method(
                "GetGamingMiscSetting",
                MiInput::new(CLASS_NAME).u32(IN_PARAM, SETTING_PLATFORM_PROFILE),
            )?
            .ok_or_else(|| AdapterError::Unexpected("GetGamingMiscSetting: no gmOutput".into()))?;
        Ok(u32::from(decode_gm_output_byte(output)))
    }

    /// Set fan behavior to auto (`SetGamingFanBehavior`; `gmInput` is
    /// declared `UInt64`).
    pub fn set_fan_auto(&self) -> Result<(), AdapterError> {
        self.exec_method(
            "SetGamingFanBehavior",
            MiInput::new(CLASS_NAME).u64(IN_PARAM, u64::from(FAN_AUTO)),
        )?;
        Ok(())
    }

    /// Read back the fan behavior value (`GetGamingFanBehavior`; `gmInput`
    /// is declared `UInt32`).
    pub fn get_fan_behavior(&self) -> Result<u32, AdapterError> {
        let output = self
            .exec_method("GetGamingFanBehavior", MiInput::new(CLASS_NAME).u32(IN_PARAM, 0))?
            .ok_or_else(|| AdapterError::Unexpected("GetGamingFanBehavior: no gmOutput".into()))?;
        Ok(output as u32)
    }

    /// Turn the keyboard backlight off (`SetGamingKBBacklight` with the
    /// 16-byte off config; `gmInput` is declared `UInt8Array` — the only
    /// config this app ever writes: when the user opts out, the keyboard
    /// lighting is left untouched).
    pub fn set_keyboard_backlight_off(&self) -> Result<(), AdapterError> {
        self.exec_method(
            "SetGamingKBBacklight",
            MiInput::new(CLASS_NAME).u8_array(IN_PARAM, KEYBOARD_BACKLIGHT_OFF.to_vec()),
        )?;
        Ok(())
    }

    /// One instance-bound MI invocation through the seam, guarded by the
    /// shared failure-streak circuit breaker.
    fn exec_method(&self, method: &'static str, input: MiInput) -> Result<Option<u64>, AdapterError> {
        self.breaker.guarded(|| self.exec_method_inner(method, input))
    }

    /// One instance-bound MI invocation: the transport enumerates the
    /// provider's first `AcerGamingFunction` instance (the binding target —
    /// the same shape PowerShell's `Invoke-CimMethod -InputObject` uses),
    /// builds the input bag from the typed `MiInput` (each element typed as
    /// declared by the MOF — no method-name inference), invokes, and reads
    /// `gmOutput` from the out-params instance. Unguarded: every public
    /// entry point runs this through `exec_method`'s breaker guard.
    fn exec_method_inner(&self, method: &'static str, input: MiInput) -> Result<Option<u64>, AdapterError> {
        let out = self
            .transport
            .invoke_first_instance(WMI_NAMESPACE, CLASS_NAME, method, &input)
            .map_err(map_mi)?;
        match out {
            None => Ok(None),
            Some(output) => match output.u64(OUT_PARAM).map_err(map_mi)? {
                Some(value) => Ok(Some(value)),
                // An out-params instance without `gmOutput` is a protocol
                // anomaly, not an absence — a Set that silently succeeded
                // here would report success for an unwritten value.
                None => Err(AdapterError::Unexpected(format!("{method}: no gmOutput"))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Profile;
    use crate::testing::{FakeTransport, no_output, some_output, transport_error};
    use crate::transport::{MiElement, MiOutput, MiValue};

    #[test]
    fn encodes_platform_profile_misc_setting() {
        let (setting, input) =
            misc_setting_request(SETTING_PLATFORM_PROFILE, Profile::Performance.firmware_value());
        assert_eq!(setting, 0x0B);
        assert_eq!(input, 0x40B);
        let (_, quiet) =
            misc_setting_request(SETTING_PLATFORM_PROFILE, Profile::Quiet.firmware_value());
        assert_eq!(quiet, 0x0B);
        let (_, eco) = misc_setting_request(SETTING_PLATFORM_PROFILE, Profile::Eco.firmware_value());
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
    fn fan_and_setting_constants_match_prior_art() {
        assert_eq!(FAN_AUTO, 0x0041_0009);
        assert_eq!(SETTING_PLATFORM_PROFILE, 0x0B);
    }

    #[test]
    fn keyboard_off_payload_is_16_bytes_static_zero_brightness() {
        assert_eq!(KEYBOARD_BACKLIGHT_OFF.len(), 16);
        assert_eq!(KEYBOARD_BACKLIGHT_OFF[0], 0); // mode: static
        assert_eq!(KEYBOARD_BACKLIGHT_OFF[2], 0); // brightness: off
        assert_eq!(KEYBOARD_BACKLIGHT_OFF[9], 1); // apply flag
    }

    #[test]
    fn breaker_trips_after_five_consecutive_failures() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        for _ in 0..5 {
            assert!(adapter.get_platform_profile().is_err());
        }
        assert!(!adapter.is_available());
        // Tripped: further calls short-circuit without touching the transport.
        assert!(adapter.get_platform_profile().is_err());
        assert_eq!(fake.count("GetGamingMiscSetting"), 5);
    }

    #[test]
    fn breaker_resets_on_success_before_the_threshold() {
        let fake = FakeTransport::new();
        let fail = transport_error(crate::mi::MI_RESULT_FAILED);
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            fail.clone(), fail.clone(), fail.clone(), fail.clone(),
            some_output(MiOutput::new().with_u64("gmOutput", 0x100)),
            fail.clone(), fail.clone(), fail.clone(), fail.clone(),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        for _ in 0..4 {
            assert!(adapter.get_platform_profile().is_err());
        }
        assert!(adapter.is_available());
        assert!(adapter.get_platform_profile().is_ok());
        for _ in 0..4 {
            assert!(adapter.get_platform_profile().is_err());
        }
        // 4 + 1 + 4 errors never reach the threshold: the success reset the streak.
        assert!(adapter.is_available());
        assert_eq!(fake.count("GetGamingMiscSetting"), 9);
    }

    #[test]
    fn set_and_get_typing_is_explicit_not_name_dispatch() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingMiscSetting", [no_output()]);
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        adapter.set_platform_profile(Profile::Eco.firmware_value()).unwrap();
        adapter.get_platform_profile().unwrap();
        let calls = fake.calls();
        assert_eq!(calls[0].method, "SetGamingMiscSetting");
        // MOF: SetGamingMiscSetting's gmInput is UInt64 — the write must NOT
        // be dispatched by the "Set" name prefix (ticket-16 bug class).
        assert_eq!(calls[0].input.elements, vec![MiElement { name: "gmInput", value: MiValue::U64(0x60B) }]);
        assert_eq!(calls[1].method, "GetGamingMiscSetting");
        // GetGamingMiscSetting's gmInput is UInt32.
        assert_eq!(calls[1].input.elements, vec![MiElement { name: "gmInput", value: MiValue::U32(0x0B) }]);
    }

    #[test]
    fn fan_methods_are_typed_u64_write_u32_read() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingFanBehavior", [no_output()]);
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingFanBehavior", [
            some_output(MiOutput::new().with_u64("gmOutput", u64::from(FAN_AUTO))),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        adapter.set_fan_auto().unwrap();
        assert_eq!(adapter.get_fan_behavior().unwrap(), FAN_AUTO);
        let calls = fake.calls();
        assert_eq!(calls[0].input.elements, vec![MiElement { name: "gmInput", value: MiValue::U64(u64::from(FAN_AUTO)) }]);
        assert_eq!(calls[1].input.elements, vec![MiElement { name: "gmInput", value: MiValue::U32(0) }]);
    }

    #[test]
    fn keyboard_off_write_is_a_typed_u8_array() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingKBBacklight", [no_output()]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        adapter.set_keyboard_backlight_off().unwrap();
        assert_eq!(
            fake.calls()[0].input.elements,
            vec![MiElement { name: "gmInput", value: MiValue::U8Array(KEYBOARD_BACKLIGHT_OFF.to_vec()) }]
        );
    }

    #[test]
    fn eco_protocol_readback_accepts_profile_six() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingMiscSetting", [no_output()]);
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x600)),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        adapter.set_platform_profile(Profile::Eco.firmware_value()).unwrap();
        assert_eq!(adapter.get_platform_profile().unwrap(), Profile::Eco.firmware_value());
        assert_eq!(fake.total(), 2);
    }

    #[test]
    fn eco_protocol_readback_rejects_other_profiles() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingMiscSetting", [no_output()]);
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            some_output(MiOutput::new().with_u64("gmOutput", 0x500)),
        ]);
        let adapter = WmiAdapter::with_transport(fake.clone());
        adapter.set_platform_profile(Profile::Eco.firmware_value()).unwrap();
        assert_ne!(adapter.get_platform_profile().unwrap(), Profile::Eco.firmware_value());
    }

    #[test]
    fn missing_gm_output_in_the_out_instance_is_unexpected() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "GetGamingMiscSetting", [
            some_output(MiOutput::new()),
        ]);
        let adapter = WmiAdapter::with_transport(fake);
        assert!(matches!(adapter.get_platform_profile(), Err(AdapterError::Unexpected(_))));
    }

    #[test]
    fn set_without_gm_output_errors_instead_of_silently_succeeding() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, "SetGamingMiscSetting", [
            some_output(MiOutput::new()),
        ]);
        let adapter = WmiAdapter::with_transport(fake);
        assert!(matches!(
            adapter.set_platform_profile(Profile::Balanced.firmware_value()),
            Err(AdapterError::Unexpected(_))
        ));
    }
}
