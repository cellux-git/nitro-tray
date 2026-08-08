use std::ffi::c_void;
use std::ptr::{null, null_mut};

use super::{
    MI_BOOLEAN, MI_FLAG_NULL, MI_RESULT_INVALID_PARAMETER, MI_RESULT_NO_SUCH_PROPERTY,
    MI_RESULT_NOT_FOUND, MI_RESULT_OK, MI_RESULT_TYPE_MISMATCH, MI_SINT8, MI_SINT16, MI_SINT32,
    MI_SINT64, MI_UINT8, MI_UINT8A, MI_UINT16, MI_UINT32, MI_UINT64, MiBoolean,
    MiChar, MiError, MiResult, MiType, MiUint32, type_name,
};
use crate::transport::{MiElement, MiInput, MiOutput, MiTransport, MiValue as TransportMiValue};

/// `MI_Timestamp` (mi.h): YYYYMMDDHHMMSS.MMMMMMSUTC, 8 x u32.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MiTimestamp {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub microseconds: u32,
    pub utc: i32,
}

/// `MI_Interval` (mi.h): DDDDDDDDHHMMSS.MMMMMM:000, 8 x u32.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MiInterval {
    pub days: u32,
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
    pub microseconds: u32,
    pub padding: [u32; 3],
}

/// `MI_Datetime` (mi.h): `{ u32 isTimestamp; union { timestamp; interval } }`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MiDatetime {
    pub is_timestamp: u32,
    pub u: MiDatetimeU,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union MiDatetimeU {
    pub timestamp: MiTimestamp,
    pub interval: MiInterval,
}

/// Layout of every `MI_<TYPE>A` array struct: `{ data; size }` (all the
/// union's array members share this shape).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MiArray {
    pub data: *mut c_void,
    pub size: u32,
}

/// `union MI_Value` (mi.h) — one member per CIM type. Only the members the
/// Acer calls need are transcribed; the union is filled by us or read typed,
/// never blindly.
#[repr(C)]
#[derive(Clone, Copy)]
pub union MiValue {
    pub boolean: MiBoolean,
    pub uint8: u8,
    pub sint8: i8,
    pub uint16: u16,
    pub sint16: i16,
    pub uint32: MiUint32,
    pub sint32: i32,
    pub uint64: u64,
    pub sint64: i64,
    pub real32: f32,
    pub real64: f64,
    pub char16: u16,
    pub datetime: MiDatetime,
    pub string: *mut MiChar,
    pub instance: *mut MiInstanceRaw,
    pub reference: *mut MiInstanceRaw,
    pub booleana: MiArray,
    pub uint8a: MiArray,
    pub sint8a: MiArray,
    pub uint16a: MiArray,
    pub sint16a: MiArray,
    pub uint32a: MiArray,
    pub sint32a: MiArray,
    pub uint64a: MiArray,
    pub sint64a: MiArray,
    pub real32a: MiArray,
    pub real64a: MiArray,
    pub char16a: MiArray,
    pub datetimea: MiArray,
    pub stringa: MiArray,
    pub referencea: MiArray,
    pub instancea: MiArray,
    pub array: MiArray,
}

/// Placeholder slot for FT methods this crate never calls (kept typed so the
/// table layout stays exact; comwbem.rs uses the same pattern).
type Slot = unsafe extern "system" fn() -> MiResult;

/// `struct _MI_Application` (mi.h): `{ u64 reserved1; ptrdiff_t reserved2;
/// const MI_ApplicationFT* ft; }` (MI_APPLICATION_NULL initializer).
#[repr(C)]
pub struct MiApplicationRaw {
    pub reserved1: u64,
    pub reserved2: isize,
    pub ft: *const MiApplicationFt,
}

/// `struct _MI_Session` (mi.h), same handle shape.
#[repr(C)]
pub struct MiSessionRaw {
    pub reserved1: u64,
    pub reserved2: isize,
    pub ft: *const MiSessionFt,
}

