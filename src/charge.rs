//! Smart-charge adapter: in-process control of the 80% charge cap via the
//! `BatteryControl` WMI health-status toggle, using the AMD direct-trust
//! write path for the target SKU class (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No interpreter is spawned.
//! Raw COM `ExecMethod` via the shared `comwbem` module.

use std::time::Duration;

use windows_sys::Win32::System::Wmi::WBEM_E_NOT_FOUND;

use crate::comwbem::{self, Bstr, ClassObject, ComApartment, ComRef, Variant, CIM_UINT8};

/// Errors from the smart-charge WMI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChargeError {
    /// A COM/WMI call failed.
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

/// Decodes the health-status byte from a sweep of readback rows. Prefers the
/// first row where `uFunctionList & 2 != 0` and `uFunctionStatus[1]` is a real
/// status; falls back to any status byte of 0/1; last resort is the max
/// non-255 status byte (prior-art §3.4).
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

/// A `SetBatteryHealthControl` attempt succeeds only when `ExecMethod`
/// returns S_OK AND the provider's `ReturnValue` is present, truthy, and not
/// an error code (prior art gates on `if ($setAnv.ReturnValue)`; a missing
/// `ReturnValue` means the attempt must be treated as failed).
pub fn method_succeeded(hr: i32, return_value: Option<u32>) -> bool {
    hr == 0 && return_value.is_some_and(|rv| rv != 0 && rv < 0x8000_0000)
}

pub struct SmartChargeAdapter {
    _com: ComApartment,
    services: ComRef,
    class: ClassObject,
}

// COM objects are apartment-bound; the adapter is only ever used from the
// thread that created it (the UI thread), and the markers only relax
// thread-safety claims for the single-threaded core that holds it.
unsafe impl Send for SmartChargeAdapter {}
unsafe impl Sync for SmartChargeAdapter {}

impl SmartChargeAdapter {
    /// Connect to the `BatteryControl` WMI class in-process
    /// (`CoInitializeEx` + `CoCreateInstance(CLSID_WBEM_LOCATOR)` +
    /// `ConnectServer(ROOT\WMI)` + `GetObject(BatteryControl)`).
    pub fn connect() -> Result<Self, ChargeError> {
        let _com = ComApartment::init().map_err(|hr| ChargeError::Com { hr, op: "CoInitializeEx" })?;
        let locator = unsafe { comwbem::create_locator() }
            .map_err(|hr| ChargeError::Com { hr, op: "CoCreateInstance(CLSID_WbemLocator)" })?;
        let services = unsafe { comwbem::connect_server(&locator, WMI_NAMESPACE) }
            .map_err(|_| ChargeError::NotAvailable)?;
        let class = unsafe { comwbem::get_class(&services, CLASS_NAME) }
            .map_err(|_| ChargeError::NotAvailable)?;
        Ok(Self { _com, services, class })
    }

