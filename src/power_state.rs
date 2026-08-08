//! Power-state source, split by platform (linux-port ticket 02):
//!
//! Windows: `GetSystemPowerStatus` (AC/battery, battery %), event-driven via
//! power notifications handled by the tray window, with a slow poll fallback.
//!
//! Linux: `/sys/class/power_supply` — the AC supply's `online` file and the
//! battery's `capacity` file. The pure mapping from the raw values to the
//! snapshot shape is unit-tested on both platforms.

#[cfg(windows)]
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::policy::PowerState;

/// One read of the system power status.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PowerStateSnapshot {
    pub state: PowerState,
    pub percent: u8,
}

/// Interval of the slow poll fallback (ms). The tray window runs a timer at
/// this rate and forwards `PowerChanged` when the snapshot differs.
pub const SLOW_POLL_MS: u32 = 10_000;

/// Read the current system power status via `GetSystemPowerStatus`.
///
/// `BatteryLifePercent` of 255 means "unknown" (no battery, e.g. a desktop or
/// a removed battery) and is mapped to 0 so the tray never shows a bogus 255%.
/// A failed call likewise degrades to a safe snapshot.
#[cfg(windows)]
pub fn read() -> PowerStateSnapshot {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return PowerStateSnapshot {
            state: PowerState::Ac,
            percent: 0,
        };
    }
    snapshot_from_status(&status)
}

/// Pure mapping from the raw struct; kept separate so it is unit-testable.
#[cfg(windows)]
fn snapshot_from_status(status: &SYSTEM_POWER_STATUS) -> PowerStateSnapshot {
    let state = if status.ACLineStatus == 1 {
        PowerState::Ac
    } else {
        PowerState::Battery
    };
    let percent = if status.BatteryLifePercent == 255 {
        0
    } else {
        status.BatteryLifePercent
    };
    PowerStateSnapshot { state, percent }
}

/// `/sys/class/power_supply` directory scanned by the Linux reader.
#[cfg(target_os = "linux")]
const POWER_SUPPLY_DIR: &str = "/sys/class/power_supply";
/// AC-supply directory name prefix (AC0 on the AN16S-61; other machines may
/// use `AC`).
#[cfg(target_os = "linux")]
const AC_PREFIX: &str = "AC";
/// Battery directory name prefix (BAT1 on the AN16S-61).
#[cfg(target_os = "linux")]
const BATTERY_PREFIX: &str = "BAT";

/// Read the current power state from sysfs: the AC supply's `online` file
/// (`1` = plugged in) and the battery's `capacity` file (0–100). Missing
/// files degrade to a safe snapshot (battery, 0%), never fail — the app's
/// never-terminal degrade philosophy.
#[cfg(target_os = "linux")]
pub fn read() -> PowerStateSnapshot {
    let online = read_sysfs_value(std::path::Path::new(POWER_SUPPLY_DIR), AC_PREFIX, "online");
    let capacity = read_sysfs_value(std::path::Path::new(POWER_SUPPLY_DIR), BATTERY_PREFIX, "capacity");
    snapshot_from_sysfs(online, capacity)
}

/// Pure mapping from the parsed sysfs values; kept separate so it is
/// unit-testable. `online` `Some(1)` means AC, anything else (absent file,
/// `0`, unparseable) means battery; `capacity` outside 0–100 (absent or
/// bogus) maps to 0.
#[cfg(target_os = "linux")]
fn snapshot_from_sysfs(online: Option<u8>, capacity: Option<u8>) -> PowerStateSnapshot {
    let state = if online == Some(1) {
        PowerState::Ac
    } else {
        PowerState::Battery
    };
    let percent = match capacity {
        Some(value) if value <= 100 => value,
        _ => 0,
    };
    PowerStateSnapshot { state, percent }
}