/// `struct _MI_Operation` (mi.h), same handle shape.
#[repr(C)]
pub struct MiOperationRaw {
    pub reserved1: u64,
    pub reserved2: isize,
    pub ft: *const MiOperationFt,
}

/// `struct _MI_Instance` (mi.h): `{ ft; classDecl; serverName; nameSpace;
/// reserved[4] }`. Only the FT pointer is dereferenced.
#[repr(C)]
pub struct MiInstanceRaw {
    pub ft: *const MiInstanceFt,
    pub class_decl: *const c_void,
    pub server_name: *const MiChar,
    pub namespace: *const MiChar,
    pub reserved: [isize; 4],
}

/// `MI_ApplicationFT` (mi.h) — slot order from the header, verbatim.
#[repr(C)]
pub struct MiApplicationFt {
    pub close: unsafe extern "system" fn(application: *mut MiApplicationRaw) -> MiResult,
    pub new_session: unsafe extern "system" fn(
        application: *mut MiApplicationRaw,
        protocol: *const MiChar,
        destination: *const MiChar,
        options: *mut c_void,
        callbacks: *mut c_void,
        extended_error: *mut *mut MiInstanceRaw,
        session: *mut MiSessionRaw,
    ) -> MiResult,
    pub new_hosted_provider: Slot,
    pub new_instance: unsafe extern "system" fn(
        application: *mut MiApplicationRaw,
        class_name: *const MiChar,
        class_rtti: *const c_void,
        instance: *mut *mut MiInstanceRaw,
    ) -> MiResult,
    pub new_destination_options: Slot,
    pub new_operation_options: Slot,
    pub new_subscription_delivery_options: Slot,
    pub new_serializer: Slot,
    pub new_deserializer: Slot,
    pub new_instance_from_class: Slot,
    pub new_class: Slot,
}

/// `MI_SessionFT` (mi.h) — slot order from the header, verbatim.
#[repr(C)]
pub struct MiSessionFt {
    pub close: unsafe extern "system" fn(
        session: *mut MiSessionRaw,
        completion_context: *mut c_void,
        completion_callback: *mut c_void,
    ) -> MiResult,
    pub get_application: Slot,
    pub get_instance: Slot,
    pub modify_instance: Slot,
    pub create_instance: Slot,
    pub delete_instance: Slot,
    pub invoke: unsafe extern "system" fn(
        session: *mut MiSessionRaw,
        flags: u32,
        options: *mut c_void,
        namespace_name: *const MiChar,
        class_name: *const MiChar,
        method_name: *const MiChar,
        inbound_instance: *const MiInstanceRaw,
        inbound_properties: *const MiInstanceRaw,
        callbacks: *mut c_void,
        operation: *mut MiOperationRaw,
    ),
    pub enumerate_instances: unsafe extern "system" fn(
        session: *mut MiSessionRaw,
        flags: u32,
        options: *mut c_void,
        namespace_name: *const MiChar,
        class_name: *const MiChar,
        keys_only: MiBoolean,
        callbacks: *mut c_void,
        operation: *mut MiOperationRaw,
    ),
    pub query_instances: Slot,
    pub associator_instances: Slot,
    pub reference_instances: Slot,
    pub subscribe: Slot,
    pub get_class: Slot,
    pub enumerate_classes: Slot,
    pub test_connection: Slot,
}

/// `MI_OperationFT` (mi.h) — slot order from the header, verbatim.
#[repr(C)]
pub struct MiOperationFt {
    pub close: unsafe extern "system" fn(operation: *mut MiOperationRaw) -> MiResult,
    pub cancel: Slot,
    pub get_session: Slot,
    pub get_instance: unsafe extern "system" fn(
        operation: *mut MiOperationRaw,
        instance: *mut *const MiInstanceRaw,
        more_results: *mut MiBoolean,
        result: *mut MiResult,
        error_message: *mut *const MiChar,
        completion_details: *mut *const MiInstanceRaw,
    ) -> MiResult,
    pub get_indication: Slot,
    pub get_class: Slot,
}

