//! Debug logging: with `--log`, appends diagnostics to `nitro-tray.log`
//! beside the exe. Without the flag, all calls are no-ops. Never blocks the
//! message pump for long (line-buffered appends).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Log filename, beside the exe.
pub const LOG_FILE_NAME: &str = "nitro-tray.log";

/// Global log state; appends serialize on the mutex.
static STATE: Mutex<LogState> = Mutex::new(LogState {
    enabled: false,
    path: None,
});

struct LogState {
    enabled: bool,
    path: Option<PathBuf>,
}

/// Enable/disable logging (`--log` flag).
pub fn set_enabled(enabled: bool) {
    if let Ok(mut state) = STATE.lock() {
        state.enabled = enabled;
    }
}

/// Resolve the log path for a later `set_enabled(true)`.
pub fn init(exe_dir: &Path) {
    if let Ok(mut state) = STATE.lock() {
        state.path = Some(exe_dir.join(LOG_FILE_NAME));
    }
}

/// Append an info line.
pub fn info(message: impl AsRef<str>) {
    write_line("INFO", message.as_ref());
}

/// Append a warning line.
pub fn warn(message: impl AsRef<str>) {
    write_line("WARN", message.as_ref());
}

/// Append an error line.
pub fn error(message: impl AsRef<str>) {
    write_line("ERROR", message.as_ref());
}

fn write_line(level: &str, message: &str) {
    let state = match STATE.lock() {
        Ok(state) => state,
        Err(_) => return, // poisoned mutex: degrade silently
    };
    if !state.enabled {
        return;
    }
    let Some(path) = state.path.as_ref() else {
        return;
    };
    let line = format!("[{}] {level} {message}\n", timestamp());
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(_) => return, // IO error: degrade silently
    };
    let _ = file.write_all(line.as_bytes());
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`, hand-formatted from
/// `SystemTime` (no chrono dependency).
fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert days since the Unix epoch to a `(year, month, day)` civil date
/// (proleptic Gregorian; standard "civil from days" algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let day = day as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The tests share global log state, so they must not interleave.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nitro-tray-log-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn disabled_writes_no_file() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = temp_dir("disabled");
        init(&dir);
        set_enabled(false);
        info("first");
        warn("second");
        error("third");
        assert!(!dir.join(LOG_FILE_NAME).exists(), "no file expected while disabled");
    }

    #[test]
    fn enabled_appends_two_lines() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = temp_dir("enabled");
        init(&dir);
        set_enabled(true);
        info("first line");
        warn("second line");
        let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "two appends produce exactly two lines");
        assert!(lines[0].starts_with('[') && lines[0].contains("Z] INFO first line"));
        assert!(lines[1].contains("WARN second line"));
    }
}