    /// Toggle the 80% charge cap via the AMD direct-trust write path
    /// (`SetBatteryHealthControl` with the proven tuple).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ChargeError> {
        let status = u8::from(enabled);
        let mut attempts: Vec<(u8, u8, u8, [u8; 5])> = vec![direct_trust_tuple(status)];
        attempts.extend(fallback_tuples(status));
        let path = unsafe { comwbem::first_instance_path(&self.services, CLASS_NAME) }
            .map_err(|hr| ChargeError::Com { hr, op: "first_instance_path(BatteryControl)" })?;
        let mut last_rejected: Option<u32> = None;
        let mut last_error: Option<ChargeError> = None;
        for (battery, mask, status_byte, _reserved) in attempts {
            match self.exec_set(&path, battery, mask, status_byte) {
                Ok(Some(return_value)) => {
                    if method_succeeded(0, Some(return_value)) {
                        return Ok(());
                    }
                    last_rejected = Some(return_value);
                }
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
    }

    /// Read back the current smart-charge state (`GetBatteryHealthControlStatus`).
    pub fn is_enabled(&self) -> Result<bool, ChargeError> {
        let path = unsafe { comwbem::first_instance_path(&self.services, CLASS_NAME) }
            .map_err(|hr| ChargeError::Com { hr, op: "first_instance_path(BatteryControl)" })?;
        let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
        for battery in 0u8..5 {
            for query in 0u8..7 {
                if let Some(row) = self.exec_get(&path, battery, query)? {
                    rows.push(row);
                }
            }
        }
        let refs: Vec<(u32, &[u8])> = rows
            .iter()
            .map(|(list, statuses)| (*list, statuses.as_slice()))
            .collect();
        match desired_status_from_rows(&refs) {
            Some(status) => Ok(status == 1),
            None => Err(ChargeError::Unexpected(
                "no health status found across GetBatteryHealthControlStatus sweep".into(),
            )),
        }
    }

    /// Executes `SetBatteryHealthControl` with one tuple; returns the
    /// provider `ReturnValue` when present.
    fn exec_set(
        &self,
        path: &Bstr,
        battery: u8,
        mask: u8,
        status: u8,
    ) -> Result<Option<u32>, ChargeError> {
        let in_params = self.spawn_input(
            METHOD_SET,
            &[(b"uBatteryNo", battery), (b"uFunctionMask", mask), (b"uFunctionStatus", status)],
        )?;
        let parray = unsafe { comwbem::uint8_array(5) }
            .ok_or_else(|| ChargeError::Unexpected("SafeArrayCreate(uReservedIn) failed".into()))?;
        let reserved_v = Variant::from_array(parray);
        let hr = unsafe {
            in_params.put(
                comwbem::wide("uReservedIn").as_ptr(),
                &reserved_v,
                CIM_UINT8,
            )
        };
        if hr != 0 {
            return Err(ChargeError::Com { hr, op: "Put(uReservedIn)" });
        }
        let method = Bstr::new(METHOD_SET)
            .ok_or_else(|| ChargeError::Unexpected("SysAllocString(method) failed".into()))?;
        let out_params = unsafe { comwbem::exec_method(&self.services, path, &method, in_params.raw()) }
            .map_err(|hr| ChargeError::Com { hr, op: "ExecMethod(SetBatteryHealthControl)" })?;
        match out_params {
            None => Ok(None),
            Some(out) => read_variant_u32(&out, "ReturnValue"),
        }
    }

    /// Executes one `GetBatteryHealthControlStatus` query row. Returns
    /// `Ok(None)` when the provider rejects the (battery, query) pair.
    fn exec_get(&self, path: &Bstr, battery: u8, query: u8) -> Result<Option<(u32, Vec<u8>)>, ChargeError> {
        let in_params = self.spawn_input(METHOD_GET, &[(b"uBatteryNo", battery), (b"uFunctionQuery", query)])?;
        let parray = unsafe { comwbem::uint8_array(2) }
            .ok_or_else(|| ChargeError::Unexpected("SafeArrayCreate(uReserved) failed".into()))?;
        let reserved_v = Variant::from_array(parray);
        let hr = unsafe { in_params.put(comwbem::wide("uReserved").as_ptr(), &reserved_v, CIM_UINT8) };
        if hr != 0 {
            return Err(ChargeError::Com { hr, op: "Put(uReserved)" });
        }
        let method = Bstr::new(METHOD_GET)
            .ok_or_else(|| ChargeError::Unexpected("SysAllocString(method) failed".into()))?;
        let out_params = match unsafe { comwbem::exec_method(&self.services, path, &method, in_params.raw()) } {
            Ok(out) => out,
            Err(_) => return Ok(None), // rejected pair: skip the row
        };
        let Some(out) = out_params else {
            return Ok(None);
        };
        let list = match read_variant_u32(&out, "uFunctionList")? {
            Some(list) => list,
            None => return Ok(None),
        };
        let statuses = read_status_array(&out)?;
        if statuses.is_empty() {
            return Ok(None);
        }
        Ok(Some((list, statuses)))
    }

    /// Spawns the input instance for `method` and puts the scalar `uint8`
    /// parameters it is given (GetMethod -> SpawnInstance -> Put).
    fn spawn_input(&self, method: &str, scalars: &[(&[u8], u8)]) -> Result<ClassObject, ChargeError> {
        let method_wide = comwbem::wide(method);
        let mut in_class: *mut core::ffi::c_void = core::ptr::null_mut();
        let mut out_class: *mut core::ffi::c_void = core::ptr::null_mut();
        let hr = unsafe { self.class.get_method(method_wide.as_ptr(), &mut in_class, &mut out_class) };
        if hr != 0 {
            return Err(ChargeError::Com { hr, op: "GetMethod" });
        }
        drop(ComRef::from_raw(out_class));
        if in_class.is_null() {
            return Err(ChargeError::Unexpected(format!("{method}: no input signature")));
        }
        let in_class = unsafe { ClassObject::from_raw(in_class) };
        let mut in_params: *mut core::ffi::c_void = core::ptr::null_mut();
        let hr = unsafe { in_class.spawn_instance(&mut in_params) };
        if hr != 0 {
            return Err(ChargeError::Com { hr, op: "SpawnInstance" });
        }
        if in_params.is_null() {
            return Err(ChargeError::Unexpected(format!("{method}: null input instance")));
        }
        let in_params = unsafe { ClassObject::from_raw(in_params) };
        for (name, value) in scalars {
            let name = core::str::from_utf8(name).unwrap_or("");
            let v = Variant::ui1(*value);
            let hr = unsafe { in_params.put(comwbem::wide(name).as_ptr(), &v, CIM_UINT8) };
            if hr != 0 {
                return Err(ChargeError::Com { hr, op: "Put(scalar)" });
            }
        }
        Ok(in_params)
    }
}

/// Reads a scalar u32 out-parameter (`uFunctionList`, `ReturnValue`);
/// `Ok(None)` when the property is absent.
fn read_variant_u32(object: &ClassObject, name: &str) -> Result<Option<u32>, ChargeError> {
    let value = match unsafe { object.get(comwbem::wide(name).as_ptr()) } {
        Ok(value) => value,
        Err(hr) if hr == WBEM_E_NOT_FOUND => return Ok(None),
        Err(hr) => return Err(ChargeError::Com { hr, op: "IWbemClassObject::Get" }),
    };
    if value.is_null_or_empty() {
        return Ok(None);
    }
    match value.as_u64() {
        Some(v) => Ok(Some(v as u32)),
        None => Err(ChargeError::Unexpected(format!("unexpected variant for {name}"))),
    }
}

/// Reads the `uFunctionStatus` byte-array out-parameter; `Ok(Vec::new())`
/// when the property is absent.
fn read_status_array(object: &ClassObject) -> Result<Vec<u8>, ChargeError> {
    let value = match unsafe { object.get(comwbem::wide("uFunctionStatus").as_ptr()) } {
        Ok(value) => value,
        Err(hr) if hr == WBEM_E_NOT_FOUND => return Ok(Vec::new()),
        Err(hr) => return Err(ChargeError::Com { hr, op: "IWbemClassObject::Get(uFunctionStatus)" }),
    };
    Ok(value.as_u8_array().unwrap_or_default())
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
}