/// `MI_InstanceFT` (mi.h) — slot order from the header, verbatim.
#[repr(C)]
pub struct MiInstanceFt {
    pub clone: unsafe extern "system" fn(
        self_: *const MiInstanceRaw,
        new_instance: *mut *mut MiInstanceRaw,
    ) -> MiResult,
    pub destruct: Slot,
    pub delete: unsafe extern "system" fn(self_: *mut MiInstanceRaw) -> MiResult,
    pub is_a: Slot,
    pub get_class_name: unsafe extern "system" fn(
        self_: *const MiInstanceRaw,
        class_name: *mut *const MiChar,
    ) -> MiResult,
    pub set_name_space: Slot,
    pub get_name_space: unsafe extern "system" fn(
        self_: *const MiInstanceRaw,
        name_space: *mut *const MiChar,
    ) -> MiResult,
    pub get_element_count: Slot,
    pub add_element: unsafe extern "system" fn(
        self_: *mut MiInstanceRaw,
        name: *const MiChar,
        value: *const MiValue,
        ty: MiType,
        flags: u32,
    ) -> MiResult,
    pub set_element: unsafe extern "system" fn(
        self_: *mut MiInstanceRaw,
        name: *const MiChar,
        value: *const MiValue,
        ty: MiType,
        flags: u32,
    ) -> MiResult,
    pub set_element_at: Slot,
    pub get_element: unsafe extern "system" fn(
        self_: *const MiInstanceRaw,
        name: *const MiChar,
        value: *mut MiValue,
        ty: *mut MiType,
        flags: *mut u32,
        index: *mut u32,
    ) -> MiResult,
    pub get_element_at: Slot,
    pub clear_element: Slot,
    pub clear_element_at: Slot,
    pub get_server_name: Slot,
    pub set_server_name: Slot,
    pub get_class: Slot,
}

// Only `MI_Application_InitializeV1` is imported from mi.dll; everything
// else is called through the function tables (the C inline wrappers do the
// same). mi.dll exports ONLY the versioned name (verified with
// `link /dump /exports` on 10.0.26100 — the plain `MI_Application_Initialize`
// spelling from mi.h is a macro mapping to the V1 export).
// `MI_MAIN_CALL` is `__cdecl` in mi.h — `extern "C"` on Windows.
#[link(name = "mi")]
unsafe extern "C" {
    fn MI_Application_InitializeV1(
        flags: u32,
        application_id: *const MiChar,
        extended_error: *mut *mut MiInstanceRaw,
        application: *mut MiApplicationRaw,
    ) -> MiResult;
}

/// RAII guard over the MI infrastructure: `MI_Application_Initialize` +
/// `MI_Application_NewSession` (local WMI, NULL protocol/destination), both
/// closed on drop. The session field is declared FIRST so it is dropped
/// before the application closes (all sessions must be closed before
/// `MI_Application_Close` completes).
pub struct MiConnection {
    session: MiSessionRaw,
    application: MiApplicationRaw,
}

impl MiConnection {
    /// First instance of `class` in `namespace`, enumerated over MI
    /// (`MI_Session_EnumerateInstances`, keysOnly = FALSE). The instance is
    /// the binding target for `invoke` — the PowerShell `-InputObject`
    /// equivalent; class-level invocation is rejected by the Acer provider on
    /// the AN16S-61 (ticket 16), so instance-bound is the only path.
    pub fn enumerate_first_instance(&self, namespace: &str, class: &str) -> Result<MiInstance, MiError> {
        let namespace_wide = wide(namespace);
        let class_wide = wide(class);
        let mut operation = MiOperationRaw { reserved1: 0, reserved2: 0, ft: null() };
        unsafe {
            ((*self.session.ft).enumerate_instances)(
                core::ptr::from_ref(&self.session).cast_mut(),
                0,
                null_mut(),
                namespace_wide.as_ptr(),
                class_wide.as_ptr(),
                0,
                null_mut(),
                &mut operation,
            );
        }
        let mut operation = MiOperation::from_raw(operation);
        let mut first: Option<MiInstance> = None;
        loop {
            let (instance, more, result, message) = operation.get_instance_sync()?;
            if !instance.is_null() && first.is_none() {
                first = Some(instance_clone(instance)?);
            }
            if !more {
                if result != MI_RESULT_OK {
                    return Err(MiError { result, op: "enumerate_first_instance", message });
                }
                break;
            }
        }
        first.ok_or(MiError {
            result: MI_RESULT_NOT_FOUND,
            op: "enumerate_first_instance",
            message: Some(format!("no instance of {class} in {namespace}")),
        })
    }

