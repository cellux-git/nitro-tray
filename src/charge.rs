//! Smart-charge adapter: in-process control of the 80% charge cap via the
//! `BatteryControl` WMI health-status toggle. Single-SKU target: the
//! AN16S-61. The write tuple is `(uBatteryNo=1, uFunctionMask=1,
//! uFunctionStatus=status)`; with status 1 the battery stops charging at
//! 80%, with status 0 it charges to full (both confirmed on-device). The
//! readback is a single pair (battery 1, query 1) whose `uFunctionStatus[0]`
//! reflects the cap state. No interpreter is spawned. In-process MI
//! (`mi.dll`) via the shared `mi` module, bound to the provider-enumerated
//! instance (ticket 16).
//!
//! No sweeping, ever: the readback is a single pair (battery 1, function
//! query 1), never the 35-call `uBatteryNo` x `uFunctionQuery` sweep. A
//! full-sweep discovery mode is deliberately absent; if a future machine
//! needs it, that is a config/decision point, not a default.
//!
//! The failure-streak circuit breaker, the adapter error type, and the
//! MI→adapter error mapping live in the shared `adapter` module; every
//! public operation here runs through `CircuitBreaker::guarded`
//! (ticket 04).

use crate::adapter::{map_mi, AdapterError, CircuitBreaker, WMI_NAMESPACE};
use crate::mi::MiConnection;
use crate::transport::{MiInput, MiTransport};

/// Single-pair readback tuple (battery 1, query 1): the only pair that
/// answers on the AN16S-61 — `uFunctionList=3`, `uFunctionStatus=[1,0,0,0,0]`
/// with the cap in effect; battery 0 returns an empty row. The query value
/// is irrelevant (every query returns the same row).
const READBACK_BATTERY: u8 = 1;
const READBACK_QUERY: u8 = 1;

/// `BatteryControl` class and method names for the smart-charge protocol —
/// public so the diagnostic probes print the same strings the adapter sends.
pub const CLASS_NAME: &str = "BatteryControl";
pub const METHOD_SET: &str = "SetBatteryHealthControl";
pub const METHOD_GET: &str = "GetBatteryHealthControlStatus";

/// Encodes the smart-charge write tuple (`uBatteryNo=1, uFunctionMask=1,
/// uFunctionStatus=status`, 5-zero reserved). Status 1 arms the 80% cap,
/// status 0 clears it; `uFunctionStatus[0]` in the readback row follows the
/// write.
pub fn write_tuple(status: u8) -> (u8, u8, u8, [u8; 5]) {
    (1, 1, status, [0, 0, 0, 0, 0])
}

/// Decodes the health-status byte from readback rows. Prefers the
/// first row where `uFunctionList & 1 != 0` and `uFunctionStatus[0]` is a real
/// status (the cap byte); falls back to any status byte of 0/1; last
/// resort is the max non-255 status byte. The single-pair readback feeds
/// exactly one row here.
pub fn desired_status_from_rows(rows: &[(u32, &[u8])]) -> Option<u8> {
    for (list, statuses) in rows {
        if list & 1 != 0 && !statuses.is_empty() && statuses[0] != 0xFF {
            return Some(statuses[0]);
        }
    }
    for (_, statuses) in rows {
        if let Some(&s) = statuses.iter().find(|&&b| b == 0 || b == 1) {
            return Some(s);
        }
    }
    rows.iter()
        .flat_map(|(_, statuses)| statuses.iter().copied())
        .filter(|&b| b != 0xFF)
        .max()
}

/// A `SetBatteryHealthControl` attempt succeeds only when the MI operation
/// reports success AND the provider's `ReturnValue` is present, truthy, and
/// not an error code; a missing `ReturnValue` means the attempt failed.
pub fn method_succeeded(hr: i32, return_value: Option<u32>) -> bool {
    hr == 0 && return_value.is_some_and(|rv| rv != 0 && rv < 0x8000_0000)
}

/// One `GetBatteryHealthControlStatus` readback row: the `uFunctionList`
/// value, the `uFunctionStatus` byte array, and the method's `uReturn`
/// (present only when the provider declares it as a scalar — the 
/// `uFunctionStatus[0]` byte is the health-status the cap state decodes
/// from).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChargeRow {
    /// `uFunctionList` — the row's function-list bitmap.
    pub function_list: u32,
    /// `uFunctionStatus` — per-function status bytes; index 0 carries the
    /// health-status byte.
    pub status: Vec<u8>,
    /// `uReturn` scalar when the provider answers with one (absent on the
    /// AN16S-61, where the readback's `uReturn` is an array).
    pub return_value: Option<u32>,
}

