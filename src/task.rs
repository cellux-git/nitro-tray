//! Scheduled-task lifecycle: the "Start at logon" checkbox (persisted in the
//! state file) installs a logon scheduled task (`NitroTray`, run only when
//! the user is logged on, highest privileges) that launches the exe, giving
//! a silent elevated start at logon. Unchecking removes it. In-process COM
//! only (ITaskService).

use std::ops::Deref;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use winapi::shared::winerror::{ERROR_FILE_NOT_FOUND, HRESULT_FROM_WIN32, SUCCEEDED};
use winapi::shared::wtypes::{BSTR, VARIANT_FALSE, VARIANT_TRUE};
use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL};
use winapi::um::oaidl::VARIANT;
use winapi::um::objbase::COINIT_MULTITHREADED;
use winapi::um::oleauto::{SysAllocString, SysFreeString};
use winapi::um::taskschd::{
    IExecAction, IRegisteredTask, ITaskService, TaskScheduler, TASK_ACTION_EXEC,
    TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN, TASK_RUNLEVEL_HIGHEST,
    TASK_TRIGGER_LOGON,
};
use winapi::um::unknwnbase::IUnknown;
use winapi::um::winnt::{HRESULT, LONG};
use winapi::{Class, Interface};

/// Scheduled task name.
pub const TASK_NAME: &str = "NitroTray";

/// Errors from task operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    /// A COM/TaskScheduler call failed.
    Com { hr: i32, op: &'static str },
    /// Could not resolve the exe path.
    Path(String),
}

/// Install the logon scheduled task pointing at `exe_path`. Idempotent
/// (create-or-update). Logon trigger, highest run level, run only when the
/// user is logged on.
pub fn install_logon_task(exe_path: &Path) -> Result<(), TaskError> {
    let exe_wide = path_to_wide(exe_path);
    if exe_wide.len() <= 1 {
        return Err(TaskError::Path(exe_path.display().to_string()));
    }

    let _com = ComInit::new()?;
    let service = create_task_service()?;

    let empty = empty_variant();
    check_hr(
        unsafe { service.Connect(empty, empty, empty, empty) },
        "ITaskService::Connect",
    )?;

    let folder_path = BStr::new(to_wide("\\").as_slice());
    let folder = acquire("ITaskService::GetFolder", |out| unsafe {
        service.GetFolder(folder_path.as_bstr(), out)
    })?;

    let definition = acquire("ITaskService::NewTask", |out| unsafe {
        service.NewTask(0, out)
    })?;

    let registration = acquire("ITaskDefinition::get_RegistrationInfo", |out| unsafe {
        definition.get_RegistrationInfo(out)
    })?;
    let description = BStr::new(to_wide("Nitro Tray - enforce power state at logon").as_slice());
    check_hr(
        unsafe { registration.put_Description(description.as_bstr()) },
        "IRegistrationInfo::put_Description",
    )?;

    let principal = acquire("ITaskDefinition::get_Principal", |out| unsafe {
        definition.get_Principal(out)
    })?;
    check_hr(
        unsafe { principal.put_LogonType(TASK_LOGON_INTERACTIVE_TOKEN) },
        "IPrincipal::put_LogonType",
    )?;
    check_hr(
        unsafe { principal.put_RunLevel(TASK_RUNLEVEL_HIGHEST) },
        "IPrincipal::put_RunLevel",
    )?;

    let settings = acquire("ITaskDefinition::get_Settings", |out| unsafe {
        definition.get_Settings(out)
    })?;
    check_hr(
        unsafe { settings.put_StartWhenAvailable(VARIANT_TRUE) },
        "ITaskSettings::put_StartWhenAvailable",
    )?;
    check_hr(
        unsafe { settings.put_DisallowStartIfOnBatteries(VARIANT_FALSE) },
        "ITaskSettings::put_DisallowStartIfOnBatteries",
    )?;
    check_hr(
        unsafe { settings.put_StopIfGoingOnBatteries(VARIANT_FALSE) },
        "ITaskSettings::put_StopIfGoingOnBatteries",
    )?;
    // "PT0S" = no execution time limit.
    let no_limit = BStr::new(to_wide("PT0S").as_slice());
    check_hr(
        unsafe { settings.put_ExecutionTimeLimit(no_limit.as_bstr()) },
        "ITaskSettings::put_ExecutionTimeLimit",
    )?;

    let triggers = acquire("ITaskDefinition::get_Triggers", |out| unsafe {
        definition.get_Triggers(out)
    })?;
    let _trigger = acquire("ITriggerCollection::Create", |out| unsafe {
        triggers.Create(TASK_TRIGGER_LOGON, out)
    })?;

    let actions = acquire("ITaskDefinition::get_Actions", |out| unsafe {
        definition.get_Actions(out)
    })?;
    let action = acquire("IActionCollection::Create", |out| unsafe {
        actions.Create(TASK_ACTION_EXEC, out)
    })?;
    let action = ComPtr::new(action.into_raw() as *mut IExecAction);
    let exe_bstr = BStr::new(exe_wide.as_slice());
    check_hr(
        unsafe { action.put_Path(exe_bstr.as_bstr()) },
        "IExecAction::put_Path",
    )?;

    let mut raw_task: *mut IRegisteredTask = ptr::null_mut();
    let task_name = BStr::new(to_wide(TASK_NAME).as_slice());
    let hr = unsafe {
        folder.RegisterTaskDefinition(
            task_name.as_bstr(),
            definition.as_ptr(),
            TASK_CREATE_OR_UPDATE as LONG,
            empty,
            empty,
            TASK_LOGON_INTERACTIVE_TOKEN,
            empty,
            &mut raw_task,
        )
    };
    check_hr(hr, "ITaskFolder::RegisterTaskDefinition")?;
    let _task = ComPtr::new(raw_task);

    Ok(())
}