    /// Invoke `method` on `instance` with the `input` parameters bag
    /// (`MI_Session_Invoke` with className = NULL — instance-bound, the
    /// documented PowerShell shape; the inbound element names must match the
    /// method's parameter names). The out-params instance (ReturnValue + out
    /// parameters) is cloned and returned when the provider produced one.
    pub fn invoke(
        &self,
        namespace: &str,
        instance: &MiInstance,
        method: &str,
        input: &MiInstance,
    ) -> Result<Option<MiInstance>, MiError> {
        let namespace = wide(namespace);
        let method = wide(method);
        let mut operation = MiOperationRaw { reserved1: 0, reserved2: 0, ft: null() };
        unsafe {
            ((*self.session.ft).invoke)(
                core::ptr::from_ref(&self.session).cast_mut(),
                0,
                null_mut(),
                namespace.as_ptr(),
                null(),
                method.as_ptr(),
                instance.raw(),
                input.raw(),
                null_mut(),
                &mut operation,
            );
        }
        let mut operation = MiOperation::from_raw(operation);
        let mut last: Option<MiInstance> = None;
        loop {
            let (result_instance, more, result, message) = operation.get_instance_sync()?;
            if !result_instance.is_null() {
                last = Some(instance_clone(result_instance)?);
            }
            if !more {
                if result != MI_RESULT_OK {
                    return Err(MiError { result, op: "invoke", message });
                }
                break;
            }
        }
        Ok(last)
    }

    /// A new dynamic instance of `class` (for method input bags; element
    /// names must match the method's parameter names).
    pub fn new_instance(&self, class: &str) -> Result<MiInstance, MiError> {
        let class = wide(class);
        let mut instance: *mut MiInstanceRaw = null_mut();
        let result = unsafe {
            ((*self.application.ft).new_instance)(
                core::ptr::from_ref(&self.application).cast_mut(),
                class.as_ptr(),
                null(),
                &mut instance,
            )
        };
        if result != MI_RESULT_OK || instance.is_null() {
            return Err(MiError { result, op: "MI_Application_NewInstance", message: None });
        }
        Ok(unsafe { MiInstance::from_raw(instance) })
    }
}

impl Drop for MiConnection {
    fn drop(&mut self) {
        if !self.session.ft.is_null() {
            let close = unsafe { (*self.session.ft).close };
            let _ = unsafe { close(&mut self.session, null_mut(), null_mut()) };
        }
        if !self.application.ft.is_null() {
            let close = unsafe { (*self.application.ft).close };
            let _ = unsafe { close(&mut self.application) };
        }
    }
}

/// RAII `MI_Instance`: `MI_Instance_Delete` on drop. Instances from the
/// operation loops are cloned (`MI_Instance_Clone`) so their lifetime no
/// longer depends on the operation.
pub struct MiInstance {
    raw: *mut MiInstanceRaw,
}

impl MiInstance {
    /// Wraps a heap-allocated `MI_Instance`; the wrapper deletes it on drop.
    ///
    /// # Safety
    /// `raw` must be a valid `MI_Instance` pointer created by the MI library
    /// (clone or application new) with no other owner.
    pub unsafe fn from_raw(raw: *mut MiInstanceRaw) -> Self {
        Self { raw }
    }

