//! Management Infrastructure (MI) — the Acer firmware transport, split by
//! platform (linux-port ticket 02). The per-platform bodies live in
//! `win.rs` (Win32 FFI) and `linux.rs` (Linux stub).
//!
//! Shared, OS-independent surface: the MI result/type codes, `MiError`, and
//! the result/type name tables (used by the `MiTransport` seam, the shared
//! circuit breaker in `adapter.rs`, and the scripted fakes in `testing.rs`).
//!
//! Windows: the in-process MI client — hand-rolled FFI to `mi.dll`
//! (`C:\Windows\System32\mi.dll`), the stack PowerShell's CIM cmdlets use.
//! Replaces the WBEM-COM route (`comwbem`) for the Acer WMI adapters: on the
//! AN16S-61 the WbemCore -> wmiprvse handoff flaps in bad windows, while the
//! MI transport succeeds 100% of the time (ticket 16). Layouts and signatures
//! are transcribed from the Windows SDK 10.0.26100 `um\mi.h` (8-byte packed):
//! `MI_Result`, `MI_Type`, `MI_Value`, the `MI_Application`/`MI_Session`/
//! `MI_Operation`/`MI_Instance` handles and their function tables. Only
//! `MI_Application_Initialize` is imported from the DLL; everything else goes
//! through the function tables, exactly like the C API's inline wrappers do.
//! All operations are synchronous (no callbacks): session operations are
//! started with a NULL `MI_OperationCallbacks` and results are pulled with
//! `MI_Operation_GetInstance`, which blocks until a result is available (the
//! documented synchronous mode). Instances returned by the pulls belong to
//! the operation and are invalidated by the next pull — they are cloned
//! (`MI_Instance_Clone`) before the loop advances. Every `pub unsafe fn`
//! carries a `# Safety` contract (house style, see `comwbem.rs`). Public
//! entry points are safe wrappers returning raw `MI_Result` codes as
//! `MiError`; adapters map those to their own error types.
//!
//! Linux: `MiConnection` is a stub whose `MiTransport` implementation reports
//! "unavailable" — mainline Linux has no generic userspace WMI API (ticket
//! 02); the real Linux transport over the ticket-03 kernel-module chardev
//! lands later and only needs to implement the same `MiTransport` seam.

#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
pub use win::{
    MiApplicationFt, MiApplicationRaw, MiArray, MiConnection, MiDatetime, MiDatetimeU, MiInstance,
    MiInstanceFt, MiInstanceRaw, MiInterval, MiOperation, MiOperationFt, MiOperationRaw,
    MiSessionFt, MiSessionRaw, MiTimestamp, MiValue, coerce_u64, wide,
};
#[cfg(target_os = "linux")]
pub use linux::MiConnection;

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
}
