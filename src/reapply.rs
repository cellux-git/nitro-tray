//! Optional periodic re-assertion loop (off by default; interval default 30s,
//! both configurable). Re-asserts only firmware-level items — WMI profile,
//! HID mode, fan auto, smart-charge state — and never the active Windows
//! plan, so manually chosen plans are respected.

use crate::app::AppCore;
use crate::config::Config;

/// Windows timer id used by the reapply loop on the tray window.
pub const TIMER_ID: usize = 1001;

/// Enabled when `config.reapply` is true.
pub fn enabled(cfg: &Config) -> bool {
    let _ = cfg;
    todo!("ticket 13: implement")
}

/// Timer interval in milliseconds (min 1s; default 30s).
pub fn interval_ms(cfg: &Config) -> u32 {
    let _ = cfg;
    todo!("ticket 13: implement")
}

/// A tick: firmware-only re-assertion plus eco re-evaluation. Never touches
/// the active plan.
pub fn on_tick(app: &mut AppCore) {
    let _ = app;
    todo!("ticket 13: implement")
}

#[cfg(test)]
mod tests {
    // ticket 13: enabled/interval_ms behavior from config.
}