    /// The raw `MI_Instance` pointer (borrowed).
    pub fn raw(&self) -> *const MiInstanceRaw {
        self.raw
    }

    /// Add a new `u64` element to a dynamic instance (`MI_Instance_AddElement` —
    /// the method input bags have no schema RTTI, so `SetElement` would fail
    /// with `MI_RESULT_NO_SUCH_PROPERTY`).
    pub fn add_u64(&mut self, name: &str, value: u64) -> Result<(), MiError> {
        let value = MiValue { uint64: value };
        self.add_element(name, &value, MI_UINT64)
    }

    /// Add a new `u32` element to a dynamic instance.
    pub fn add_u32(&mut self, name: &str, value: u32) -> Result<(), MiError> {
        let value = MiValue { uint32: value };
        self.add_element(name, &value, MI_UINT32)
    }

    /// Add a new `u8` element to a dynamic instance.
    pub fn add_u8(&mut self, name: &str, value: u8) -> Result<(), MiError> {
        let value = MiValue { uint8: value };
        self.add_element(name, &value, MI_UINT8)
    }

    /// Add a new `u8` array element to a dynamic instance (CIM `MI_UINT8A`);
    /// the bytes are copied by the MI library.
    pub fn add_u8_array(&mut self, name: &str, values: &[u8]) -> Result<(), MiError> {
        let value = MiValue {
            uint8a: MiArray { data: values.as_ptr().cast_mut().cast(), size: values.len() as u32 },
        };
        self.add_element(name, &value, MI_UINT8A)
    }

    fn add_element(&mut self, name: &str, value: &MiValue, ty: MiType) -> Result<(), MiError> {
        let name = wide(name);
        let ft = unsafe { (*self.raw).ft };
        let result = unsafe { ((*ft).add_element)(self.raw, name.as_ptr(), value, ty, 0) };
        if result != MI_RESULT_OK {
            return Err(MiError { result, op: "MI_Instance_AddElement", message: None });
        }
        Ok(())
    }

    /// Read a scalar integer element (CIM `MI_UINT8`/`MI_UINT16`/`MI_UINT32`/
    /// `MI_UINT64` and their signed forms) as `u64`. `Ok(None)` when the
    /// element is absent or null.
    pub fn get_u64(&self, name: &str) -> Result<Option<u64>, MiError> {
        let Some((value, ty, flags)) = self.get_element(name)? else {
            return Ok(None);
        };
        if flags & MI_FLAG_NULL != 0 {
            return Ok(None);
        }
        Ok(coerce_u64(&value, ty))
    }

    /// Read a scalar integer element as `u32` (CIM `MI_UINT8`/`MI_UINT16`/
    /// `MI_UINT32`). `Ok(None)` when absent or null or wider than u32.
    pub fn get_u32(&self, name: &str) -> Result<Option<u32>, MiError> {
        Ok(self.get_u64(name)?.and_then(|value| u32::try_from(value).ok()))
    }

    /// The instance's class name (diagnostics).
    pub fn class_name(&self) -> String {
        let ft = unsafe { (*self.raw).ft };
        let mut name: *const MiChar = null();
        let result = unsafe { ((*ft).get_class_name)(self.raw, &mut name) };
        if result != MI_RESULT_OK || name.is_null() {
            return "?".into();
        }
        unsafe { wide_from_ptr(name) }
    }

    /// Read a `u8` array element (CIM `MI_UINT8A`). `Ok(None)` when absent or
    /// null; the bytes are copied out.
    pub fn get_u8_array(&self, name: &str) -> Result<Option<Vec<u8>>, MiError> {
        let Some((value, ty, flags)) = self.get_element(name)? else {
            return Ok(None);
        };
        if flags & MI_FLAG_NULL != 0 {
            return Ok(None);
        }
        if ty != MI_UINT8A {
            return Err(MiError {
                result: MI_RESULT_TYPE_MISMATCH,
                op: "MI_Instance_GetElement(u8 array)",
                message: Some(format!("expected MI_UINT8A, got {}", type_name(ty))),
            });
        }
        Ok(Some(unsafe { array_to_vec(value.uint8a) }))
    }

