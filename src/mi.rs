//! In-process MI (Management Infrastructure) client — hand-rolled FFI to
//! `mi.dll` (`C:\Windows\System32\mi.dll`), the stack PowerShell's CIM
//! cmdlets use. Replaces the WBEM-COM route (`comwbem`) for the Acer WMI
//! adapters: on the AN16S-61 the WbemCore -> wmiprvse handoff flaps in bad
//! windows, while the MI transport succeeds 100% of the time (ticket 16).
//!
//! Layouts and signatures are transcribed from the Windows SDK 10.0.26100
//! `um\mi.h` (8-byte packed): `MI_Result`, `MI_Type`, `MI_Value`, the
//! `MI_Application`/`MI_Session`/`MI_Operation`/`MI_Instance` handles and
//! their function tables. Only `MI_Application_Initialize` is imported from
//! the DLL; everything else goes through the function tables, exactly like
//! the C API's inline wrappers do.
//!
//! All operations are synchronous (no callbacks): session operations are
//! started with a NULL `MI_OperationCallbacks` and results are pulled with
//! `MI_Operation_GetInstance`, which blocks until a result is available
//! (the documented synchronous mode). Instances returned by the pulls
//! belong to the operation and are invalidated by the next pull — they are
//! cloned (`MI_Instance_Clone`) before the loop advances.
//!
//! Every `pub unsafe fn` carries a `# Safety` contract (house style, see
//! `comwbem.rs`). Public entry points are safe wrappers returning raw
//! `MI_Result` codes as `MiError`; adapters map those to their own error
//! types.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

/// MI_Char is `wchar_t` (`MI_CHAR_TYPE` defaults to 2 in mi.h).
pub type MiChar = u16;
pub type MiBoolean = u8;
pub type MiUint32 = u32;
/// `typedef enum _MI_Result { ... } MI_Result;` — C enums are `int`.
pub type MiResult = i32;
pub type MiType = i32;

// MI_RESULT_* (mi.h, enum _MI_Result, values 0..28).
pub const MI_RESULT_OK: MiResult = 0;
pub const MI_RESULT_FAILED: MiResult = 1;
pub const MI_RESULT_ACCESS_DENIED: MiResult = 2;
pub const MI_RESULT_INVALID_NAMESPACE: MiResult = 3;
pub const MI_RESULT_INVALID_PARAMETER: MiResult = 4;
pub const MI_RESULT_INVALID_CLASS: MiResult = 5;
pub const MI_RESULT_NOT_FOUND: MiResult = 6;
pub const MI_RESULT_NOT_SUPPORTED: MiResult = 7;
pub const MI_RESULT_CLASS_HAS_CHILDREN: MiResult = 8;
pub const MI_RESULT_CLASS_HAS_INSTANCES: MiResult = 9;
pub const MI_RESULT_INVALID_SUPERCLASS: MiResult = 10;
pub const MI_RESULT_ALREADY_EXISTS: MiResult = 11;
pub const MI_RESULT_NO_SUCH_PROPERTY: MiResult = 12;
pub const MI_RESULT_TYPE_MISMATCH: MiResult = 13;
pub const MI_RESULT_QUERY_LANGUAGE_NOT_SUPPORTED: MiResult = 14;
pub const MI_RESULT_INVALID_QUERY: MiResult = 15;
pub const MI_RESULT_METHOD_NOT_AVAILABLE: MiResult = 16;
pub const MI_RESULT_METHOD_NOT_FOUND: MiResult = 17;
pub const MI_RESULT_NAMESPACE_NOT_EMPTY: MiResult = 20;
pub const MI_RESULT_INVALID_ENUMERATION_CONTEXT: MiResult = 21;
pub const MI_RESULT_INVALID_OPERATION_TIMEOUT: MiResult = 22;
pub const MI_RESULT_PULL_HAS_BEEN_ABANDONED: MiResult = 23;
pub const MI_RESULT_PULL_CANNOT_BE_ABANDONED: MiResult = 24;
pub const MI_RESULT_FILTERED_ENUMERATION_NOT_SUPPORTED: MiResult = 25;
pub const MI_RESULT_CONTINUATION_ON_ERROR_NOT_SUPPORTED: MiResult = 26;
pub const MI_RESULT_SERVER_LIMITS_EXCEEDED: MiResult = 27;
pub const MI_RESULT_SERVER_IS_SHUTTING_DOWN: MiResult = 28;

