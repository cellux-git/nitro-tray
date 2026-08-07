//! Smart-charge adapter: in-process control of the 80% charge cap via the
//! `BatteryControl` WMI health-status toggle, using the AMD direct-trust
//! write path for the target SKU class (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No interpreter is spawned.
//!
//! Raw COM `ExecMethod` against `ROOT\WMI` / `BatteryControl`, identical in
//! style to `src/wmi.rs`. windows-sys 0.61 ships the WMI constants and the
//! `WbemLocator` CLSID but no `IWbem*` interface bindings and no `VARIANT`
//! (that feature is not enabled), so the vtables, the variant and
//! `SafeArrayCreate` are declared here with their canonical layouts.

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::core::{BSTR, GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{SysAllocString, SysFreeString};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, SAFEARRAY, SAFEARRAYBOUND, CLSCTX_INPROC_SERVER,
};
use windows_sys::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayUnaccessData};
use windows_sys::Win32::System::Wmi::{CIM_UINT8, CIMTYPE_ENUMERATION, WbemLocator};

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

const IID_WBEM_LOCATOR: GUID = GUID::from_u128(0xdc12a687_737f_11cf_884d_00aa004b2e24);

const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
const WBEM_E_NOT_FOUND: i32 = -2147217406;

const VT_EMPTY: u16 = 0;
const VT_NULL: u16 = 1;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_UI1: u16 = 17;
const VT_UI2: u16 = 18;
const VT_UI4: u16 = 19;
const VT_I8: u16 = 20;
const VT_UI8: u16 = 21;
const VT_ARRAY: u16 = 0x2000;

#[link(name = "oleaut32")]
unsafe extern "system" {
    fn SafeArrayCreate(vt: u16, cdims: u32, rgsabound: *const SAFEARRAYBOUND) -> *mut SAFEARRAY;
}

/// Minimal C-layout `VARIANT` (16 bytes on x64) covering the members this
/// module reads from and writes to WMI in/out parameters.
#[repr(C)]
struct Variant {
    vt: u16,
    _w_reserved1: u16,
    _w_reserved2: u16,
    _w_reserved3: u16,
    data: VariantData,
}

#[repr(C)]
union VariantData {
    ll_val: i64,
    l_val: i32,
    b_val: u8,
    ui2_val: u16,
    ul_val: u32,
    ull_val: u64,
    bstr_val: BSTR,
    parray: *mut SAFEARRAY,
}