/// Read one sysfs attribute of the first supply whose directory name starts
/// with `prefix`, scanning the given supply directory. `None` when no supply
/// matches or none has a readable, parseable value — an unreadable/unparseable
/// file falls through to the next matching supply before giving up (callers
/// degrade to defaults). The supply directory is a parameter so this
/// fall-through logic is unit-testable against fixture temp dirs on both
/// platforms; the Linux reader keeps the hardcoded `/sys/class/power_supply`
/// constants.
#[cfg_attr(windows, allow(dead_code))]
fn read_sysfs_value(dir: &std::path::Path, prefix: &str, file: &str) -> Option<u8> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(prefix) {
            continue;
        }
        let path = entry.path().join(file);
        if let Ok(value) = std::fs::read_to_string(path) {
            if let Ok(parsed) = value.trim().parse::<u8>() {
                return Some(parsed);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn status(ac_line: u8, percent: u8) -> SYSTEM_POWER_STATUS {
        SYSTEM_POWER_STATUS {
            ACLineStatus: ac_line,
            BatteryFlag: 0,
            BatteryLifePercent: percent,
            SystemStatusFlag: 0,
            BatteryLifeTime: 0,
            BatteryFullLifeTime: 0,
        }
    }

    #[cfg(windows)]
    #[test]
    fn ac_line_1_maps_to_ac() {
        let snap = snapshot_from_status(&status(1, 87));
        assert_eq!(snap.state, PowerState::Ac);
        assert_eq!(snap.percent, 87);
    }

    #[cfg(windows)]
    #[test]
    fn ac_line_0_maps_to_battery() {
        let snap = snapshot_from_status(&status(0, 42));
        assert_eq!(snap.state, PowerState::Battery);
        assert_eq!(snap.percent, 42);
    }

    #[cfg(windows)]
    #[test]
    fn unknown_ac_line_maps_to_battery() {
        for ac_line in [2, 255] {
            let snap = snapshot_from_status(&status(ac_line, 50));
            assert_eq!(snap.state, PowerState::Battery, "ACLineStatus {ac_line}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn unknown_percent_maps_to_zero() {
        let ac = snapshot_from_status(&status(1, 255));
        assert_eq!(ac.percent, 0);
        assert_eq!(ac.state, PowerState::Ac);
        let battery = snapshot_from_status(&status(0, 255));
        assert_eq!(battery.percent, 0);
        assert_eq!(battery.state, PowerState::Battery);
    }

    #[cfg(windows)]
    #[test]
    fn percent_passthrough_including_zero() {
        for percent in [0, 1, 50, 100] {
            let snap = snapshot_from_status(&status(0, percent));
            assert_eq!(snap.percent, percent);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn online_1_maps_to_ac() {
        let snap = snapshot_from_sysfs(Some(1), Some(87));
        assert_eq!(snap.state, PowerState::Ac);
        assert_eq!(snap.percent, 87);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn online_0_maps_to_battery() {
        let snap = snapshot_from_sysfs(Some(0), Some(42));
        assert_eq!(snap.state, PowerState::Battery);
        assert_eq!(snap.percent, 42);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_online_maps_to_battery() {
        for online in [None, Some(2), Some(255)] {
            let snap = snapshot_from_sysfs(online, Some(50));
            assert_eq!(snap.state, PowerState::Battery, "online {online:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_or_bogus_capacity_maps_to_zero() {
        for capacity in [None, Some(101), Some(255)] {
            let snap = snapshot_from_sysfs(Some(1), capacity);
            assert_eq!(snap.percent, 0, "capacity {capacity:?}");
            assert_eq!(snap.state, PowerState::Ac);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn capacity_passthrough_including_zero() {
        for percent in [0, 1, 50, 100] {
            let snap = snapshot_from_sysfs(Some(0), Some(percent));
            assert_eq!(snap.percent, percent);
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nitro-tray-sysfs-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn remove_dir(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn first_matching_supply_with_readable_value_wins() {
        let dir = temp_dir("first-supply-wins");
        std::fs::create_dir_all(dir.join("AC0")).unwrap();
        std::fs::create_dir_all(dir.join("AC1")).unwrap();
        std::fs::write(dir.join("AC0").join("online"), "1\n").unwrap();
        std::fs::write(dir.join("AC1").join("online"), "0\n").unwrap();
        let value = read_sysfs_value(&dir, "AC", "online");
        remove_dir(&dir);
        assert_eq!(value, Some(1), "AC0 (created first) must win over AC1");
    }

    #[test]
    fn unreadable_value_falls_through_to_next_supply() {
        let dir = temp_dir("unreadable-falls-through");
        std::fs::create_dir_all(dir.join("AC0")).unwrap();
        std::fs::create_dir_all(dir.join("AC1")).unwrap();
        std::fs::write(dir.join("AC1").join("online"), "1\n").unwrap();
        let value = read_sysfs_value(&dir, "AC", "online");
        remove_dir(&dir);
        assert_eq!(value, Some(1), "AC0 has no online file; AC1 must be read");
    }

    #[test]
    fn unparseable_value_falls_through_to_next_supply() {
        let dir = temp_dir("unparseable-falls-through");
        std::fs::create_dir_all(dir.join("AC0")).unwrap();
        std::fs::create_dir_all(dir.join("AC1")).unwrap();
        std::fs::write(dir.join("AC0").join("online"), "abc\n").unwrap();
        std::fs::write(dir.join("AC1").join("online"), "1\n").unwrap();
        let value = read_sysfs_value(&dir, "AC", "online");
        remove_dir(&dir);
        assert_eq!(value, Some(1), "unparseable AC0 must fall through to AC1");
    }

    #[test]
    fn no_matching_supply_returns_none() {
        let dir = temp_dir("no-matching-supply");
        std::fs::create_dir_all(dir.join("BAT1")).unwrap();
        std::fs::write(dir.join("BAT1").join("capacity"), "85\n").unwrap();
        let value = read_sysfs_value(&dir, "AC", "online");
        remove_dir(&dir);
        assert_eq!(value, None);
    }

    #[test]
    fn matching_supply_but_no_such_file_returns_none() {
        let dir = temp_dir("matching-but-no-file");
        std::fs::create_dir_all(dir.join("AC0")).unwrap();
        let value = read_sysfs_value(&dir, "AC", "online");
        remove_dir(&dir);
        assert_eq!(value, None);
    }
}
