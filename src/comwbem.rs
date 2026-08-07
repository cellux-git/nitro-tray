//! Shared in-process COM/WMI plumbing for the WMI and smart-charge adapters.
//!
//! windows-sys 0.61 ships the WMI constants (CLSID_WbemLocator, CIM types)
//! but no `IWbem*` COM bindings and no `VARIANT` (the `Win32_System_Variant`
//! feature is not enabled), so the canonical wbemcli.h vtables and a minimal
//! `VARIANT` live here — one definition shared by both adapters so the
//! layouts cannot drift apart.
//!
//! All entry points are `unsafe`; callers must keep the wrapped interfaces
//! alive (RAII guards) and respect the ownership contracts documented on each
//! item. HRESULTs are returned raw and mapped to per-adapter errors by
//! callers.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::core::{BSTR, GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{
    SysAllocString, SysFreeString, E_FAIL, E_OUTOFMEMORY, RPC_E_CHANGED_MODE, RPC_E_TOO_LATE, S_OK,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize,
    SAFEARRAY, SAFEARRAYBOUND, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows_sys::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayUnaccessData};
use windows_sys::Win32::System::Wmi::{CIMTYPE_ENUMERATION, WbemLocator};

/// IID_IWbemLocator.
pub const IID_WBEM_LOCATOR: GUID = GUID::from_u128(0xdc12a687_737f_11cf_884d_00aa004b2e24);

// VARIANT type tags (oleauto).
pub const VT_EMPTY: u16 = 0;
pub const VT_NULL: u16 = 1;
pub const VT_I2: u16 = 2;
pub const VT_I4: u16 = 3;
pub const VT_BSTR: u16 = 8;
pub const VT_UI1: u16 = 17;
pub const VT_UI2: u16 = 18;
pub const VT_UI4: u16 = 19;
pub const VT_I8: u16 = 20;
pub const VT_UI8: u16 = 21;
pub const VT_ARRAY: u16 = 0x2000;

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

#[link(name = "oleaut32")]
unsafe extern "system" {
    fn SafeArrayCreate(vt: u16, cdims: u32, rgsabound: *const SAFEARRAYBOUND) -> *mut SAFEARRAY;
}

/// COM apartment: `CoInitializeEx(COINIT_MULTITHREADED)` +
/// `CoInitializeSecurity` (first call wins; RPC_E_TOO_LATE is tolerated),
/// `CoUninitialize` on drop only when this guard performed the init.
pub struct ComApartment {
    own_init: bool,
}

impl ComApartment {
    /// Initialize the COM apartment for this thread. `Ok` even when the
    /// apartment already exists (RPC_E_CHANGED_MODE).
    pub fn init() -> Result<Self, HRESULT> {
        unsafe {
            let hr = CoInitializeEx(null_mut(), COINIT_MULTITHREADED as u32);
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                return Err(hr);
            }
            let own_init = hr == S_OK;
            let hr = CoInitializeSecurity(
                null_mut(),
                -1,
                null(),
                null(),
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                null(),
                EOAC_NONE as u32,
                null(),
            );
            if hr != S_OK && hr != RPC_E_TOO_LATE {
                if own_init {
                    CoUninitialize();
                }
                return Err(hr);
            }
            Ok(ComApartment { own_init })
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.own_init {
            unsafe { CoUninitialize() };
        }
    }
}

/// Canonical wbemcli.h vtable slot placeholder for methods this crate never
/// calls (kept as typed slots so the layout stays exact).
type Slot = unsafe extern "system" fn(*mut c_void) -> HRESULT;

