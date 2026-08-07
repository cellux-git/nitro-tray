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
    cfg.reapply
}

/// Timer interval in milliseconds (min 1s; default 30s).
pub fn interval_ms(cfg: &Config) -> u32 {
    (cfg.reapply_interval_secs.max(1).saturating_mul(1000)).min(u32::MAX as u64) as u32
}

/// A tick: firmware-only re-assertion plus eco re-evaluation. Never touches
/// the active plan.
pub fn on_tick(app: &mut AppCore) {
    app.re_evaluate_eco();
    app.reapply_firmware();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(reapply: bool, interval_secs: u64) -> Config {
        Config {
            reapply,
            reapply_interval_secs: interval_secs,
            ..Config::default()
        }
    }

    #[test]
    fn enabled_reflects_config_flag() {
        assert!(!enabled(&Config::default()));
        assert!(enabled(&cfg_with(true, 30)));
    }

    #[test]
    fn interval_ms_default_is_30_seconds() {
        assert_eq!(interval_ms(&Config::default()), 30_000);
        assert_eq!(interval_ms(&cfg_with(true, 30)), 30_000);
    }

    #[test]
    fn interval_ms_clamps_to_minimum_one_second() {
        assert_eq!(interval_ms(&cfg_with(true, 0)), 1_000);
        assert_eq!(interval_ms(&cfg_with(true, 1)), 1_000);
    }

    #[test]
    fn interval_ms_scales_seconds_to_ms() {
        assert_eq!(interval_ms(&cfg_with(true, 5)), 5_000);
    }

    #[test]
    fn interval_ms_saturates_at_u32_max() {
        assert_eq!(interval_ms(&cfg_with(true, u64::MAX)), u32::MAX);
    }
}
