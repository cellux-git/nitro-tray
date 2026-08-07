//! Smart-charge adapter: in-process control of the 80% charge cap via the
//! `BatteryControl` WMI health-status toggle, using the AMD direct-trust
//! write path for the target SKU class (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No interpreter is spawned.
//! In-process MI (`mi.dll`) via the shared `mi` module, bound to the
//! provider-enumerated instance (ticket 16).
//!
//! No sweeping, ever: the readback is a single direct-trust pair (battery 1,
//! function query 1), never the 35-call `uBatteryNo` x `uFunctionQuery`
//! sweep. A full-sweep discovery mode is deliberately absent; if a future
//! machine needs it, that is a config/decision point, not a default.

use std::cell::Cell;
use std::time::Duration;

use crate::log;
use crate::mi::{MiConnection, MiError, MI_RESULT_ACCESS_DENIED, MI_RESULT_INVALID_CLASS, MI_RESULT_NOT_FOUND};

/// Consecutive failures after which the adapter disables itself (a flapping
/// provider must stop all further calls; the recovery loop reconnects the
/// adapter, never terminal).
const MAX_ADAPTER_FAILURES: u32 = 5;

/// Single-pair readback tuple, verified live via the MI stack on the
/// AN16S-61 (2026-08-07): battery 1 answers every `uFunctionQuery` with
/// `uFunctionList=3` and `uFunctionStatus=[0,1,0,0,0]` (health status at
/// index 1 — `0` healthy/`1` cap in effect); battery 0 returns an empty row
/// (`uReturn=1`). Query 1 targets the function bit preferred by prior-art
/// `Find-DesiredStatus` (`uFunctionList & 2`, `uFunctionStatus[1]`).
const READBACK_BATTERY: u8 = 1;
const READBACK_QUERY: u8 = 1;

/// Errors from the smart-charge WMI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChargeError {
    /// An MI/WMI call failed (hr carries the `MI_RESULT` code).
    Com { hr: i32, op: &'static str },
    /// The BatteryControl interface was not available.
    NotAvailable,
    /// Readback returned an unexpected shape.
    Unexpected(String),
}

/// WMI namespace, class and methods for battery health control.
const WMI_NAMESPACE: &str = "ROOT\\WMI";
const CLASS_NAME: &str = "BatteryControl";
const METHOD_SET: &str = "SetBatteryHealthControl";
const METHOD_GET: &str = "GetBatteryHealthControlStatus";

/// Delay between fallback write attempts (prior art: 250 ms).
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Encodes the AMD direct-trust write tuple for the target SKU class
/// (`uBatteryNo=1, uFunctionMask=1, uFunctionStatus=status`, 5-zero reserved).
/// Prior-art §3.2; attempted first, reported immediately on success.
pub fn direct_trust_tuple(status: u8) -> (u8, u8, u8, [u8; 5]) {
    (1, 1, status, [0, 0, 0, 0, 0])
}

/// Ordered fallback write tuples (prior-art §3.3, battery 1 before battery 0,
/// masks 2 then 3; the mask-1 legacy tuples are subsumed by the direct path).
/// The first whose `ExecMethod` reports a truthy `ReturnValue` wins.
pub fn fallback_tuples(status: u8) -> Vec<(u8, u8, u8, [u8; 5])> {
    vec![
        (1, 2, status, [0, 0, 0, 0, 0]),
        (1, 3, status, [0, 0, 0, 0, 0]),
        (0, 1, status, [0, 0, 0, 0, 0]),
        (0, 2, status, [0, 0, 0, 0, 0]),
        (0, 3, status, [0, 0, 0, 0, 0]),
    ]
}