#[repr(C)]
pub struct IWbemLocator_Vtbl {
    pub base: windows_sys::core::IUnknown_Vtbl,
    pub connect_server: unsafe extern "system" fn(
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
pub struct IWbemServices_Vtbl {
    pub base: windows_sys::core::IUnknown_Vtbl,
    pub open_namespace: Slot,
    pub cancel_async_call: Slot,
    pub query_object_sink: Slot,
    pub get_object: unsafe extern "system" fn(
        this: *mut c_void,
        object_path: BSTR,
        flags: i32,
        ctx: *mut c_void,
        object: *mut *mut c_void,
        call_result: *mut c_void,
    ) -> HRESULT,
    pub get_object_async: Slot,
    pub put_class: Slot,
    pub put_class_async: Slot,
    pub delete_class: Slot,
    pub delete_class_async: Slot,
    pub create_class_enum: Slot,
    pub create_class_enum_async: Slot,
    pub put_instance: Slot,
    pub put_instance_async: Slot,
    pub delete_instance: Slot,
    pub delete_instance_async: Slot,
    pub create_instance_enum: unsafe extern "system" fn(
        this: *mut c_void,
        class: BSTR,
        flags: i32,
        ctx: *mut c_void,
        enumerator: *mut *mut c_void,
    ) -> HRESULT,
    pub create_instance_enum_async: Slot,
    pub exec_query: Slot,
    pub exec_query_async: Slot,
    pub exec_notification_query: Slot,
    pub exec_notification_query_async: Slot,
    pub exec_method: unsafe extern "system" fn(
        this: *mut c_void,
        object_path: BSTR,
        method_name: BSTR,
        flags: i32,
        ctx: *mut c_void,
        in_params: *mut c_void,
        out_params: *mut *mut c_void,
        call_result: *mut c_void,
    ) -> HRESULT,
    pub exec_method_async: Slot,
}

#[repr(C)]
pub struct IWbemClassObject_Vtbl {
    pub base: windows_sys::core::IUnknown_Vtbl,
    pub get_qualifier_set: Slot,
    pub get: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        value: *mut Variant,
        cim_type: *mut i32,
        flavor: *mut i32,
    ) -> HRESULT,
    pub put: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        value: *const Variant,
        cim_type: CIMTYPE_ENUMERATION,
    ) -> HRESULT,
    pub delete: Slot,
    pub get_names: Slot,
    pub begin_enumeration: Slot,
    pub next: Slot,
    pub end_enumeration: Slot,
    pub get_property_qualifier_set: Slot,
    pub clone: Slot,
    pub get_object_text: Slot,
    pub spawn_derived_class: Slot,
    pub spawn_instance: unsafe extern "system" fn(
        this: *mut c_void,
        flags: i32,
        instance: *mut *mut c_void,
    ) -> HRESULT,
    pub compare_to: Slot,
    pub get_property_origin: Slot,
    pub inherits_from: Slot,
    pub get_method: unsafe extern "system" fn(
        this: *mut c_void,
        name: PCWSTR,
        flags: i32,
        in_signature: *mut *mut c_void,
        out_signature: *mut *mut c_void,
    ) -> HRESULT,
    pub put_method: Slot,
    pub delete_method: Slot,
    pub begin_method_enumeration: Slot,
    pub next_method: Slot,
    pub end_method_enumeration: Slot,
    pub get_method_qualifier_set: Slot,
    pub get_method_origin: Slot,
    pub put_method_qualifier_set: Slot,
}

#[repr(C)]
pub struct IEnumWbemClassObject_Vtbl {
    pub base: windows_sys::core::IUnknown_Vtbl,
    pub reset: Slot,
    pub next: unsafe extern "system" fn(
        this: *mut c_void,
        timeout: i32,
        count: u32,
        objects: *mut *mut c_void,
        returned: *mut u32,
    ) -> HRESULT,
    pub next_async: Slot,
    pub clone: Slot,
    pub skip: Slot,
}

/// Minimal C-layout `VARIANT` covering the members WMI in/out parameters use.
/// Owns its payload: `Drop` frees BSTR/SAFEARRAY contents.
#[repr(C)]
pub struct Variant {
    vt: u16,
    _w_reserved1: u16,
    _w_reserved2: u16,
    _w_reserved3: u16,
    data: VariantData,
}

#[repr(C)]
union VariantData {
    i_val: i16,
    l_val: i32,
    b_val: u8,
    ui2_val: u16,
    ul_val: u32,
    ll_val: i64,
    ull_val: u64,
    bstr_val: BSTR,
    parray: *mut SAFEARRAY,
}

impl Variant {
    pub fn empty() -> Self {
        Self {
            vt: VT_EMPTY,
            _w_reserved1: 0,
            _w_reserved2: 0,
            _w_reserved3: 0,
            data: VariantData { ull_val: 0 },
        }
    }

    pub fn ui1(value: u8) -> Self {
        Self {
            vt: VT_UI1,
            _w_reserved1: 0,
            _w_reserved2: 0,
            _w_reserved3: 0,
            data: VariantData { b_val: value },
        }
    }

    pub fn ui4(value: u32) -> Self {
        Self {
            vt: VT_UI4,
            _w_reserved1: 0,
            _w_reserved2: 0,
            _w_reserved3: 0,
            data: VariantData { ul_val: value },
        }
    }