/// Human-readable name of an `MI_Result` code (log/diagnostic use).
pub fn result_name(result: MiResult) -> &'static str {
    match result {
        MI_RESULT_OK => "MI_RESULT_OK",
        MI_RESULT_FAILED => "MI_RESULT_FAILED",
        MI_RESULT_ACCESS_DENIED => "MI_RESULT_ACCESS_DENIED",
        MI_RESULT_INVALID_NAMESPACE => "MI_RESULT_INVALID_NAMESPACE",
        MI_RESULT_INVALID_PARAMETER => "MI_RESULT_INVALID_PARAMETER",
        MI_RESULT_INVALID_CLASS => "MI_RESULT_INVALID_CLASS",
        MI_RESULT_NOT_FOUND => "MI_RESULT_NOT_FOUND",
        MI_RESULT_NOT_SUPPORTED => "MI_RESULT_NOT_SUPPORTED",
        MI_RESULT_CLASS_HAS_CHILDREN => "MI_RESULT_CLASS_HAS_CHILDREN",
        MI_RESULT_CLASS_HAS_INSTANCES => "MI_RESULT_CLASS_HAS_INSTANCES",
        MI_RESULT_INVALID_SUPERCLASS => "MI_RESULT_INVALID_SUPERCLASS",
        MI_RESULT_ALREADY_EXISTS => "MI_RESULT_ALREADY_EXISTS",
        MI_RESULT_NO_SUCH_PROPERTY => "MI_RESULT_NO_SUCH_PROPERTY",
        MI_RESULT_TYPE_MISMATCH => "MI_RESULT_TYPE_MISMATCH",
        MI_RESULT_QUERY_LANGUAGE_NOT_SUPPORTED => "MI_RESULT_QUERY_LANGUAGE_NOT_SUPPORTED",
        MI_RESULT_INVALID_QUERY => "MI_RESULT_INVALID_QUERY",
        MI_RESULT_METHOD_NOT_AVAILABLE => "MI_RESULT_METHOD_NOT_AVAILABLE",
        MI_RESULT_METHOD_NOT_FOUND => "MI_RESULT_METHOD_NOT_FOUND",
        MI_RESULT_NAMESPACE_NOT_EMPTY => "MI_RESULT_NAMESPACE_NOT_EMPTY",
        MI_RESULT_INVALID_ENUMERATION_CONTEXT => "MI_RESULT_INVALID_ENUMERATION_CONTEXT",
        MI_RESULT_INVALID_OPERATION_TIMEOUT => "MI_RESULT_INVALID_OPERATION_TIMEOUT",
        MI_RESULT_PULL_HAS_BEEN_ABANDONED => "MI_RESULT_PULL_HAS_BEEN_ABANDONED",
        MI_RESULT_PULL_CANNOT_BE_ABANDONED => "MI_RESULT_PULL_CANNOT_BE_ABANDONED",
        MI_RESULT_FILTERED_ENUMERATION_NOT_SUPPORTED => "MI_RESULT_FILTERED_ENUMERATION_NOT_SUPPORTED",
        MI_RESULT_CONTINUATION_ON_ERROR_NOT_SUPPORTED => "MI_RESULT_CONTINUATION_ON_ERROR_NOT_SUPPORTED",
        MI_RESULT_SERVER_LIMITS_EXCEEDED => "MI_RESULT_SERVER_LIMITS_EXCEEDED",
        MI_RESULT_SERVER_IS_SHUTTING_DOWN => "MI_RESULT_SERVER_IS_SHUTTING_DOWN",
        _ => "MI_RESULT_?",
    }
}

// MI_Type (enum _MI_Type, values 0..31; MI_ARRAY = 16 is the array bit).
pub const MI_BOOLEAN: MiType = 0;
pub const MI_UINT8: MiType = 1;
pub const MI_SINT8: MiType = 2;
pub const MI_UINT16: MiType = 3;
pub const MI_SINT16: MiType = 4;
pub const MI_UINT32: MiType = 5;
pub const MI_SINT32: MiType = 6;
pub const MI_UINT64: MiType = 7;
pub const MI_SINT64: MiType = 8;
pub const MI_REAL32: MiType = 9;
pub const MI_REAL64: MiType = 10;
pub const MI_CHAR16: MiType = 11;
pub const MI_DATETIME: MiType = 12;
pub const MI_STRING: MiType = 13;
pub const MI_REFERENCE: MiType = 14;
pub const MI_INSTANCE: MiType = 15;
pub const MI_UINT8A: MiType = 17;
pub const MI_UINT16A: MiType = 19;
pub const MI_UINT32A: MiType = 21;
pub const MI_UINT64A: MiType = 23;
pub const MI_STRINGA: MiType = 29;

