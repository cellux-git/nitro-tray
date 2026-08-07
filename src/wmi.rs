//! In-process raw COM/WMI control of the Acer gaming firmware
//! (`AcerGamingFunction` in `ROOT\WMI`, instance `ACPI\PNP0C14\APGe_0`).
//! Opcode/method encodings match the proven AeroForge tables (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No PowerShell/CIM fallback
//! exists — everything is raw COM `ExecMethod`.
//!
//! windows-sys 0.61 does not ship the `IWbemLocator`/`IWbemServices`/
//! `IWbemClassObject` COM interfaces, so their vtables are declared here
//! (canonical wbemcli.h layouts, identical to AeroForge's hand-rolled
//! bindings). `VARIANT` is likewise declared locally because the
//! `Win32_System_Variant` feature is not enabled.

use std::ptr;

use windows_sys::Win32::Foundation::{
    SysAllocString, SysFreeString, RPC_E_CHANGED_MODE, RPC_E_TOO_LATE, REGDB_E_CLASSNOTREG, S_FALSE,
    S_OK,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows_sys::Win32::System::Wmi::{
    CIM_UINT32, CIM_UINT64, WBEM_E_INVALID_CLASS, WBEM_E_NOT_FOUND, WbemLocator,
};

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

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

const NAMESPACE: &str = "ROOT\\WMI";
const CLASS_NAME: &str = "AcerGamingFunction";
const INSTANCE_PATH: &str = "AcerGamingFunction.InstanceName=\"ACPI\\PNP0C14\\APGe_0\"";
const IN_PARAM: &str = "gmInput";
const OUT_PARAM: &str = "gmOutput";

const IID_IWBEM_LOCATOR: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0xdc12a687_737f_11cf_884d_00aa004b2e24);

const VT_EMPTY: u16 = 0;
const VT_I2: u16 = 2;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_UI1: u16 = 17;
const VT_UI2: u16 = 18;
const VT_UI4: u16 = 19;
const VT_I8: u16 = 20;
const VT_UI8: u16 = 21;

#[repr(C)]
#[allow(dead_code)]
struct IWbemLocator_Vtbl {
    base: windows_sys::core::IUnknown_Vtbl,
    connect_server: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        network_resource: windows_sys::core::BSTR,
        user: windows_sys::core::BSTR,
        password: windows_sys::core::BSTR,
        locale: windows_sys::core::BSTR,
        security_flags: i32,
        authority: windows_sys::core::BSTR,
        ctx: *mut core::ffi::c_void,
        namespace: *mut *mut core::ffi::c_void,
    ) -> i32,
}

#[repr(C)]
#[allow(dead_code)]
struct IWbemServices_Vtbl {
    base: windows_sys::core::IUnknown_Vtbl,
    open_namespace: usize,
    cancel_async_call: usize,
    query_object_sink: usize,
    get_object: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        object_path: windows_sys::core::BSTR,
        flags: i32,
        ctx: *mut core::ffi::c_void,
        object: *mut *mut core::ffi::c_void,
        call_result: *mut core::ffi::c_void,
    ) -> i32,
    get_object_async: usize,
    put_class: usize,
    put_class_async: usize,
    delete_class: usize,
    delete_class_async: usize,
    create_class_enum: usize,
    create_class_enum_async: usize,
    put_instance: usize,
    put_instance_async: usize,
    delete_instance: usize,
    delete_instance_async: usize,
    create_instance_enum: usize,
    create_instance_enum_async: usize,
    exec_query: usize,
    exec_query_async: usize,
    exec_notification_query: usize,
    exec_notification_query_async: usize,
    exec_method: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        object_path: windows_sys::core::BSTR,
        method_name: windows_sys::core::BSTR,
        flags: i32,
        ctx: *mut core::ffi::c_void,
        in_params: *mut core::ffi::c_void,
        out_params: *mut *mut core::ffi::c_void,
        call_result: *mut core::ffi::c_void,
    ) -> i32,
    exec_method_async: usize,
}

