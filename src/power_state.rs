//! Power-state source: `GetSystemPowerStatus` (AC/battery, battery %),
//! event-driven via power notifications handled by the tray window, with a
//! slow poll fallback.

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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn ac_line_1_maps_to_ac() {
        let snap = snapshot_from_status(&status(1, 87));
        assert_eq!(snap.state, PowerState::Ac);
        assert_eq!(snap.percent, 87);
    }

    #[test]
    fn ac_line_0_maps_to_battery() {
        let snap = snapshot_from_status(&status(0, 42));
        assert_eq!(snap.state, PowerState::Battery);
        assert_eq!(snap.percent, 42);
    }

    #[test]
    fn unknown_ac_line_maps_to_battery() {
        for ac_line in [2, 255] {
            let snap = snapshot_from_status(&status(ac_line, 50));
            assert_eq!(snap.state, PowerState::Battery, "ACLineStatus {ac_line}");
        }
    }

    #[test]
    fn unknown_percent_maps_to_zero() {
        let ac = snapshot_from_status(&status(1, 255));
        assert_eq!(ac.percent, 0);
        assert_eq!(ac.state, PowerState::Ac);
        let battery = snapshot_from_status(&status(0, 255));
        assert_eq!(battery.percent, 0);
        assert_eq!(battery.state, PowerState::Battery);
    }

    #[test]
    fn percent_passthrough_including_zero() {
        for percent in [0, 1, 50, 100] {
            let snap = snapshot_from_status(&status(0, percent));
            assert_eq!(snap.percent, percent);
        }
    }
}