/// Remove the scheduled task. Leaves power plans and hardware state untouched.
pub fn uninstall_logon_task() -> Result<(), TaskError> {
    let _com = ComInit::new()?;
    let service = create_task_service()?;

    let empty = empty_variant();
    check_hr(
        unsafe { service.Connect(empty, empty, empty, empty) },
        "ITaskService::Connect",
    )?;

    let folder_path = BStr::new(to_wide("\\").as_slice());
    let folder = acquire("ITaskService::GetFolder", |out| unsafe {
        service.GetFolder(folder_path.as_bstr(), out)
    })?;

    let task_name = BStr::new(to_wide(TASK_NAME).as_slice());
    let hr = unsafe { folder.DeleteTask(task_name.as_bstr(), 0) };
    if hr == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND) {
        return Ok(()); // task does not exist: nothing to remove
    }
    check_hr(hr, "ITaskFolder::DeleteTask")
}

/// RAII COM apartment: `CoInitializeEx(COINIT_MULTITHREADED)` on construction,
/// `CoUninitialize` on drop only when this call performed the init (S_OK).
struct ComInit(bool);

impl ComInit {
    fn new() -> Result<Self, TaskError> {
        let hr = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_MULTITHREADED) };
        match hr {
            0 => Ok(ComInit(true)),  // S_OK: initialized here
            1 => Ok(ComInit(false)), // S_FALSE: already initialized on this thread
            _ => Err(TaskError::Com { hr, op: "CoInitializeEx" }),
        }
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// RAII COM pointer: calls `IUnknown::Release` on drop.
struct ComPtr<T: Interface>(*mut T);

impl<T: Interface> ComPtr<T> {
    fn new(raw: *mut T) -> Self {
        ComPtr(raw)
    }

    fn as_ptr(&self) -> *const T {
        self.0
    }

    fn into_raw(self) -> *mut T {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }
}

impl<T: Interface> Deref for ComPtr<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.0 }
    }
}

impl<T: Interface> Drop for ComPtr<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let unk = self.0 as *mut IUnknown;
            unsafe { (&*unk).Release() };
        }
    }
}

/// A BSTR that frees itself via `SysFreeString` on drop.
struct BStr(BSTR);

impl BStr {
    /// Allocate a BSTR from a null-terminated wide buffer.
    fn new(wide: &[u16]) -> Self {
        BStr(unsafe { SysAllocString(wide.as_ptr()) })
    }

    fn as_bstr(&self) -> BSTR {
        self.0
    }
}

impl Drop for BStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

/// `CoCreateInstance` the Task Scheduler service.
fn create_task_service() -> Result<ComPtr<ITaskService>, TaskError> {
    let clsid = <TaskScheduler as Class>::uuidof();
    let iid = <ITaskService as Interface>::uuidof();
    let mut raw: *mut ITaskService = ptr::null_mut();
    let hr = unsafe {
        CoCreateInstance(
            &clsid,
            ptr::null_mut(),
            CLSCTX_ALL,
            &iid,
            &mut raw as *mut *mut ITaskService as *mut *mut winapi::ctypes::c_void,
        )
    };
    if !SUCCEEDED(hr) {
        return Err(TaskError::Com { hr, op: "CoCreateInstance" });
    }
    if raw.is_null() {
        return Err(TaskError::Com { hr, op: "CoCreateInstance" });
    }
    Ok(ComPtr::new(raw))
}

/// Run a COM call; map a failed HRESULT to `TaskError::Com`.
fn check_hr(hr: HRESULT, op: &'static str) -> Result<(), TaskError> {
    if SUCCEEDED(hr) {
        Ok(())
    } else {
        Err(TaskError::Com { hr, op })
    }
}

/// Run a COM call with an out-pointer; on success wrap the interface in an
/// RAII `ComPtr`, failing on a null out-pointer.
fn acquire<T: Interface>(
    op: &'static str,
    call: impl FnOnce(*mut *mut T) -> HRESULT,
) -> Result<ComPtr<T>, TaskError> {
    let mut raw: *mut T = ptr::null_mut();
    let hr = call(&mut raw);
    if !SUCCEEDED(hr) {
        return Err(TaskError::Com { hr, op });
    }
    if raw.is_null() {
        return Err(TaskError::Com { hr, op });
    }
    Ok(ComPtr::new(raw))
}

/// A zeroed VARIANT is an empty (VT_EMPTY) variant.
fn empty_variant() -> VARIANT {
    unsafe { std::mem::zeroed() }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    // ticket 01: none OS-independent — covered by on-device verification.
}