#[repr(C)]
#[allow(dead_code)]
struct IWbemClassObject_Vtbl {
    base: windows_sys::core::IUnknown_Vtbl,
    get_qualifier_set: usize,
    get: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        name: windows_sys::core::PCWSTR,
        flags: i32,
        value: *mut Variant,
        cim_type: *mut i32,
        flavor: *mut i32,
    ) -> i32,
    put: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        name: windows_sys::core::PCWSTR,
        flags: i32,
        value: *mut Variant,
        cim_type: i32,
    ) -> i32,
    delete: usize,
    get_names: usize,
    begin_enumeration: usize,
    next: usize,
    end_enumeration: usize,
    get_property_qualifier_set: usize,
    clone: usize,
    get_object_text: usize,
    spawn_derived_class: usize,
    spawn_instance:
        unsafe extern "system" fn(this: *mut core::ffi::c_void, flags: i32, instance: *mut *mut core::ffi::c_void) -> i32,
    compare_to: usize,
    get_property_origin: usize,
    inherits_from: usize,
    get_method: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        name: windows_sys::core::PCWSTR,
        flags: i32,
        in_signature: *mut *mut core::ffi::c_void,
        out_signature: *mut *mut core::ffi::c_void,
    ) -> i32,
    put_method: usize,
    delete_method: usize,
    get_method_origin: usize,
    begin_method_enumeration: usize,
    next_method: usize,
    end_method_enumeration: usize,
}

#[repr(C)]
struct Variant {
    vt: u16,
    reserved1: u16,
    reserved2: u16,
    reserved3: u16,
    data: VariantData,
}

#[repr(C)]
union VariantData {
    bstr_val: windows_sys::core::BSTR,
    ull_val: u64,
    ll_val: i64,
    ul_val: u32,
    l_val: i32,
    ui_val: u16,
    i_val: i16,
    b_val: u8,
    reserved: u64,
}

impl Variant {
    fn empty() -> Self {
        Self {
            vt: VT_EMPTY,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            data: VariantData { reserved: 0 },
        }
    }

    fn ui4(value: u32) -> Self {
        Self {
            vt: VT_UI4,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            data: VariantData { ul_val: value },
        }
    }

    fn u64_bstr(value: u64) -> Option<(Self, WmiBstr)> {
        let bstr = WmiBstr::new(&value.to_string())?;
        Some((
            Self {
                vt: VT_BSTR,
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                data: VariantData { bstr_val: bstr.0 },
            },
            bstr,
        ))
    }
}

struct WmiBstr(windows_sys::core::BSTR);

impl WmiBstr {
    fn new(value: &str) -> Option<Self> {
        let wide: Vec<u16> = value.encode_utf16().collect();
        let bstr = unsafe { SysAllocString(wide.as_ptr()) };
        if bstr.is_null() {
            None
        } else {
            Some(Self(bstr))
        }
    }
}

impl Drop for WmiBstr {
    fn drop(&mut self) {
        unsafe { SysFreeString(self.0) };
    }
}

struct IWbemLocator(*mut IWbemLocator_Vtbl);

impl IWbemLocator {
    unsafe fn connect_server(
        &self,
        namespace: windows_sys::core::BSTR,
        services: *mut *mut core::ffi::c_void,
    ) -> i32 {
        unsafe {
            ((*self.0).connect_server)(
                self.0.cast(),
                namespace,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                ptr::null(),
                ptr::null_mut(),
                services,
            )
        }
    }
}

struct IWbemServices(*mut IWbemServices_Vtbl);

impl IWbemServices {
    unsafe fn get_object(
        &self,
        object_path: windows_sys::core::BSTR,
        object: *mut *mut core::ffi::c_void,
    ) -> i32 {
        unsafe { ((*self.0).get_object)(self.0.cast(), object_path, 0, ptr::null_mut(), object, ptr::null_mut()) }
    }

    unsafe fn exec_method(
        &self,
        object_path: windows_sys::core::BSTR,
        method_name: windows_sys::core::BSTR,
        in_params: *mut core::ffi::c_void,
        out_params: *mut *mut core::ffi::c_void,
    ) -> i32 {
        unsafe {
            ((*self.0).exec_method)(
                self.0.cast(),
                object_path,
                method_name,
                0,
                ptr::null_mut(),
                in_params,
                out_params,
                ptr::null_mut(),
            )
        }
    }
}

struct IWbemClassObject(*mut IWbemClassObject_Vtbl);

impl IWbemClassObject {
    unsafe fn get_method(
        &self,
        name: windows_sys::core::PCWSTR,
        in_signature: *mut *mut core::ffi::c_void,
        out_signature: *mut *mut core::ffi::c_void,
    ) -> i32 {
        unsafe { ((*self.0).get_method)(self.0.cast(), name, 0, in_signature, out_signature) }
    }

    unsafe fn spawn_instance(&self, instance: *mut *mut core::ffi::c_void) -> i32 {
        unsafe { ((*self.0).spawn_instance)(self.0.cast(), 0, instance) }
    }