/// Smart-charge adapter over a `MiTransport` seam: production uses
/// `SmartChargeAdapter::connect()` (a real `MiConnection`), tests use
/// `SmartChargeAdapter::with_transport(fake)`. The shared circuit breaker
/// and the write→verify protocol are exercised through the seam.
pub struct SmartChargeAdapter<M: MiTransport = MiConnection> {
    transport: M,
    /// Shared failure-streak circuit breaker: trips at
    /// `adapter::MAX_ADAPTER_FAILURES` and short-circuits every call.
    breaker: CircuitBreaker,
}

// MI is thread-safe, and the markers relax thread-safety claims for the
// single-threaded core that holds the adapter (COM no longer involved).
// Unsafe: the adapter serializes all transport access on its owning thread,
// and every `MiTransport` implementor in this crate is Send+Sync.
unsafe impl<M: MiTransport> Send for SmartChargeAdapter<M> {}
unsafe impl<M: MiTransport> Sync for SmartChargeAdapter<M> {}

impl<M: MiTransport> SmartChargeAdapter<M> {
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
            breaker: CircuitBreaker::new("charge: adapter disabled after repeated failures"),
        }
    }

    /// Adapter still usable (not disabled by a failure streak)?
    pub fn is_available(&self) -> bool {
        self.breaker.is_available()
    }

    /// Toggle the 80% charge cap via the smart-charge write tuple
    /// (`SetBatteryHealthControl`, battery 1, mask 1). Success requires the
    /// readback match; a lying or rejected write is an error (the minute
    /// tick retries on the next readback).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), AdapterError> {
        self.breaker.guarded(|| {
            let status = u8::from(enabled);
            let (battery, mask, status_byte, _reserved) = write_tuple(status);
            let return_value = match self.exec_set(battery, mask, status_byte) {
                Ok(Some(value)) => value,
                Ok(None) => {
                    return Err(AdapterError::Unexpected(format!(
                        "SetBatteryHealthControl ({battery},{mask},{status_byte}): no ReturnValue"
                    )));
                }
                Err(e) => return Err(e),
            };
            if !method_succeeded(0, Some(return_value)) {
                return Err(AdapterError::Unexpected(format!(
                    "SetBatteryHealthControl ({battery},{mask},{status_byte}) rejected (ReturnValue={return_value})"
                )));
            }
            // A truthy ReturnValue alone is not proof of effect — the
            // readback match is what decides; the byte follows the write
            // synchronously.
            match self.verify_status(READBACK_BATTERY, READBACK_QUERY, status) {
                Ok(true) => Ok(()),
                Ok(false) => Err(AdapterError::Unexpected(format!(
                    "SetBatteryHealthControl ({battery},{mask},{status_byte}) accepted but readback did not match (ReturnValue={return_value})"
                ))),
                Err(e) => Err(e),
            }
        })
    }

    /// Read back the current smart-charge state
    /// (`GetBatteryHealthControlStatus`): one single-pair readback (battery
    /// 1, query 1), decoded from function-list bit 0 / status byte index 0.
    /// No sweeping; a rejected or empty row degrades to an error (the caller
    /// shows `None`).
    pub fn is_enabled(&self) -> Result<bool, AdapterError> {
        self.breaker.guarded(|| {
            let Some(row) = self.exec_get(READBACK_BATTERY, READBACK_QUERY)? else {
                return Err(AdapterError::Unexpected(
                    "no GetBatteryHealthControlStatus row for the (battery 1, query 1) pair".into(),
                ));
            };
            let refs = [(row.function_list, row.status.as_slice())];
            match desired_status_from_rows(&refs) {
                Some(status) => Ok(status == 1),
                None => Err(AdapterError::Unexpected(
                    "no GetBatteryHealthControlStatus row for the (battery 1, query 1) pair".into(),
                )),
            }
        })
    }

    /// One `GetBatteryHealthControlStatus` row for an arbitrary (battery,
    /// query) pair — the diagnostic surface the probes sweep. `Ok(None)` when
    /// the provider rejects the pair or the row carries no list/status data;
    /// transport errors propagate.
    pub fn read_row(&self, battery: u8, query: u8) -> Result<Option<ChargeRow>, AdapterError> {
        self.breaker.guarded(|| self.exec_get(battery, query))
    }

    /// One `SetBatteryHealthControl` write tuple for an arbitrary (battery,
    /// mask, status) — the diagnostic surface the probes write with. Returns
    /// the provider `ReturnValue` (falls back to `uReturn`); `Ok(None)` when
    /// the provider produced no return value at all.
    pub fn write_tuple(&self, battery: u8, mask: u8, status: u8) -> Result<Option<u32>, AdapterError> {
        self.breaker.guarded(|| self.exec_set(battery, mask, status))
    }

    /// Single-pair readback of the health-status byte (`Ok(None)` when the
    /// provider rejects the (battery, query) pair or the row carries no
    /// decodable status).
    fn read_status(&self, battery: u8, query: u8) -> Result<Option<bool>, AdapterError> {
        let Some(row) = self.exec_get(battery, query)? else { return Ok(None) };
        let refs = [(row.function_list, row.status.as_slice())];
        match desired_status_from_rows(&refs) {
            Some(status) => Ok(Some(status == 1)),
            None => Ok(None),
        }
    }

    /// Write verification: the health-status byte read back for the readback
    /// pair must equal the requested status.
    fn verify_status(&self, battery: u8, query: u8, requested: u8) -> Result<bool, AdapterError> {
        match self.read_status(battery, query)? {
            Some(actual) => Ok(actual == (requested == 1)),
            None => Ok(false),
        }
    }

    /// Executes `SetBatteryHealthControl` with one typed tuple; returns the
    /// provider `ReturnValue` when present (falls back to the declared
    /// `uReturn` out parameter when MI does not synthesize a ReturnValue).
    fn exec_set(&self, battery: u8, mask: u8, status: u8) -> Result<Option<u32>, AdapterError> {
        let input = MiInput::new(CLASS_NAME)
            .u8("uBatteryNo", battery)
            .u8("uFunctionMask", mask)
            .u8("uFunctionStatus", status)
            .u8_array("uReservedIn", vec![0; 5]);
        let out = self
            .transport
            .invoke_first_instance(WMI_NAMESPACE, CLASS_NAME, METHOD_SET, &input)
            .map_err(map_mi)?;
        let Some(output) = out else { return Ok(None) };
        if let Some(value) = output.u32("ReturnValue").map_err(map_mi)? {
            return Ok(Some(value));
        }
        output.u32("uReturn").map_err(map_mi)
    }

    /// Executes one `GetBatteryHealthControlStatus` query row. Returns
    /// `Ok(None)` when the provider rejects the (battery, query) pair or the
    /// row carries no list/status data; transport errors propagate (a probe
    /// must see them, not mistake them for an empty row).
    fn exec_get(&self, battery: u8, query: u8) -> Result<Option<ChargeRow>, AdapterError> {
        let input = MiInput::new(CLASS_NAME)
            .u8("uBatteryNo", battery)
            .u8("uFunctionQuery", query)
            .u8_array("uReserved", vec![0; 2]);
        let out = self
            .transport
            .invoke_first_instance(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, &input)
            .map_err(map_mi)?;
        let Some(output) = out else { return Ok(None) };
        let Some(list) = output.u32("uFunctionList").map_err(map_mi)? else {
            return Ok(None);
        };
        let Some(statuses) = output.u8_array("uFunctionStatus").map_err(map_mi)? else {
            return Ok(None);
        };
        if statuses.is_empty() {
            return Ok(None);
        }
        let return_value = output.u32("uReturn").map_err(map_mi)?;
        Ok(Some(ChargeRow { function_list: list, status: statuses, return_value }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeTransport, some_output, transport_error};
    use crate::transport::{MiElement, MiOutput, MiValue};

    /// Script one `SetBatteryHealthControl` outcome: `return_value = Some(rv)`
    /// yields an out instance carrying that `ReturnValue`; `None` yields an
    /// out instance without one.
    fn script_charge_set(fake: &FakeTransport, return_value: Option<u32>) {
        let output = match return_value {
            Some(rv) => some_output(MiOutput::new().with_u32("ReturnValue", rv)),
            None => some_output(MiOutput::new()),
        };
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_SET, [output]);
    }

    /// Script one `GetBatteryHealthControlStatus` outcome carrying the given
    /// `uFunctionList` / `uFunctionStatus` row.
    fn script_charge_get(fake: &FakeTransport, list: u32, statuses: &[u8]) {
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, [
            some_output(MiOutput::new().with_u32("uFunctionList", list).with_u8_array("uFunctionStatus", statuses.to_vec())),
        ]);
    }

    #[test]
    fn write_tuple_encodes_the_direct_trust_path() {
        assert_eq!(write_tuple(1), (1, 1, 1, [0, 0, 0, 0, 0]));
        assert_eq!(write_tuple(0), (1, 1, 0, [0, 0, 0, 0, 0]));
    }

    #[test]
    fn prefers_function_list_bit0_row_status_byte() {
        let rows = [
            (0u32, &[255u8, 0u8][..]),
            (3u32, &[1u8, 0u8][..]),
        ];
        assert_eq!(desired_status_from_rows(&rows), Some(1));
    }

    #[test]
    fn decodes_the_live_an16s_61_readback_row() {
        // Live on the AN16S-61: battery 1 answers `uFunctionList=3,
        // uFunctionStatus=[1,0,0,0,0]` with the cap in effect; index 0 is
        // the health-status byte (1 = charging stops at 80%).
        let enabled_row = (3u32, &[1u8, 0, 0, 0, 0][..]);
        assert_eq!(desired_status_from_rows(&[enabled_row]), Some(1));
        let disabled_row = (3u32, &[0u8, 0, 0, 0, 0][..]);
        assert_eq!(desired_status_from_rows(&[disabled_row]), Some(0));
        // A [0,1,0,0,0] row decodes as off: bit 1 does not gate charging on
        // this SKU (confirmed on-device).
        let mask2_row = (3u32, &[0u8, 1, 0, 0, 0][..]);
        assert_eq!(desired_status_from_rows(&[mask2_row]), Some(0));
    }

    #[test]
    fn falls_back_to_any_zero_or_one_status_byte() {
        let rows = [(0u32, &[255u8, 255u8][..]), (1u32, &[255u8, 1u8][..])];
        assert_eq!(desired_status_from_rows(&rows), Some(1));
        let rows = [(0u32, &[1u8, 255u8][..])];
        assert_eq!(desired_status_from_rows(&rows), Some(1));
    }

    #[test]
    fn ignores_na_status_byte_on_preferred_row() {
        let rows = [
            (2u32, &[255u8, 255u8][..]),
            (0u32, &[255u8, 0u8][..]),
        ];
        assert_eq!(desired_status_from_rows(&rows), Some(0));
    }

    #[test]
    fn last_resort_is_max_non_255_status_byte() {
        let rows = [
            (1u32, &[255u8, 255u8][..]),
            (4u32, &[254u8, 255u8][..]),
        ];
        assert_eq!(desired_status_from_rows(&rows), Some(254));
        let rows: [(u32, &[u8]); 0] = [];
        assert_eq!(desired_status_from_rows(&rows), None);
        let rows = [(0u32, &[255u8, 255u8][..])];
        assert_eq!(desired_status_from_rows(&rows), None);
    }

    #[test]
    fn method_succeeded_matches_return_value_semantics() {
        assert!(method_succeeded(0, Some(1)));
        assert!(method_succeeded(0, Some(0x0B)));
        assert!(!method_succeeded(0, Some(0)));
        assert!(!method_succeeded(0, Some(0x8004_1002u32)));
        assert!(!method_succeeded(0x8004_1002u32 as i32, Some(1)));
        assert!(!method_succeeded(0, None));
    }

    #[test]
    fn set_enabled_write_verify_protocol_pins_wire_shape() {
        let fake = FakeTransport::new();
        script_charge_set(&fake, Some(1));
        script_charge_get(&fake, 3, &[1, 0, 0, 0, 0]);
        let adapter = SmartChargeAdapter::with_transport(fake.clone());
        adapter.set_enabled(true).unwrap();
        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].method, METHOD_SET);
        // Write tuple: battery 1, mask 1, status 1, 5-zero reserved — in order.
        assert_eq!(calls[0].input.elements, vec![
            MiElement { name: "uBatteryNo", value: MiValue::U8(1) },
            MiElement { name: "uFunctionMask", value: MiValue::U8(1) },
            MiElement { name: "uFunctionStatus", value: MiValue::U8(1) },
            MiElement { name: "uReservedIn", value: MiValue::U8Array(vec![0; 5]) },
        ]);
        // Then the readback pair: battery 1, query 1, 2-zero reserved.
        assert_eq!(calls[1].method, METHOD_GET);
        assert_eq!(calls[1].input.elements, vec![
            MiElement { name: "uBatteryNo", value: MiValue::U8(1) },
            MiElement { name: "uFunctionQuery", value: MiValue::U8(1) },
            MiElement { name: "uReserved", value: MiValue::U8Array(vec![0; 2]) },
        ]);
    }

    #[test]
    fn lying_write_is_rejected_when_readback_does_not_match() {
        let fake = FakeTransport::new();
        script_charge_set(&fake, Some(1));
        // Truthy ReturnValue but the readback row still reports the cap off.
        script_charge_get(&fake, 3, &[0, 0, 0, 0, 0]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert!(matches!(adapter.set_enabled(true), Err(AdapterError::Unexpected(_))));
    }

    #[test]
    fn missing_return_value_rejects_the_write() {
        let fake = FakeTransport::new();
        script_charge_set(&fake, None);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert!(matches!(adapter.set_enabled(true), Err(AdapterError::Unexpected(_))));
    }

    #[test]
    fn rejected_write_with_error_return_value_is_unexpected() {
        let fake = FakeTransport::new();
        script_charge_set(&fake, Some(0x8004_1002u32));
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert!(matches!(adapter.set_enabled(true), Err(AdapterError::Unexpected(_))));
    }

    #[test]
    fn is_enabled_decodes_the_single_pair_readback() {
        let enabled = FakeTransport::new();
        script_charge_get(&enabled, 3, &[1, 0, 0, 0, 0]);
        let adapter = SmartChargeAdapter::with_transport(enabled);
        assert!(adapter.is_enabled().unwrap());

        let disabled = FakeTransport::new();
        script_charge_get(&disabled, 3, &[0, 0, 0, 0, 0]);
        let adapter = SmartChargeAdapter::with_transport(disabled);
        assert!(!adapter.is_enabled().unwrap());
    }

    #[test]
    fn read_row_returns_the_scripted_row_and_asserts_wire_input() {
        let fake = FakeTransport::new();
        script_charge_get(&fake, 3, &[1, 0, 0, 0, 0]);
        let adapter = SmartChargeAdapter::with_transport(fake.clone());
        let row = adapter.read_row(2, 3).unwrap().unwrap();
        assert_eq!(row.function_list, 3);
        assert_eq!(row.status, vec![1, 0, 0, 0, 0]);
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, METHOD_GET);
        assert_eq!(calls[0].input.elements, vec![
            MiElement { name: "uBatteryNo", value: MiValue::U8(2) },
            MiElement { name: "uFunctionQuery", value: MiValue::U8(3) },
            MiElement { name: "uReserved", value: MiValue::U8Array(vec![0; 2]) },
        ]);
    }

    #[test]
    fn read_row_carries_the_method_u_return_when_present() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, [
            some_output(
                MiOutput::new()
                    .with_u32("uFunctionList", 3)
                    .with_u8_array("uFunctionStatus", vec![1, 0, 0, 0, 0])
                    .with_u32("uReturn", 0),
            ),
        ]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        let row = adapter.read_row(1, 1).unwrap().unwrap();
        assert_eq!(row.return_value, Some(0));
    }

    #[test]
    fn read_row_is_none_for_a_row_without_list_or_status() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, [some_output(MiOutput::new())]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert_eq!(adapter.read_row(1, 1).unwrap(), None);
    }

    #[test]
    fn read_row_propagates_transport_errors() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert!(matches!(adapter.read_row(1, 1), Err(AdapterError::Com { .. })));
    }

    #[test]
    fn write_tuple_sends_the_typed_tuple_and_returns_return_value() {
        let fake = FakeTransport::new();
        script_charge_set(&fake, Some(0x0B));
        let adapter = SmartChargeAdapter::with_transport(fake.clone());
        assert_eq!(adapter.write_tuple(1, 2, 1).unwrap(), Some(0x0B));
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, METHOD_SET);
        assert_eq!(calls[0].input.elements, vec![
            MiElement { name: "uBatteryNo", value: MiValue::U8(1) },
            MiElement { name: "uFunctionMask", value: MiValue::U8(2) },
            MiElement { name: "uFunctionStatus", value: MiValue::U8(1) },
            MiElement { name: "uReservedIn", value: MiValue::U8Array(vec![0; 5]) },
        ]);
    }

    #[test]
    fn write_tuple_falls_back_to_u_return_when_return_value_is_absent() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_SET, [
            some_output(MiOutput::new().with_u32("uReturn", 7)),
        ]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert_eq!(adapter.write_tuple(1, 1, 1).unwrap(), Some(7));
    }

    #[test]
    fn write_tuple_propagates_transport_errors() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_SET, [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let adapter = SmartChargeAdapter::with_transport(fake);
        assert!(matches!(adapter.write_tuple(1, 1, 1), Err(AdapterError::Com { .. })));
    }

    #[test]
    fn charge_breaker_trips_after_five_consecutive_failures() {
        let fake = FakeTransport::new();
        fake.script(WMI_NAMESPACE, CLASS_NAME, METHOD_GET, [
            transport_error(crate::mi::MI_RESULT_FAILED),
        ]);
        let adapter = SmartChargeAdapter::with_transport(fake.clone());
        for _ in 0..5 {
            assert!(adapter.is_enabled().is_err());
        }
        assert!(!adapter.is_available());
        assert!(adapter.is_enabled().is_err());
        assert_eq!(fake.count(METHOD_GET), 5);
    }
}