/// Decodes the health-status byte from readback rows. Prefers the
/// first row where `uFunctionList & 2 != 0` and `uFunctionStatus[1]` is a real
/// status; falls back to any status byte of 0/1; last resort is the max
/// non-255 status byte (prior-art §3.4). The single-pair readback feeds
/// exactly one row here.
pub fn desired_status_from_rows(rows: &[(u32, &[u8])]) -> Option<u8> {
    for (list, statuses) in rows {
        if list & 2 != 0 && statuses.len() >= 2 && statuses[1] != 0xFF {
            return Some(statuses[1]);
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
/// not an error code (prior art gates on `if ($setAnv.ReturnValue)`; a
/// missing `ReturnValue` means the attempt must be treated as failed).
pub fn method_succeeded(hr: i32, return_value: Option<u32>) -> bool {
    hr == 0 && return_value.is_some_and(|rv| rv != 0 && rv < 0x8000_0000)
}

pub struct SmartChargeAdapter {
    connection: MiConnection,
    /// Consecutive failed calls; disables the adapter at `MAX_ADAPTER_FAILURES`.
    failures: Cell<u32>,
    /// When set, every call short-circuits to `NotAvailable`.
    dead: Cell<bool>,
}

// MI is thread-safe, and the markers relax thread-safety claims for the
// single-threaded core that holds the adapter (COM no longer involved).
unsafe impl Send for SmartChargeAdapter {}
unsafe impl Sync for SmartChargeAdapter {}

impl SmartChargeAdapter {
    /// Connect to the `BatteryControl` WMI class in-process via MI
    /// (`mi.dll`): initializes the MI client and a local session. Session
    /// creation does not talk to the provider, so reachability is proven by
    /// the first operation; failures trip the circuit breaker and the
    /// recovery loop reconnects.
    pub fn connect() -> Result<Self, ChargeError> {
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

    /// Runs `call` guarded by the failure-streak circuit breaker; the adapter
    /// disables itself after `MAX_ADAPTER_FAILURES` consecutive errors.
    fn guarded<T>(
        &self,
        call: impl FnOnce() -> Result<T, ChargeError>,
    ) -> Result<T, ChargeError> {
        if self.dead.get() {
            return Err(ChargeError::NotAvailable);
        }
        let result = call();
        match &result {
            Ok(_) => self.failures.set(0),
            Err(_) => {
                let count = self.failures.get() + 1;
                self.failures.set(count);
                if count >= MAX_ADAPTER_FAILURES {
                    self.dead.set(true);
                    log::warn("charge: adapter disabled after repeated failures");
                }
            }
        }
        result
    }

    /// Toggle the 80% charge cap via the AMD direct-trust write path
    /// (`SetBatteryHealthControl` with the proven tuple).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ChargeError> {
        self.guarded(|| {
            let status = u8::from(enabled);
            let mut attempts: Vec<(u8, u8, u8, [u8; 5])> = vec![direct_trust_tuple(status)];
            attempts.extend(fallback_tuples(status));
            let mut last_rejected: Option<u32> = None;
            let mut last_error: Option<ChargeError> = None;
            for (battery, mask, status_byte, _reserved) in attempts {
                match self.exec_set(battery, mask, status_byte) {
                    Ok(Some(return_value)) if method_succeeded(0, Some(return_value)) => {
                        // Prior-art §3.4: a truthy ReturnValue alone is not
                        // proof of effect. On the AN16S-61 the direct-trust
                        // tuple (1,1,status) is accepted with ReturnValue=1
                        // yet clears the health bit (verified live,
                        // 2026-08-08) — the readback match is what decides.
                        std::thread::sleep(ATTEMPT_DELAY);
                        match self.verify_status(READBACK_BATTERY, READBACK_QUERY, status) {
                            Ok(true) => return Ok(()),
                            Ok(false) => {
                                last_rejected = Some(return_value);
                            }
                            Err(e) => last_error = Some(e),
                        }
                    }
                    Ok(Some(return_value)) => last_rejected = Some(return_value),
                    Ok(None) => last_rejected = Some(0),
                    Err(e) => last_error = Some(e),
                }
                std::thread::sleep(ATTEMPT_DELAY);
            }
            match last_error {
                Some(e) => Err(e),
                None => Err(ChargeError::Unexpected(format!(
                    "provider rejected every SetBatteryHealthControl tuple (last ReturnValue={})",
                    last_rejected.unwrap_or(0)
                ))),
            }
        })
    }

    /// Read back the current smart-charge state
    /// (`GetBatteryHealthControlStatus`): one direct-trust pair (battery 1,
    /// query 1), decoded with the prior-art preference (function-list bit 1,
    /// status byte index 1). No sweeping; a rejected or empty row degrades to
    /// an error (the caller shows `None`).
    pub fn is_enabled(&self) -> Result<bool, ChargeError> {
        self.guarded(|| {
            let status = self.read_status(READBACK_BATTERY, READBACK_QUERY)?;
            status.ok_or_else(|| {
                ChargeError::Unexpected(
                    "no GetBatteryHealthControlStatus row for the direct-trust pair".into(),
                )
            })
        })
    }

    /// Single-pair readback of the health-status byte (`Ok(None)` when the
    /// provider rejects the (battery, query) pair or the row carries no
    /// decodable status).
    fn read_status(&self, battery: u8, query: u8) -> Result<Option<bool>, ChargeError> {
        let Some(row) = self.exec_get(battery, query)? else { return Ok(None) };
        let refs = [(row.0, row.1.as_slice())];
        match desired_status_from_rows(&refs) {
            Some(status) => Ok(Some(status == 1)),
            None => Ok(None),
        }
    }

    /// Prior-art §3.4 write verification: the health-status byte read back
    /// for the direct-trust pair must equal the requested status.
    fn verify_status(&self, battery: u8, query: u8, requested: u8) -> Result<bool, ChargeError> {
        match self.read_status(battery, query)? {
            Some(actual) => Ok(actual == (requested == 1)),
            None => Ok(false),
        }
    }

    /// Executes `SetBatteryHealthControl` with one tuple; returns the
    /// provider `ReturnValue` when present (falls back to the declared
    /// `uReturn` out parameter when MI does not synthesize a ReturnValue).
    fn exec_set(&self, battery: u8, mask: u8, status: u8) -> Result<Option<u32>, ChargeError> {
        let instance = self.enumerate_instance()?;
        let mut input = self.connection.new_instance(CLASS_NAME).map_err(map_mi)?;
        input.add_u8("uBatteryNo", battery).map_err(map_mi)?;
        input.add_u8("uFunctionMask", mask).map_err(map_mi)?;
        input.add_u8("uFunctionStatus", status).map_err(map_mi)?;
        input.add_u8_array("uReservedIn", &[0; 5]).map_err(map_mi)?;
        let out = self
            .connection
            .invoke(WMI_NAMESPACE, &instance, METHOD_SET, &input)
            .map_err(map_mi)?;
        let Some(result) = out else { return Ok(None) };
        if let Some(value) = result.get_u32("ReturnValue").map_err(map_mi)? {
            return Ok(Some(value));
        }
        result.get_u32("uReturn").map_err(map_mi)
    }

    /// Executes one `GetBatteryHealthControlStatus` query row. Returns
    /// `Ok(None)` when the provider rejects the (battery, query) pair.
    fn exec_get(&self, battery: u8, query: u8) -> Result<Option<(u32, Vec<u8>)>, ChargeError> {
        let instance = self.enumerate_instance()?;
        let mut input = self.connection.new_instance(CLASS_NAME).map_err(map_mi)?;
        input.add_u8("uBatteryNo", battery).map_err(map_mi)?;
        input.add_u8("uFunctionQuery", query).map_err(map_mi)?;
        input.add_u8_array("uReserved", &[0; 2]).map_err(map_mi)?;
        let out = match self.connection.invoke(WMI_NAMESPACE, &instance, METHOD_GET, &input) {
            Ok(out) => out,
            Err(_) => return Ok(None), // rejected pair: skip the row
        };
        let Some(result) = out else { return Ok(None) };
        let Some(list) = result.get_u32("uFunctionList").map_err(map_mi)? else {
            return Ok(None);
        };
        let Some(statuses) = result.get_u8_array("uFunctionStatus").map_err(map_mi)? else {
            return Ok(None);
        };
        if statuses.is_empty() {
            return Ok(None);
        }
        Ok(Some((list, statuses)))
    }

    /// The provider's first `BatteryControl` instance (the instance-bound
    /// binding target, PowerShell `-InputObject` shape).
    fn enumerate_instance(&self) -> Result<crate::mi::MiInstance, ChargeError> {
        self.connection.enumerate_first_instance(WMI_NAMESPACE, CLASS_NAME).map_err(map_mi)
    }
}

/// Maps an `MiError` to `ChargeError`: interface-unavailable codes become
/// `NotAvailable`, everything else is `Com`.
fn map_mi(err: MiError) -> ChargeError {
    match err.result {
        MI_RESULT_INVALID_CLASS | MI_RESULT_NOT_FOUND | MI_RESULT_ACCESS_DENIED => ChargeError::NotAvailable,
        _ => ChargeError::Com { hr: err.result, op: err.op },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_trust_tuple_encodes_prior_art_anv16_41_path() {
        assert_eq!(direct_trust_tuple(1), (1, 1, 1, [0, 0, 0, 0, 0]));
        assert_eq!(direct_trust_tuple(0), (1, 1, 0, [0, 0, 0, 0, 0]));
    }

    #[test]
    fn fallback_tuples_are_ordered_and_scalar() {
        let expected: Vec<(u8, u8, u8, [u8; 5])> = vec![
            (1, 2, 1, [0, 0, 0, 0, 0]),
            (1, 3, 1, [0, 0, 0, 0, 0]),
            (0, 1, 1, [0, 0, 0, 0, 0]),
            (0, 2, 1, [0, 0, 0, 0, 0]),
            (0, 3, 1, [0, 0, 0, 0, 0]),
        ];
        assert_eq!(fallback_tuples(1), expected);
        for (_, _, status, reserved) in fallback_tuples(0) {
            assert_eq!(status, 0);
            assert_eq!(reserved, [0, 0, 0, 0, 0]);
        }
    }

    #[test]
    fn prefers_function_list_bit1_row_status_byte() {
        let rows = [
            (0u32, &[255u8, 0u8][..]),
            (3u32, &[255u8, 1u8][..]),
        ];
        assert_eq!(desired_status_from_rows(&rows), Some(1));
    }

    #[test]
    fn decodes_the_live_an16s_61_readback_row() {
        // Verified via MI on the AN16S-61 (2026-08-07): battery 1 answers
        // `uFunctionList=3, uFunctionStatus=[0,1,0,0,0]`; index 1 is the
        // health-status byte (1 = cap in effect).
        let enabled_row = (3u32, &[0u8, 1, 0, 0, 0][..]);
        assert_eq!(desired_status_from_rows(&[enabled_row]), Some(1));
        let disabled_row = (3u32, &[0u8, 0, 0, 0, 0][..]);
        assert_eq!(desired_status_from_rows(&[disabled_row]), Some(0));
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
    fn method_succeeded_matches_prior_art_return_value_semantics() {
        assert!(method_succeeded(0, Some(1)));
        assert!(method_succeeded(0, Some(0x0B)));
        assert!(!method_succeeded(0, Some(0)));
        assert!(!method_succeeded(0, Some(0x8004_1002u32)));
        assert!(!method_succeeded(0x8004_1002u32 as i32, Some(1)));
        assert!(!method_succeeded(0, None));
    }

    #[test]
    fn mi_interface_unavailable_codes_map_to_not_available() {
        let err = MiError { result: MI_RESULT_INVALID_CLASS, op: "t", message: None };
        assert_eq!(map_mi(err), ChargeError::NotAvailable);
        let err = MiError { result: MI_RESULT_ACCESS_DENIED, op: "t", message: None };
        assert_eq!(map_mi(err), ChargeError::NotAvailable);
        let err = MiError { result: crate::mi::MI_RESULT_TYPE_MISMATCH, op: "t", message: None };
        assert_eq!(map_mi(err), ChargeError::Com { hr: 13, op: "t" });
    }
}