    /// Takes ownership of the given BSTR.
    pub fn from_bstr(bstr: BSTR) -> Self {
        Self {
            vt: VT_BSTR,
            _w_reserved1: 0,
            _w_reserved2: 0,
            _w_reserved3: 0,
            data: VariantData { bstr_val: bstr },
        }
    }

    /// Takes ownership of the given SAFEARRAY.
    pub fn from_array(parray: *mut SAFEARRAY) -> Self {
        Self {
            vt: VT_UI1 | VT_ARRAY,
            _w_reserved1: 0,
            _w_reserved2: 0,
            _w_reserved3: 0,
            data: VariantData { parray },
        }
    }

    pub fn is_null_or_empty(&self) -> bool {
        self.vt == VT_EMPTY || self.vt == VT_NULL
    }

    pub fn as_u64(&self) -> Option<u64> {
        unsafe {
            match self.vt {
                VT_UI1 => Some(self.data.b_val as u64),
                VT_UI2 => Some(self.data.ui2_val as u64),
                VT_UI4 => Some(self.data.ul_val as u64),
                VT_UI8 => Some(self.data.ull_val),
                VT_I2 => u64::try_from(self.data.i_val).ok(),
                VT_I4 => u64::try_from(self.data.l_val).ok(),
                VT_I8 => u64::try_from(self.data.ll_val).ok(),
                VT_BSTR => {
                    let bstr = self.data.bstr_val;
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

    pub fn as_u8_array(&self) -> Option<Vec<u8>> {
        if self.vt != VT_UI1 | VT_ARRAY {
            return None;
        }
        let parray = unsafe { self.data.parray };
        if parray.is_null() {
            return Some(Vec::new());
        }
        unsafe {
            let len = (*parray).rgsabound[0].cElements as usize;
            let mut data: *mut c_void = null_mut();
            if SafeArrayAccessData(parray, &mut data) != 0 {
                return None;
            }
            let bytes = core::slice::from_raw_parts(data as *const u8, len).to_vec();
            SafeArrayUnaccessData(parray);
            Some(bytes)
        }
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            match self.vt {
                VT_BSTR => SysFreeString(self.data.bstr_val),
                VT_UI1 | VT_ARRAY => {
                    let _ = SafeArrayDestroy(self.data.parray);
                }
                _ => {}
            }
        }
    }
}

/// A BSTR that frees itself on drop.
pub struct Bstr(BSTR);

impl Bstr {
    pub fn new(text: &str) -> Option<Self> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let bstr = unsafe { SysAllocString(wide.as_ptr()) };
        if bstr.is_null() {
            None
        } else {
            Some(Self(bstr))
        }
    }

    /// Allocate an owned copy of an existing BSTR (for taking ownership of
    /// out-params that a `Variant` will free). The argument is an opaque
    /// BSTR handle passed straight to oleaut32; it is not dereferenced here.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn copy_of(other: BSTR) -> Option<Self> {
        if other.is_null() {
            return None;
        }
        let bstr = unsafe { SysAllocString(other) };
        if bstr.is_null() {
            None
        } else {
            Some(Self(bstr))
        }
    }

    pub fn raw(&self) -> BSTR {
        self.0
    }

    /// Hand the BSTR to the caller without freeing it (ownership transfer).
    pub fn into_raw(self) -> BSTR {
        let raw = self.0;
        core::mem::forget(self);
        raw
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        unsafe { SysFreeString(self.0) };
    }
}

/// RAII release of a COM interface pointer.
pub struct ComRef(*mut c_void);

impl ComRef {
    pub fn from_raw(ptr: *mut c_void) -> Self {
        ComRef(ptr)
    }

    pub fn raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for ComRef {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let vtbl = *(self.0 as *const *const windows_sys::core::IUnknown_Vtbl);
                ((*vtbl).Release)(self.0);
            }
        }
    }
}

/// An `IWbemClassObject` interface.
pub struct ClassObject(ComRef);

impl ClassObject {
    /// Wraps an `IWbemClassObject` pointer; the wrapper releases it on drop.
    ///
    /// # Safety
    /// `ptr` must be a valid `IWbemClassObject` pointer with an outstanding
    /// reference, and must remain valid for the wrapper's lifetime.
    pub unsafe fn from_raw(ptr: *mut c_void) -> Self {
        ClassObject(ComRef::from_raw(ptr))
    }

