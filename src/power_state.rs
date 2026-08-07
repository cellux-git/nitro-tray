//! Power-state source: `GetSystemPowerStatus` (AC/battery, battery %),
//! event-driven via power notifications handled by the tray window, with a
//! slow poll fallback.

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
pub fn read() -> PowerStateSnapshot {
    todo!("ticket 08: implement")
}

#[cfg(test)]
mod tests {
    // ticket 08: nothing OS-independent to test here; logic lives in the
    // app core / tray wiring.
}