    /// `Ok(None)` when the element is absent (`MI_RESULT_NO_SUCH_PROPERTY`).
    fn get_element(&self, name: &str) -> Result<Option<(MiValue, MiType, u32)>, MiError> {
        let name = wide(name);
        let ft = unsafe { (*self.raw).ft };
        let mut value = MiValue { uint64: 0 };
        let mut ty: MiType = 0;
        let mut flags: u32 = 0;
        let result =
            unsafe { ((*ft).get_element)(self.raw, name.as_ptr(), &mut value, &mut ty, &mut flags, null_mut()) };
        if result == MI_RESULT_NO_SUCH_PROPERTY {
            return Ok(None);
        }
        if result != MI_RESULT_OK {
            return Err(MiError { result, op: "MI_Instance_GetElement", message: None });
        }
        Ok(Some((value, ty, flags)))
    }
}

impl Drop for MiInstance {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            let ft = unsafe { (*self.raw).ft };
            if !ft.is_null() {
                let delete = unsafe { (*ft).delete };
                let _ = unsafe { delete(self.raw) };
            }
        }
    }
}

/// RAII `MI_Operation`: `MI_Operation_Close` on drop (synchronous — blocks
/// until the operation is done; a never-started operation has a null FT and
/// is a no-op).
pub struct MiOperation {
    raw: MiOperationRaw,
}

impl MiOperation {
    fn from_raw(raw: MiOperationRaw) -> Self {
        Self { raw }
    }

    /// One synchronous `MI_Operation_GetInstance` pull: blocks until a
    /// result is available. Returns `(instance, more_results, result,
    /// message)`; the instance is borrowed by the operation and invalidated
    /// by the next pull, and `result`/`message` are meaningful on the final
    /// pull.
    fn get_instance_sync(&mut self) -> Result<(*const MiInstanceRaw, bool, MiResult, Option<String>), MiError> {
        if self.raw.ft.is_null() {
            return Err(MiError {
                result: MI_RESULT_INVALID_PARAMETER,
                op: "MI_Operation_GetInstance",
                message: Some("operation was not started".into()),
            });
        }
        let mut instance: *const MiInstanceRaw = null();
        let mut more: MiBoolean = 0;
        let mut result: MiResult = MI_RESULT_OK;
        let mut error_message: *const MiChar = null();
        let mut completion: *const MiInstanceRaw = null();
        let call_result = unsafe {
            ((*self.raw.ft).get_instance)(
                &mut self.raw,
                &mut instance,
                &mut more,
                &mut result,
                &mut error_message,
                &mut completion,
            )
        };
        if call_result != MI_RESULT_OK {
            return Err(MiError { result: call_result, op: "MI_Operation_GetInstance", message: None });
        }
        let message =
            if error_message.is_null() { None } else { Some(unsafe { wide_from_ptr(error_message) }) };
        Ok((instance, more != 0, result, message))
    }
}

impl Drop for MiOperation {
    fn drop(&mut self) {
        if !self.raw.ft.is_null() {
            let close = unsafe { (*self.raw.ft).close };
            let _ = unsafe { close(&mut self.raw) };
        }
    }
}

/// `MI_Instance_Clone` — the cloned instance is owned by the caller.
fn instance_clone(instance: *const MiInstanceRaw) -> Result<MiInstance, MiError> {
    let ft = unsafe { (*instance).ft };
    let mut clone: *mut MiInstanceRaw = null_mut();
    let result = unsafe { ((*ft).clone)(instance, &mut clone) };
    if result != MI_RESULT_OK || clone.is_null() {
        return Err(MiError { result, op: "MI_Instance_Clone", message: None });
    }
    Ok(unsafe { MiInstance::from_raw(clone) })
}