/// MI_Type name, for diagnostics.
pub fn type_name(ty: MiType) -> &'static str {
    match ty {
        MI_BOOLEAN => "MI_BOOLEAN",
        MI_UINT8 => "MI_UINT8",
        MI_SINT8 => "MI_SINT8",
        MI_UINT16 => "MI_UINT16",
        MI_SINT16 => "MI_SINT16",
        MI_UINT32 => "MI_UINT32",
        MI_SINT32 => "MI_SINT32",
        MI_UINT64 => "MI_UINT64",
        MI_SINT64 => "MI_SINT64",
        MI_REAL32 => "MI_REAL32",
        MI_REAL64 => "MI_REAL64",
        MI_CHAR16 => "MI_CHAR16",
        MI_DATETIME => "MI_DATETIME",
        MI_STRING => "MI_STRING",
        MI_REFERENCE => "MI_REFERENCE",
        MI_INSTANCE => "MI_INSTANCE",
        MI_UINT8A => "MI_UINT8A",
        MI_UINT16A => "MI_UINT16A",
        MI_UINT32A => "MI_UINT32A",
        MI_UINT64A => "MI_UINT64A",
        MI_STRINGA => "MI_STRINGA",
        _ => "MI_TYPE_?",
    }
}

// MI_FLAG_* (bit flags): only the ones element access needs.
/// Property/parameter has a null value.
pub const MI_FLAG_NULL: u32 = 1 << 29;
/// Value memory is borrowed by the instance (caller keeps it alive).
pub const MI_FLAG_BORROW: u32 = 1 << 30;

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

/// A failed MI call: the raw result code, the operation that failed, and the
/// provider's error message when the transport reported one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiError {
    pub result: MiResult,
    pub op: &'static str,
    pub message: Option<String>,
}

impl std::fmt::Display for MiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.message {
            Some(message) => write!(f, "{} ({}: {message})", result_name(self.result), self.op),
            None => write!(f, "{} ({})", result_name(self.result), self.op),
        }
    }
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
    /// Initialize the MI client and open a session to the local machine
    /// (`MI_Application_InitializeV1(0, "NitroTray", ...)` +
    /// `MI_Application_NewSession(app, NULL, NULL, ...)`). Session creation
    /// does not talk to the server, so this never proves the provider is
    /// reachable — the first operation does.
    pub fn connect() -> Result<Self, MiError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_names_cover_the_codes_adapters_map() {
        assert_eq!(result_name(0), "MI_RESULT_OK");
        assert_eq!(result_name(2), "MI_RESULT_ACCESS_DENIED");
        assert_eq!(result_name(5), "MI_RESULT_INVALID_CLASS");
        assert_eq!(result_name(6), "MI_RESULT_NOT_FOUND");
        assert_eq!(result_name(12), "MI_RESULT_NO_SUCH_PROPERTY");
        assert_eq!(result_name(13), "MI_RESULT_TYPE_MISMATCH");
        assert_eq!(result_name(-1), "MI_RESULT_?");
    }

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
    fn mi_error_displays_result_name_and_op() {
        let err = MiError { result: MI_RESULT_ACCESS_DENIED, op: "test", message: None };
        assert_eq!(err.to_string(), "MI_RESULT_ACCESS_DENIED (test)");
        let err = MiError {
            result: MI_RESULT_FAILED,
            op: "test",
            message: Some("provider says no".into()),
        };
        assert_eq!(err.to_string(), "MI_RESULT_FAILED (test: provider says no)");
    }

    #[test]
    fn wide_buffers_are_nul_terminated() {
        assert_eq!(wide("ROOT\\WMI"), vec![
            b'R' as u16, b'O' as u16, b'O' as u16, b'T' as u16, b'\\' as u16, b'W' as u16, b'M' as u16,
            b'I' as u16, 0
        ]);
    }
}
