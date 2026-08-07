//! Debug logging: with `--log`, appends diagnostics to `nitro-tray.log`
//! beside the exe. Without the flag, all calls are no-ops. Never blocks the
//! message pump for long (line-buffered appends).

use std::path::Path;

/// Log filename, beside the exe.
pub const LOG_FILE_NAME: &str = "nitro-tray.log";

/// Enable/disable logging (`--log` flag).
pub fn set_enabled(enabled: bool) {
    let _ = enabled;
    todo!("ticket 01: implement")
}

/// Resolve the log path for a later `set_enabled(true)`.
pub fn init(exe_dir: &Path) {
    let _ = exe_dir;
    todo!("ticket 01: implement")
}

/// Append an info line.
pub fn info(message: impl AsRef<str>) {
    let _ = message;
    todo!("ticket 01: implement")
}

/// Append a warning line.
pub fn warn(message: impl AsRef<str>) {
    let _ = message;
    todo!("ticket 01: implement")
}

/// Append an error line.
pub fn error(message: impl AsRef<str>) {
    let _ = message;
    todo!("ticket 01: implement")
}

#[cfg(test)]
mod tests {
    // ticket 01: enabled/disabled behavior via a temp dir.
}