/// Copy an `MI_Uint8A` out into an owned `Vec` (the MI library owns the
/// source memory).
///
/// # Safety
/// `array` must have been produced by the MI library and be valid until the
/// copy completes.
unsafe fn array_to_vec(array: MiArray) -> Vec<u8> {
    unsafe {
        if array.data.is_null() || array.size == 0 {
            return Vec::new();
        }
        core::slice::from_raw_parts(array.data.cast::<u8>(), array.size as usize).to_vec()
    }
}

/// Coerce a scalar `MI_Value` of the given type to `u64` (pure, unit-tested).
pub fn coerce_u64(value: &MiValue, ty: MiType) -> Option<u64> {
    unsafe {
        match ty {
            MI_BOOLEAN => Some(u64::from(value.boolean)),
            MI_UINT8 => Some(u64::from(value.uint8)),
            MI_SINT8 => value.sint8.try_into().ok(),
            MI_UINT16 => Some(u64::from(value.uint16)),
            MI_SINT16 => value.sint16.try_into().ok(),
            MI_UINT32 => Some(u64::from(value.uint32)),
            MI_SINT32 => value.sint32.try_into().ok(),
            MI_UINT64 => Some(value.uint64),
            MI_SINT64 => value.sint64.try_into().ok(),
            _ => None,
        }
    }
}

/// Read a NUL-terminated wide string from `ptr` into an owned `String`.
///
/// # Safety
/// `ptr` must be a valid NUL-terminated wide string for the call duration.
unsafe fn wide_from_ptr(ptr: *const MiChar) -> String {
    unsafe {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(ptr, len))
    }
}

/// Wide NUL-terminated buffer for API calls (mirrors `comwbem::wide`).
pub fn wide(text: &str) -> Vec<MiChar> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// The out-param elements the adapters read, probed by name with the type
/// each method's MOF declares. The real out instance carries more elements
/// (e.g. the readback's `uBatteryNo`/`uFunctionQuery` echoes); only the
/// consumed ones cross the seam — absent elements are skipped.
const OUT_PROBES: &[(&str, OutProbe)] = &[
    ("gmOutput", OutProbe::U64),
    ("ReturnValue", OutProbe::U32),
    ("uReturn", OutProbe::U32),
    ("uFunctionList", OutProbe::U32),
    ("uFunctionStatus", OutProbe::U8Array),
];

enum OutProbe {
    U64,
    U32,
    U8Array,
}

impl MiTransport for MiConnection {
    /// The production transport: `connect()` carries the former inherent
    /// `MiConnection::connect()` body — the transport is the per-platform
    /// seam module now, and the adapter constructors call the trait method —
    /// initializing the MI client and opening a session to the local machine
    /// (`MI_Application_InitializeV1(0, "NitroTray", ...)` +
    /// `MI_Application_NewSession(app, NULL, NULL, ...)`; session creation
    /// does not talk to the server, so reachability is proven by the first
    /// operation). `invoke_first_instance` then invokes `method` on the
    /// first instance of `class` (the `-InputObject` binding target —
    /// class-level invocation is rejected by the Acer provider, ticket 16),
    /// building the dynamic input bag from `input` and pulling the probed
    /// out params into an `MiOutput`. The `MiConnection`/`MiInstance` raw
    /// API is unchanged.
    fn connect() -> Result<Self, MiError> {
        let mut application = MiApplicationRaw { reserved1: 0, reserved2: 0, ft: null() };
        let app_id = wide("NitroTray");
        let result =
            unsafe { MI_Application_InitializeV1(0, app_id.as_ptr(), null_mut(), &mut application) };
        if result != MI_RESULT_OK || application.ft.is_null() {
            return Err(MiError {
                result,
                op: "MI_Application_Initialize",
                message: None,
            });
        }
        let mut session = MiSessionRaw { reserved1: 0, reserved2: 0, ft: null() };
        let result = unsafe {
            ((*application.ft).new_session)(
                &mut application,
                null(),
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut session,
            )
        };
        if result != MI_RESULT_OK || session.ft.is_null() {
            let close = unsafe { (*application.ft).close };
            let _ = unsafe { close(&mut application) };
            return Err(MiError {
                result,
                op: "MI_Application_NewSession",
                message: None,
            });
        }
        Ok(Self { session, application })
    }