    unsafe fn put(&self, name: windows_sys::core::PCWSTR, value: *const Variant, cim_type: i32) -> i32 {
        unsafe { ((*self.0).put)(self.0.cast(), name, 0, value as *mut Variant, cim_type) }
    }

    unsafe fn get(&self, name: windows_sys::core::PCWSTR, value: *mut Variant) -> i32 {
        unsafe { ((*self.0).get)(self.0.cast(), name, 0, value, ptr::null_mut(), ptr::null_mut()) }
    }
}

unsafe fn com_release(ptr: *mut core::ffi::c_void) {
    if !ptr.is_null() {
        unsafe {
            let vtbl = *(ptr as *const *const windows_sys::core::IUnknown_Vtbl);
            ((*vtbl).Release)(ptr);
        }
    }
}

struct ComRef(*mut core::ffi::c_void);

impl Drop for ComRef {
    fn drop(&mut self) {
        unsafe { com_release(self.0) };
    }
}

fn wide(value: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    wide.push(0);
    wide
}

fn hr_error(hr: i32, op: &'static str) -> WmiError {
    if hr == WBEM_E_NOT_FOUND {
        WmiError::Unexpected(format!("{op}: WBEM_E_NOT_FOUND"))
    } else {
        WmiError::Com { hr, op }
    }
}

fn variant_to_u64(variant: &Variant) -> Option<u64> {
    unsafe {
        match variant.vt {
            VT_UI1 => Some(variant.data.b_val as u64),
            VT_UI2 => Some(variant.data.ui_val as u64),
            VT_UI4 => Some(variant.data.ul_val as u64),
            VT_UI8 => Some(variant.data.ull_val),
            VT_I2 => u64::try_from(variant.data.i_val).ok(),
            VT_I4 => u64::try_from(variant.data.l_val).ok(),
            VT_I8 => u64::try_from(variant.data.ll_val).ok(),
            VT_BSTR => {
                let bstr = variant.data.bstr_val;
                if bstr.is_null() {
                    return None;
                }
                let mut len = 0usize;
                while *bstr.add(len) != 0 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(core::slice::from_raw_parts(bstr, len));
                text.trim().parse::<u64>().ok()
            }
            _ => None,
        }
    }
}

