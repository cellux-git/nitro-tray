//! Scheduled-task lifecycle: the first elevated run installs a logon
//! scheduled task (`NitroTray`, run only when the user is logged on, highest
//! privileges) that launches the exe, giving a silent elevated start at
//! logon. `--uninstall` removes the task. In-process COM only (ITaskService).

use std::path::Path;

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
    let _ = exe_path;
    todo!("ticket 01: implement")
}

/// Remove the scheduled task. Leaves power plans and hardware state untouched.
pub fn uninstall_logon_task() -> Result<(), TaskError> {
    todo!("ticket 01: implement")
}

#[cfg(test)]
mod tests {
    // ticket 01: none OS-independent — covered by on-device verification.
}
