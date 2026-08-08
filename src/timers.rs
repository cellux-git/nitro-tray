//! Windows timer ids, tick intervals, and the reapply-loop config helpers
//! (the former `reapply`/`recovery` modules' constant surface, folded here
//! when their tick bodies moved into `AppCore` — ticket 05). `main.rs` arms
//! the timers on the tray window; `tray.rs` matches the ids to `TrayEvent`s;
//! the tick handling itself lives on `AppCore` (`on_reapply_tick`,
//! `on_recovery_tick`, `reassert_smart_charge`).

use crate::config::Config;

/// Windows timer id used by the reapply loop on the tray window.
pub const REAPPLY_TIMER_ID: usize = 1001;
/// Windows timer id for the recovery tick (30 s).
pub const RECOVERY_TIMER_ID: usize = 1002;
/// Recovery tick interval: reconnect attempts while an adapter is degraded.
pub const RECOVERY_INTERVAL_MS: u32 = 30_000;
/// Windows timer id for the periodic readback tick (60 s).
pub const READBACK_TIMER_ID: usize = 1003;
/// Readback tick interval: targeted state re-reads and tray refresh.
pub const READBACK_INTERVAL_MS: u32 = 60_000;

/// Reapply loop enabled when `config.reapply` is true.
pub fn reapply_enabled(cfg: &Config) -> bool {
    cfg.reapply
}

/// Reapply timer interval in milliseconds (min 1s; default 30s).
pub fn reapply_interval_ms(cfg: &Config) -> u32 {
    (cfg.reapply_interval_secs.max(1).saturating_mul(1000)).min(u32::MAX as u64) as u32
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
        assert!(!reapply_enabled(&Config::default()));
        assert!(reapply_enabled(&cfg_with(true, 30)));
    }

    #[test]
    fn interval_ms_default_is_30_seconds() {
        assert_eq!(reapply_interval_ms(&Config::default()), 30_000);
        assert_eq!(reapply_interval_ms(&cfg_with(true, 30)), 30_000);
    }

    #[test]
    fn interval_ms_clamps_to_minimum_one_second() {
        assert_eq!(reapply_interval_ms(&cfg_with(true, 0)), 1_000);
        assert_eq!(reapply_interval_ms(&cfg_with(true, 1)), 1_000);
    }

    #[test]
    fn interval_ms_scales_seconds_to_ms() {
        assert_eq!(reapply_interval_ms(&cfg_with(true, 5)), 5_000);
    }

    #[test]
    fn interval_ms_saturates_at_u32_max() {
        assert_eq!(reapply_interval_ms(&cfg_with(true, u64::MAX)), u32::MAX);
    }
}