pub struct WmiAdapter {
    locator: IWbemLocator,
    services: IWbemServices,
    class: IWbemClassObject,
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
        unsafe {
            let hr = CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32);
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                return Err(WmiError::Com { hr, op: "CoInitializeEx" });
            }
            let initialized = hr == S_OK || hr == S_FALSE;
            let cleanup = |err: WmiError| {
                if initialized {
                    CoUninitialize();
                }
                err
            };

            let hr = CoInitializeSecurity(
                ptr::null_mut(),
                -1,
                ptr::null(),
                ptr::null(),
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                ptr::null(),
                EOAC_NONE as u32,
                ptr::null(),
            );
            if hr != S_OK && hr != RPC_E_TOO_LATE {
                return Err(cleanup(WmiError::Com { hr, op: "CoInitializeSecurity" }));
            }

            let mut locator: *mut core::ffi::c_void = ptr::null_mut();
            let hr = CoCreateInstance(
                &WbemLocator,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IWBEM_LOCATOR,
                &mut locator,
            );
            if hr != S_OK {
                return Err(cleanup(if hr == REGDB_E_CLASSNOTREG {
                    WmiError::NotAvailable
                } else {
                    WmiError::Com { hr, op: "CoCreateInstance(CLSID_WbemLocator)" }
                }));
            }
            if locator.is_null() {
                return Err(cleanup(WmiError::Unexpected(
                    "CoCreateInstance returned a null locator".into(),
                )));
            }

            let namespace = match WmiBstr::new(NAMESPACE) {
                Some(b) => b,
                None => {
                    com_release(locator);
                    return Err(cleanup(WmiError::Unexpected("SysAllocString(namespace) failed".into())));
                }
            };
            let mut services: *mut core::ffi::c_void = ptr::null_mut();
            let hr = IWbemLocator(locator.cast()).connect_server(namespace.0, &mut services);
            drop(namespace);
            if hr != S_OK {
                com_release(locator);
                return Err(cleanup(WmiError::Com { hr, op: "ConnectServer(ROOT\\WMI)" }));
            }
            if services.is_null() {
                com_release(locator);
                return Err(cleanup(WmiError::Unexpected("ConnectServer returned null services".into())));
            }

            let hr = CoSetProxyBlanket(
                services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                ptr::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                ptr::null(),
                EOAC_NONE as u32,
            );
            if hr != S_OK {
                com_release(services);
                com_release(locator);
                return Err(cleanup(WmiError::Com { hr, op: "CoSetProxyBlanket" }));
            }

            let class_path = match WmiBstr::new(CLASS_NAME) {
                Some(b) => b,
                None => {
                    com_release(services);
                    com_release(locator);
                    return Err(cleanup(WmiError::Unexpected("SysAllocString(class) failed".into())));
                }
            };
            let mut class: *mut core::ffi::c_void = ptr::null_mut();
            let hr = IWbemServices(services.cast()).get_object(class_path.0, &mut class);
            drop(class_path);
            if hr != S_OK {
                com_release(services);
                com_release(locator);
                return Err(cleanup(if hr == WBEM_E_INVALID_CLASS || hr == WBEM_E_NOT_FOUND {
                    WmiError::NotAvailable
                } else {
                    WmiError::Com { hr, op: "GetObject(AcerGamingFunction)" }
                }));
            }
            if class.is_null() {
                com_release(services);
                com_release(locator);
                return Err(cleanup(WmiError::Unexpected("GetObject returned null class".into())));
            }

            Ok(Self {
                locator: IWbemLocator(locator.cast()),
                services: IWbemServices(services.cast()),
                class: IWbemClassObject(class.cast()),
            })
        }
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
        unsafe {
            let mut in_signature: *mut core::ffi::c_void = ptr::null_mut();
            let mut out_signature: *mut core::ffi::c_void = ptr::null_mut();
            let method_wide = wide(method);
            let hr = self.class.get_method(method_wide.as_ptr(), &mut in_signature, &mut out_signature);
            com_release(out_signature);
            if hr != S_OK {
                return Err(hr_error(hr, "GetMethod"));
            }
            if in_signature.is_null() {
                return Err(WmiError::Unexpected(format!("{method}: no input signature")));
            }

            let mut in_params: *mut core::ffi::c_void = ptr::null_mut();
            let hr = IWbemClassObject(in_signature.cast()).spawn_instance(&mut in_params);
            com_release(in_signature);
            if hr != S_OK {
                return Err(hr_error(hr, "SpawnInstance"));
            }
            if in_params.is_null() {
                return Err(WmiError::Unexpected(format!("{method}: null input instance")));
            }

            let in_param_wide = wide(IN_PARAM);
            let in_params = IWbemClassObject(in_params.cast());
            let _in_params_guard = ComRef(in_params.0.cast());
            let ui4 = Variant::ui4(input as u32);
            let hr = in_params.put(in_param_wide.as_ptr(), &ui4, CIM_UINT32);
            if hr != S_OK {
                let (u64_bstr, guard) = Variant::u64_bstr(input)
                    .ok_or_else(|| WmiError::Unexpected("SysAllocString(gmInput) failed".into()))?;
                let hr = in_params.put(in_param_wide.as_ptr(), &u64_bstr, CIM_UINT64);
                drop(guard);
                if hr != S_OK {
                    return Err(hr_error(hr, "Put(gmInput)"));
                }
            }

            let path = match WmiBstr::new(INSTANCE_PATH) {
                Some(b) => b,
                None => return Err(WmiError::Unexpected("SysAllocString(instance path) failed".into())),
            };
            let method_bstr = match WmiBstr::new(method) {
                Some(b) => b,
                None => return Err(WmiError::Unexpected("SysAllocString(method) failed".into())),
            };
            let mut out_params: *mut core::ffi::c_void = ptr::null_mut();
            let hr = self.services.exec_method(path.0, method_bstr.0, in_params.0.cast(), &mut out_params);
            if hr != S_OK {
                return Err(hr_error(hr, method));
            }
            if out_params.is_null() {
                return Ok(None);
            }

            let mut value = Variant::empty();
            let out_param_wide = wide(OUT_PARAM);
            let hr = IWbemClassObject(out_params.cast()).get(out_param_wide.as_ptr(), &mut value);
            com_release(out_params);
            if hr != S_OK {
                return Err(hr_error(hr, "Get(gmOutput)"));
            }
            let result = variant_to_u64(&value);
            if value.vt == VT_BSTR {
                SysFreeString(value.data.bstr_val);
            }
            Ok(result)
        }
    }
}

impl Drop for WmiAdapter {
    fn drop(&mut self) {
        unsafe {
            com_release(self.class.0.cast());
            com_release(self.services.0.cast());
            com_release(self.locator.0.cast());
            CoUninitialize();
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