    pub fn raw(&self) -> *mut c_void {
        self.0.raw()
    }

    /// # Safety
    /// `name` must be a valid NUL-terminated wide string; out-parameters must
    /// point to writable storage the caller owns.
    pub unsafe fn get_method(
        &self,
        name: PCWSTR,
        in_signature: *mut *mut c_void,
        out_signature: *mut *mut c_void,
    ) -> HRESULT {
        unsafe {
            let vtbl = self.0.raw() as *const IWbemClassObject_Vtbl;
            ((*vtbl).get_method)(self.0.raw(), name, 0, in_signature, out_signature)
        }
    }

    /// # Safety
    /// `instance` must point to writable storage the caller owns.
    pub unsafe fn spawn_instance(&self, instance: *mut *mut c_void) -> HRESULT {
        unsafe {
            let vtbl = self.0.raw() as *const IWbemClassObject_Vtbl;
            ((*vtbl).spawn_instance)(self.0.raw(), 0, instance)
        }
    }

    /// # Safety
    /// `name` must be a valid NUL-terminated wide string; `value` must be a
    /// live `Variant` for the duration of the call.
    pub unsafe fn put(&self, name: PCWSTR, value: &Variant, cim_type: CIMTYPE_ENUMERATION) -> HRESULT {
        unsafe {
            let vtbl = self.0.raw() as *const IWbemClassObject_Vtbl;
            ((*vtbl).put)(self.0.raw(), name, 0, value, cim_type)
        }
    }

    /// Returns a `Variant` the caller owns (freed on drop). `WBEM_E_NOT_FOUND`
    /// surfaces as `Err(hr)` like any other failure.
    ///
    /// # Safety
    /// `name` must be a valid NUL-terminated wide string.
    pub unsafe fn get(&self, name: PCWSTR) -> Result<Variant, HRESULT> {
        unsafe {
            let vtbl = self.0.raw() as *const IWbemClassObject_Vtbl;
            let mut value = Variant::empty();
            let hr = ((*vtbl).get)(self.0.raw(), name, 0, &mut value, null_mut(), null_mut());
            if hr != 0 {
                return Err(hr);
            }
            Ok(value)
        }
    }
}

/// Wide NUL-terminated buffer for API calls.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// `CoCreateInstance(CLSID_WbemLocator)`. Requires the COM apartment to be
/// initialized (see `ComApartment::init`).
///
/// # Safety
/// Call only from a thread with an initialized COM apartment.
pub unsafe fn create_locator() -> Result<ComRef, HRESULT> {
    unsafe {
        let mut locator: *mut c_void = null_mut();
        let hr = CoCreateInstance(
            &WbemLocator,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_WBEM_LOCATOR,
            &mut locator,
        );
        if hr != 0 {
            return Err(hr);
        }
        if locator.is_null() {
            return Err(E_FAIL);
        }
        Ok(ComRef::from_raw(locator))
    }
}

/// `IWbemLocator::ConnectServer(namespace)` + `CoSetProxyBlanket` on the
/// returned services (local WMI: default authentication, impersonation).
///
/// # Safety
/// `locator` must be a live locator from `create_locator`; the returned
/// services are wrapped in a releasing guard.
pub unsafe fn connect_server(locator: &ComRef, namespace: &str) -> Result<ComRef, HRESULT> {
    unsafe {
        let ns = Bstr::new(namespace).ok_or(windows_sys::Win32::Foundation::E_OUTOFMEMORY)?;
        let vtbl = locator.raw() as *const IWbemLocator_Vtbl;
        let mut services: *mut c_void = null_mut();
        let hr = ((*vtbl).connect_server)(
            locator.raw(),
            ns.raw(),
            null(),
            null(),
            null(),
            0,
            null(),
            null_mut(),
            &mut services,
        );
        drop(ns);
        if hr != 0 {
            return Err(hr);
        }
        if services.is_null() {
            return Err(windows_sys::Win32::Foundation::E_FAIL);
        }
        let blanket = CoSetProxyBlanket(
            services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            null(),
            EOAC_NONE as u32,
        );
        if blanket != 0 {
            return Err(blanket);
        }
        Ok(ComRef::from_raw(services))
    }
}