impl Variant {
    fn clear(&mut self) {
        unsafe {
            match self.vt {
                VT_BSTR => SysFreeString(self.data.bstr_val),
                VT_ARRAY | VT_UI1 => {
                    SafeArrayDestroy(self.data.parray);
                }
                _ => {}
            }
            *self = std::mem::zeroed();
        }
    }
}

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IWbemLocatorVtbl {
    parent: IUnknownVtbl,
    connect_server: unsafe extern "system" fn(
        this: *mut c_void,
        network_resource: BSTR,
        user: BSTR,
        password: BSTR,
        locale: BSTR,
        security_flags: i32,
        authority: BSTR,
        ctx: *mut c_void,
        services: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct IWbemServicesVtbl {
    parent: IUnknownVtbl,
    open_namespace: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    cancel_async_call: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    query_object_sink: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_object: unsafe extern "system" fn(
        this: *mut c_void,
        object_path: BSTR,
        flags: i32,
        ctx: *mut c_void,
        object: *mut *mut c_void,
        call_result: *mut *mut c_void,
    ) -> HRESULT,
    get_object_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    put_class: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    put_class_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    delete_class: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    delete_class_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    create_class_enum: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    create_class_enum_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    put_instance: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    put_instance_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    delete_instance: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    delete_instance_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    create_instance_enum: unsafe extern "system" fn(
        this: *mut c_void,
        class: BSTR,
        flags: i32,
        ctx: *mut c_void,
        enumerator: *mut *mut c_void,
    ) -> HRESULT,
    create_instance_enum_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    exec_query: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    exec_query_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    exec_notification_query: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    exec_notification_query_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    exec_method: unsafe extern "system" fn(
        this: *mut c_void,
        object_path: BSTR,
        method_name: BSTR,
        flags: i32,
        ctx: *mut c_void,
        in_params: *mut c_void,
        out_params: *mut *mut c_void,
        call_result: *mut *mut c_void,
    ) -> HRESULT,
    exec_method_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[repr(C)]
struct IWbemClassObjectVtbl {
    parent: IUnknownVtbl,
    get_qualifier_set: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        value: *mut Variant,
        cim_type: *mut i32,
        flavor: *mut i32,
    ) -> HRESULT,
    put: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        value: *const Variant,
        cim_type: CIMTYPE_ENUMERATION,
    ) -> HRESULT,
    delete: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_names: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    begin_enumeration: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    next: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    end_enumeration: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_property_qualifier_set: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clone: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_object_text: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    spawn_derived_class: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    spawn_instance: unsafe extern "system" fn(
        this: *mut c_void,
        flags: i32,
        instance: *mut *mut c_void,
    ) -> HRESULT,
    compare_to: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_property_origin: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    inherits_from: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_method: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        in_signature: *mut *mut c_void,
        out_signature: *mut *mut c_void,
    ) -> HRESULT,
    put_method: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    delete_method: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_method_origin: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_method_qualifier_set: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[repr(C)]
struct IEnumWbemClassObjectVtbl {
    parent: IUnknownVtbl,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    next: unsafe extern "system" fn(
        this: *mut c_void,
        timeout: i32,
        count: u32,
        objects: *mut *mut c_void,
        returned: *mut u32,
    ) -> HRESULT,
    next_async: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clone: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    skip: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

unsafe fn vtbl<T>(com: *mut c_void) -> *const T {
    unsafe { *(com as *const *const T) }
}

fn release(com: *mut c_void) {
    if !com.is_null() {
        let v = unsafe { vtbl::<IUnknownVtbl>(com) };
        unsafe { ((*v).release)(com) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn bstr(s: &str) -> BSTR {
    let w = wide(s);
    unsafe { SysAllocString(w.as_ptr()) }
}

fn connect_server(locator: *mut c_void, namespace: BSTR) -> Result<*mut c_void, HRESULT> {
    unsafe {
        let v = vtbl::<IWbemLocatorVtbl>(locator);
        let mut services: *mut c_void = null_mut();
        let hr = ((*v).connect_server)(
            locator,
            namespace,
            null(),
            null(),
            null(),
            0,
            null(),
            null_mut(),
            &mut services,
        );
        if hr != 0 {
            return Err(hr);
        }
        Ok(services)
    }
}

fn get_object(services: *mut c_void, class: BSTR) -> Result<*mut c_void, HRESULT> {
    unsafe {
        let v = vtbl::<IWbemServicesVtbl>(services);
        let mut object: *mut c_void = null_mut();
        let hr = ((*v).get_object)(services, class, 0, null_mut(), &mut object, null_mut());
        if hr != 0 {
            return Err(hr);
        }
        Ok(object)
    }
}

fn create_instance_enum(services: *mut c_void, class: BSTR) -> Result<*mut c_void, HRESULT> {
    unsafe {
        let v = vtbl::<IWbemServicesVtbl>(services);
        let mut enumerator: *mut c_void = null_mut();
        let hr = ((*v).create_instance_enum)(services, class, 0, null_mut(), &mut enumerator);
        if hr != 0 {
            return Err(hr);
        }
        Ok(enumerator)
    }
}

fn enum_next_first(enumerator: *mut c_void) -> Result<*mut c_void, HRESULT> {
    unsafe {
        let v = vtbl::<IEnumWbemClassObjectVtbl>(enumerator);
        let mut object: *mut c_void = null_mut();
        let mut returned: u32 = 0;
        let hr = ((*v).next)(enumerator, 0, 1, &mut object, &mut returned);
        if hr != 0 {
            return Err(hr);
        }
        if returned == 0 {
            return Err(WBEM_E_NOT_FOUND);
        }
        Ok(object)
    }
}

fn class_get(object: *mut c_void, name: PCWSTR, value: *mut Variant) -> HRESULT {
    unsafe {
        let v = vtbl::<IWbemClassObjectVtbl>(object);
        ((*v).get)(object, name, 0, value, null_mut(), null_mut())
    }
}

fn class_put(
    object: *mut c_void,
    name: PCWSTR,
    value: *const Variant,
    cim_type: CIMTYPE_ENUMERATION,
) -> HRESULT {
    unsafe {
        let v = vtbl::<IWbemClassObjectVtbl>(object);
        ((*v).put)(object, name, 0, value, cim_type)
    }
}

fn class_get_method(
    object: *mut c_void,
    name: PCWSTR,
    in_signature: *mut *mut c_void,
    out_signature: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        let v = vtbl::<IWbemClassObjectVtbl>(object);
        ((*v).get_method)(object, name, 0, in_signature, out_signature)
    }
}

fn class_spawn_instance(object: *mut c_void, instance: *mut *mut c_void) -> HRESULT {
    unsafe {
        let v = vtbl::<IWbemClassObjectVtbl>(object);
        ((*v).spawn_instance)(object, 0, instance)
    }
}

fn exec_method(
    services: *mut c_void,
    object_path: BSTR,
    method_name: BSTR,
    in_params: *mut c_void,
    out_params: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        let v = vtbl::<IWbemServicesVtbl>(services);
        ((*v).exec_method)(
            services,
            object_path,
            method_name,
            0,
            null_mut(),
            in_params,
            out_params,
            null_mut(),
        )
    }
}

/// Creates a `uint8[5]` (or `uint8[2]`) SAFEARRAY for a reserved-array in
/// parameter. Caller destroys it via `SafeArrayDestroy`.
fn reserved_array(len: usize) -> *mut SAFEARRAY {
    unsafe {
        let bound = SAFEARRAYBOUND {
            cElements: len as u32,
            lLbound: 0,
        };
        SafeArrayCreate(VT_UI1 | VT_ARRAY, 1, &bound)
    }
}

fn read_u8_array(parray: *mut SAFEARRAY) -> Vec<u8> {
    unsafe {
        let len = (*parray).rgsabound[0].cElements as usize;
        let mut data: *mut c_void = null_mut();
        SafeArrayAccessData(parray, &mut data);
        let bytes = std::slice::from_raw_parts(data as *const u8, len).to_vec();
        SafeArrayUnaccessData(parray);
        bytes
    }
}

fn com_err(hr: i32, op: &'static str) -> ChargeError {
    ChargeError::Com { hr, op }
}

fn unexpected(msg: impl Into<String>) -> ChargeError {
    ChargeError::Unexpected(msg.into())
}

/// Encodes the AMD direct-trust write tuple for the target SKU class
/// (`uBatteryNo=1, uFunctionMask=1, uFunctionStatus=status`, 5-zero reserved).
/// Prior-art §3.2; attempted first, reported immediately on success.
pub fn direct_trust_tuple(status: u8) -> (u8, u8, u8, [u8; 5]) {
    (1, 1, status, [0, 0, 0, 0, 0])
}

/// Ordered fallback write tuples: (battery 0, mask 1), (battery 1, mask 2),
/// (battery 0, mask 2). The first whose `ExecMethod` succeeds wins.
pub fn fallback_tuples(status: u8) -> Vec<(u8, u8, u8, [u8; 5])> {
    vec![
        (0, 1, status, [0, 0, 0, 0, 0]),
        (1, 2, status, [0, 0, 0, 0, 0]),
        (0, 2, status, [0, 0, 0, 0, 0]),
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

/// A `SetBatteryHealthControl` attempt succeeds when `ExecMethod` returns
/// S_OK and the provider's `ReturnValue` is truthy and not an error code
/// (prior art treats a nonzero `ReturnValue` as the success signal).
pub fn method_succeeded(hr: i32, return_value: Option<u32>) -> bool {
    hr == 0 && return_value.is_none_or(|rv| rv != 0 && rv < 0x8000_0000)
}

pub struct SmartChargeAdapter {
    services: *mut c_void,
    class: *mut c_void,
}

unsafe impl Send for SmartChargeAdapter {}
unsafe impl Sync for SmartChargeAdapter {}

impl Drop for SmartChargeAdapter {
    fn drop(&mut self) {
        release(self.class);
        release(self.services);
    }
}

impl SmartChargeAdapter {
    /// Connect to the `BatteryControl` WMI class in-process
    /// (`CoInitializeEx` + `CoCreateInstance(CLSID_WBEM_LOCATOR)` +
    /// `ConnectServer(ROOT\WMI)` + `GetObject(BatteryControl)`).
    pub fn connect() -> Result<Self, ChargeError> {
        unsafe {
            let hr = CoInitializeEx(null_mut(), 0);
            if hr != 0 && hr != RPC_E_CHANGED_MODE {
                return Err(ChargeError::NotAvailable);
            }
            let mut locator: *mut c_void = null_mut();
            let hr = CoCreateInstance(
                &WbemLocator,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_WBEM_LOCATOR,
                &mut locator,
            );
            if hr != 0 {
                return Err(ChargeError::NotAvailable);
            }
            let namespace = bstr(WMI_NAMESPACE);
            let services = match connect_server(locator, namespace) {
                Ok(s) => s,
                Err(_) => {
                    SysFreeString(namespace);
                    release(locator);
                    return Err(ChargeError::NotAvailable);
                }
            };
            SysFreeString(namespace);
            release(locator);
            let class_name = bstr(CLASS_NAME);
            let class = match get_object(services, class_name) {
                Ok(c) => c,
                Err(_) => {
                    SysFreeString(class_name);
                    release(services);
                    return Err(ChargeError::NotAvailable);
                }
            };
            SysFreeString(class_name);
            Ok(SmartChargeAdapter { services, class })
        }
    }

    /// Object path of the first `BatteryControl` instance (prior art binds
    /// the first instance from `Get-CimInstance`).
    fn instance_path(&self) -> Result<BSTR, ChargeError> {
        unsafe {
            let class_name = bstr(CLASS_NAME);
            let enumerator = match create_instance_enum(self.services, class_name) {
                Ok(e) => e,
                Err(hr) => {
                    SysFreeString(class_name);
                    return Err(com_err(hr, "CreateInstanceEnum(BatteryControl)"));
                }
            };
            SysFreeString(class_name);
            let instance = match enum_next_first(enumerator) {
                Ok(o) => o,
                Err(hr) => {
                    release(enumerator);
                    return Err(com_err(hr, "IEnumWbemClassObject::Next"));
                }
            };
            release(enumerator);
            let mut value = std::mem::zeroed::<Variant>();
            let hr = class_get(instance, wide("__PATH").as_ptr(), &mut value);
            release(instance);
            if hr != 0 {
                return Err(com_err(hr, "IWbemClassObject::Get(__PATH)"));
            }
            if value.vt != VT_BSTR {
                return Err(unexpected("BatteryControl instance has no __PATH"));
            }
            let path = SysAllocString(value.data.bstr_val);
            value.clear();
            if path.is_null() {
                return Err(unexpected("SysAllocString(__PATH) failed"));
            }
            Ok(path)
        }
    }

    /// Executes `SetBatteryHealthControl` with one tuple; returns the
    /// provider `ReturnValue` when present.
    fn exec_set(
        &self,
        path: BSTR,
        battery: u8,
        mask: u8,
        status: u8,
        reserved: &[u8; 5],
    ) -> Result<Option<u32>, ChargeError> {
        unsafe {
            let method_name = wide(METHOD_SET);
            let mut in_class: *mut c_void = null_mut();
            let mut out_class: *mut c_void = null_mut();
            let hr = class_get_method(
                self.class,
                method_name.as_ptr(),
                &mut in_class,
                &mut out_class,
            );
            if hr != 0 {
                return Err(com_err(hr, "GetMethod(SetBatteryHealthControl)"));
            }
            let mut in_params: *mut c_void = null_mut();
            let hr = class_spawn_instance(in_class, &mut in_params);
            release(in_class);
            release(out_class);
            if hr != 0 {
                return Err(com_err(hr, "SpawnInstance"));
            }

            let scalar = |value: u8| Variant {
                vt: VT_UI1,
                _w_reserved1: 0,
                _w_reserved2: 0,
                _w_reserved3: 0,
                data: VariantData { b_val: value },
            };
            let battery_v = scalar(battery);
            let hr = class_put(in_params, wide("uBatteryNo").as_ptr(), &battery_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                return Err(com_err(hr, "Put(uBatteryNo)"));
            }
            let mask_v = scalar(mask);
            let hr = class_put(in_params, wide("uFunctionMask").as_ptr(), &mask_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                return Err(com_err(hr, "Put(uFunctionMask)"));
            }
            let status_v = scalar(status);
            let hr = class_put(in_params, wide("uFunctionStatus").as_ptr(), &status_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                return Err(com_err(hr, "Put(uFunctionStatus)"));
            }
            let parray = reserved_array(5);
            if parray.is_null() {
                release(in_params);
                return Err(unexpected("SafeArrayCreate(uReservedIn) failed"));
            }
            let mut data: *mut c_void = null_mut();
            SafeArrayAccessData(parray, &mut data);
            std::ptr::copy_nonoverlapping(reserved.as_ptr(), data as *mut u8, 5);
            SafeArrayUnaccessData(parray);
            let reserved_v = Variant {
                vt: VT_UI1 | VT_ARRAY,
                _w_reserved1: 0,
                _w_reserved2: 0,
                _w_reserved3: 0,
                data: VariantData { parray },
            };
            let hr = class_put(in_params, wide("uReservedIn").as_ptr(), &reserved_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                SafeArrayDestroy(parray);
                return Err(com_err(hr, "Put(uReservedIn)"));
            }

            let method_bstr = bstr(METHOD_SET);
            let mut out_params: *mut c_void = null_mut();
            let hr = exec_method(
                self.services,
                path,
                method_bstr,
                in_params,
                &mut out_params,
            );
            SysFreeString(method_bstr);
            release(in_params);
            SafeArrayDestroy(parray);
            if hr != 0 {
                return Err(com_err(hr, "ExecMethod(SetBatteryHealthControl)"));
            }
            if out_params.is_null() {
                return Ok(None);
            }
            let return_value = self.read_variant_u32(out_params, "ReturnValue")?;
            release(out_params);
            Ok(return_value)
        }
    }

    /// Reads a scalar u32 out-parameter (`uFunctionList`, `ReturnValue`).
    fn read_variant_u32(
        &self,
        object: *mut c_void,
        name: &str,
    ) -> Result<Option<u32>, ChargeError> {
        unsafe {
            let mut value = std::mem::zeroed::<Variant>();
            let hr = class_get(object, wide(name).as_ptr(), &mut value);
            if hr == WBEM_E_NOT_FOUND {
                return Ok(None);
            }
            if hr != 0 {
                return Err(com_err(hr, "IWbemClassObject::Get"));
            }
            let result = match value.vt {
                VT_UI1 => Ok(Some(value.data.b_val as u32)),
                VT_UI2 => Ok(Some(value.data.ui2_val as u32)),
                VT_UI4 => Ok(Some(value.data.ul_val)),
                VT_UI8 => Ok(Some(value.data.ull_val as u32)),
                VT_I4 => Ok(Some(value.data.l_val as u32)),
                VT_I8 => Ok(Some(value.data.ll_val as u32)),
                VT_EMPTY | VT_NULL => Ok(None),
                other => Err(unexpected(format!("unexpected variant type {other} for {name}"))),
            };
            value.clear();
            result
        }
    }

    /// Reads the `uFunctionStatus` byte-array out-parameter.
    fn read_status_array(&self, object: *mut c_void) -> Result<Vec<u8>, ChargeError> {
        unsafe {
            let mut value = std::mem::zeroed::<Variant>();
            let hr = class_get(object, wide("uFunctionStatus").as_ptr(), &mut value);
            if hr == WBEM_E_NOT_FOUND {
                return Ok(Vec::new());
            }
            if hr != 0 {
                return Err(com_err(hr, "IWbemClassObject::Get(uFunctionStatus)"));
            }
            let result = match value.vt {
                VT_UI1 | VT_ARRAY => Ok(if value.data.parray.is_null() {
                    Vec::new()
                } else {
                    read_u8_array(value.data.parray)
                }),
                VT_EMPTY | VT_NULL => Ok(Vec::new()),
                other => Err(unexpected(format!(
                    "unexpected variant type {other} for uFunctionStatus"
                ))),
            };
            value.clear();
            result
        }
    }

    /// Executes one `GetBatteryHealthControlStatus` query row. Returns
    /// `Ok(None)` when the provider rejects the (battery, query) pair.
    fn exec_get(
        &self,
        path: BSTR,
        battery: u8,
        query: u8,
    ) -> Result<Option<(u32, Vec<u8>)>, ChargeError> {
        unsafe {
            let method_name = wide(METHOD_GET);
            let mut in_class: *mut c_void = null_mut();
            let mut out_class: *mut c_void = null_mut();
            let hr = class_get_method(
                self.class,
                method_name.as_ptr(),
                &mut in_class,
                &mut out_class,
            );
            if hr != 0 {
                return Err(com_err(hr, "GetMethod(GetBatteryHealthControlStatus)"));
            }
            let mut in_params: *mut c_void = null_mut();
            let hr = class_spawn_instance(in_class, &mut in_params);
            release(in_class);
            release(out_class);
            if hr != 0 {
                return Err(com_err(hr, "SpawnInstance"));
            }

            let scalar = |value: u8| Variant {
                vt: VT_UI1,
                _w_reserved1: 0,
                _w_reserved2: 0,
                _w_reserved3: 0,
                data: VariantData { b_val: value },
            };
            let battery_v = scalar(battery);
            let hr = class_put(in_params, wide("uBatteryNo").as_ptr(), &battery_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                return Err(com_err(hr, "Put(uBatteryNo)"));
            }
            let query_v = scalar(query);
            let hr = class_put(in_params, wide("uFunctionQuery").as_ptr(), &query_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                return Err(com_err(hr, "Put(uFunctionQuery)"));
            }
            let parray = reserved_array(2);
            if parray.is_null() {
                release(in_params);
                return Err(unexpected("SafeArrayCreate(uReserved) failed"));
            }
            let mut data: *mut c_void = null_mut();
            SafeArrayAccessData(parray, &mut data);
            std::ptr::write_bytes(data as *mut u8, 0, 2);
            SafeArrayUnaccessData(parray);
            let reserved_v = Variant {
                vt: VT_UI1 | VT_ARRAY,
                _w_reserved1: 0,
                _w_reserved2: 0,
                _w_reserved3: 0,
                data: VariantData { parray },
            };
            let hr = class_put(in_params, wide("uReserved").as_ptr(), &reserved_v, CIM_UINT8);
            if hr != 0 {
                release(in_params);
                SafeArrayDestroy(parray);
                return Err(com_err(hr, "Put(uReserved)"));
            }

            let method_bstr = bstr(METHOD_GET);
            let mut out_params: *mut c_void = null_mut();
            let hr = exec_method(
                self.services,
                path,
                method_bstr,
                in_params,
                &mut out_params,
            );
            SysFreeString(method_bstr);
            release(in_params);
            SafeArrayDestroy(parray);
            if hr != 0 {
                return Ok(None);
            }
            if out_params.is_null() {
                return Ok(None);
            }
            let list = match self.read_variant_u32(out_params, "uFunctionList")? {
                Some(l) => l,
                None => {
                    release(out_params);
                    return Ok(None);
                }
            };
            let statuses = self.read_status_array(out_params)?;
            release(out_params);
            if statuses.is_empty() {
                return Ok(None);
            }
            Ok(Some((list, statuses)))
        }
    }

    /// Toggle the 80% charge cap via the AMD direct-trust write path
    /// (`SetBatteryHealthControl` with the proven tuple).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ChargeError> {
        let status = u8::from(enabled);
        let mut attempts: Vec<(u8, u8, u8, [u8; 5])> = vec![direct_trust_tuple(status)];
        attempts.extend(fallback_tuples(status));
        unsafe {
            let path = self.instance_path()?;
            let mut last_rejected: Option<u32> = None;
            let mut last_error: Option<ChargeError> = None;
            for (battery, mask, status_byte, reserved) in attempts {
                match self.exec_set(path, battery, mask, status_byte, &reserved) {
                    Ok(Some(return_value)) => {
                        if method_succeeded(0, Some(return_value)) {
                            SysFreeString(path);
                            return Ok(());
                        }
                        last_rejected = Some(return_value);
                    }
                    Ok(None) => last_rejected = Some(0),
                    Err(e) => last_error = Some(e),
                }
                std::thread::sleep(ATTEMPT_DELAY);
            }
            SysFreeString(path);
            match last_error {
                Some(e) => Err(e),
                None => Err(unexpected(format!(
                    "provider rejected every SetBatteryHealthControl tuple (last ReturnValue={})",
                    last_rejected.unwrap_or(0)
                ))),
            }
        }
    }

    /// Read back the current smart-charge state (`GetBatteryHealthControlStatus`).
    pub fn is_enabled(&self) -> Result<bool, ChargeError> {
        unsafe {
            let path = self.instance_path()?;
            let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
            for battery in 0u8..5 {
                for query in 0u8..7 {
                    if let Some(row) = self.exec_get(path, battery, query)? {
                        rows.push(row);
                    }
                }
            }
            SysFreeString(path);
            let refs: Vec<(u32, &[u8])> = rows
                .iter()
                .map(|(list, statuses)| (*list, statuses.as_slice()))
                .collect();
            match desired_status_from_rows(&refs) {
                Some(status) => Ok(status == 1),
                None => Err(unexpected(
                    "no health status found across GetBatteryHealthControlStatus sweep",
                )),
            }
        }
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
            (0, 1, 1, [0, 0, 0, 0, 0]),
            (1, 2, 1, [0, 0, 0, 0, 0]),
            (0, 2, 1, [0, 0, 0, 0, 0]),
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
        assert!(method_succeeded(0, None));
    }
}