    fn invoke_first_instance(
        &self,
        namespace: &str,
        class: &str,
        method: &str,
        input: &MiInput,
    ) -> Result<Option<MiOutput>, MiError> {
        let instance = self.enumerate_first_instance(namespace, class)?;
        let mut input_bag = self.new_instance(&input.class)?;
        for element in &input.elements {
            match &element.value {
                TransportMiValue::U8(value) => input_bag.add_u8(element.name, *value)?,
                TransportMiValue::U32(value) => input_bag.add_u32(element.name, *value)?,
                TransportMiValue::U64(value) => input_bag.add_u64(element.name, *value)?,
                TransportMiValue::U8Array(values) => input_bag.add_u8_array(element.name, values)?,
            }
        }
        let out = self.invoke(namespace, &instance, method, &input_bag)?;
        match out {
            None => Ok(None),
            Some(result) => Ok(Some(MiOutput { elements: out_elements(&result)? })),
        }
    }
}

/// Materialize the probed out params of an out-params instance into owned
/// elements; an absent element is skipped (`MI_Instance_GetElement` answers
/// `NO_SUCH_PROPERTY`), a present one with an unexpected type is an error.
fn out_elements(result: &MiInstance) -> Result<Vec<MiElement>, MiError> {
    let mut elements = Vec::new();
    for (name, probe) in OUT_PROBES {
        match probe {
            OutProbe::U64 => {
                if let Some(value) = result.get_u64(name)? {
                    elements.push(MiElement { name, value: TransportMiValue::U64(value) });
                }
            }
            OutProbe::U32 => {
                if let Some(value) = result.get_u32(name)? {
                    elements.push(MiElement { name, value: TransportMiValue::U32(value) });
                }
            }
            OutProbe::U8Array => {
                if let Some(value) = result.get_u8_array(name)? {
                    elements.push(MiElement { name, value: TransportMiValue::U8Array(value) });
                }
            }
        }
    }
    Ok(elements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mi::MI_STRING;

    #[test]
    fn coerce_u64_reads_every_scalar_shape() {
        assert_eq!(coerce_u64(&MiValue { uint8: 0x73 }, MI_UINT8), Some(0x73));
        assert_eq!(coerce_u64(&MiValue { uint16: 0x0041 }, MI_UINT16), Some(0x41));
        assert_eq!(coerce_u64(&MiValue { uint32: 0x40B }, MI_UINT32), Some(0x40B));
        assert_eq!(coerce_u64(&MiValue { uint64: 0x600 }, MI_UINT64), Some(0x600));
        assert_eq!(coerce_u64(&MiValue { sint32: 7 }, MI_SINT32), Some(7));
        assert_eq!(coerce_u64(&MiValue { boolean: 1 }, MI_BOOLEAN), Some(1));
        assert_eq!(coerce_u64(&MiValue { sint32: -1 }, MI_SINT32), None);
        assert_eq!(coerce_u64(&MiValue { uint32: 5 }, MI_STRING), None);
    }

    #[test]
    fn value_union_is_the_c_union_size() {
        assert_eq!(std::mem::size_of::<MiValue>(), 40); // largest member: MI_Datetime
        assert_eq!(std::mem::align_of::<MiValue>(), 8);
        assert_eq!(std::mem::size_of::<MiArray>(), 16);
        assert_eq!(std::mem::size_of::<MiInstanceRaw>(), 64);
        assert_eq!(std::mem::size_of::<MiApplicationRaw>(), 24);
    }

    #[test]
    fn wide_buffers_are_nul_terminated() {
        assert_eq!(wide("ROOT\\WMI"), vec![
            b'R' as u16, b'O' as u16, b'O' as u16, b'T' as u16, b'\\' as u16, b'W' as u16, b'M' as u16,
            b'I' as u16, 0
        ]);
    }
}