/// `IWbemServices::GetObject(class)`.
///
/// # Safety
/// `services` must be a live services connection; the returned class object
/// is wrapped in a releasing guard.
pub unsafe fn get_class(services: &ComRef, class: &str) -> Result<ClassObject, HRESULT> {
    unsafe {
        let name = Bstr::new(class).ok_or(windows_sys::Win32::Foundation::E_OUTOFMEMORY)?;
        let vtbl = services.raw() as *const IWbemServices_Vtbl;
        let mut object: *mut c_void = null_mut();
        let hr = ((*vtbl).get_object)(services.raw(), name.raw(), 0, null_mut(), &mut object, null_mut());
        drop(name);
        if hr != 0 {
            return Err(hr);
        }
        if object.is_null() {
            return Err(windows_sys::Win32::Foundation::E_FAIL);
        }
        Ok(ClassObject::from_raw(object))
    }
}

/// Object path (`__PATH`) of the first instance of `class` — the WMI
/// provider's own instance path, used as the `ExecMethod` target.
///
/// # Safety
/// `services` must be a live services connection.
pub unsafe fn first_instance_path(services: &ComRef, class: &str) -> Result<Bstr, HRESULT> {
    unsafe {
        let name = Bstr::new(class).ok_or(E_OUTOFMEMORY)?;
        let vtbl = services.raw() as *const IWbemServices_Vtbl;
        let mut enumerator: *mut c_void = null_mut();
        let hr = ((*vtbl).create_instance_enum)(services.raw(), name.raw(), 0, null_mut(), &mut enumerator);
        drop(name);
        if hr != 0 {
            return Err(hr);
        }
        let enumerator = ComRef::from_raw(enumerator);
        let enum_vtbl = enumerator.raw() as *const IEnumWbemClassObject_Vtbl;
        let mut instance: *mut c_void = null_mut();
        let mut returned: u32 = 0;
        let hr = ((*enum_vtbl).next)(enumerator.raw(), 0, 1, &mut instance, &mut returned);
        if hr != 0 {
            return Err(hr);
        }
        if returned == 0 || instance.is_null() {
            return Err(E_FAIL);
        }
        let instance = ClassObject::from_raw(instance);
        let path = instance.get(wide("__PATH").as_ptr())?;
        if path.vt != VT_BSTR {
            return Err(E_FAIL);
        }
        // The variant frees the provider's BSTR on drop; hand the caller an
        // owned copy.
        Bstr::copy_of(path.data.bstr_val).ok_or(E_OUTOFMEMORY)
    }
}

/// Create a zero-initialized `uint8[len]` SAFEARRAY (caller hands ownership
/// to a `Variant::from_array`).
///
/// # Safety
/// The returned array must be handed to exactly one owner (`Variant`,
/// `SafeArrayDestroy`, or another single owner).
pub unsafe fn uint8_array(len: usize) -> Option<*mut SAFEARRAY> {
    unsafe {
        let bound = SAFEARRAYBOUND {
            cElements: len as u32,
            lLbound: 0,
        };
        let parray = SafeArrayCreate(VT_UI1 | VT_ARRAY, 1, &bound);
        if parray.is_null() {
            return None;
        }
        let mut data: *mut c_void = null_mut();
        if SafeArrayAccessData(parray, &mut data) != 0 {
            SafeArrayDestroy(parray);
            return None;
        }
        core::ptr::write_bytes(data as *mut u8, 0, len);
        SafeArrayUnaccessData(parray);
        Some(parray)
    }
}

/// `IWbemServices::ExecMethod(object_path, method, in_params)`; returns the
/// out-params object when the provider produced one, `Ok(None)` otherwise.
///
/// # Safety
/// `services` must be live; `object_path`/`method` must be live BSTRs;
/// `in_params` must be a live input-instance pointer (or null); the returned
/// out-params object is wrapped in a releasing guard.
pub unsafe fn exec_method(
    services: &ComRef,
    object_path: &Bstr,
    method: &Bstr,
    in_params: *mut c_void,
) -> Result<Option<ClassObject>, HRESULT> {
    unsafe {
        let vtbl = services.raw() as *const IWbemServices_Vtbl;
        let mut out_params: *mut c_void = null_mut();
        let hr = ((*vtbl).exec_method)(
            services.raw(),
            object_path.raw(),
            method.raw(),
            0,
            null_mut(),
            in_params,
            &mut out_params,
            null_mut(),
        );
        if hr != 0 {
            return Err(hr);
        }
        if out_params.is_null() {
            return Ok(None);
        }
        Ok(Some(ClassObject::from_raw(out_params)))
    }
}
