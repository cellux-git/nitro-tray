//! Recovery loop: a broken adapter is never terminal for the process
//! lifetime. Every 30 s, adapters that failed their circuit breaker or never
//! connected are reconnected with a fresh COM stack; on a successful
//! reconnect enforcement re-runs, so "Hardware unavailable" clears by itself
//! in the tray.
//!
//! A separate once-a-minute readback tick re-reads the targeted state
//! (single profile read, single-pair smart-charge read, plan read) so a quiet
//! session — no events at all — cannot leave the tray view stale.
//!
//! Both timers are always armed; recovery must not depend on the reapply
//! loop, which is off by default.

use crate::app::AppCore;

/// Windows timer id for the recovery tick (30 s).
pub const TIMER_ID: usize = 1002;
/// Recovery tick interval: reconnect attempts while an adapter is degraded.
pub const INTERVAL_MS: u32 = 30_000;
/// Windows timer id for the periodic readback tick (60 s).
pub const READBACK_TIMER_ID: usize = 1003;
/// Readback tick interval: targeted state re-reads and tray refresh.
pub const READBACK_INTERVAL_MS: u32 = 60_000;

/// A recovery tick: reconnect any adapter that is missing or tripped its
/// circuit breaker; when something reconnected, re-evaluate eco acceptance
/// and re-run enforcement. Returns true when the hardware state may have
/// changed, so the caller refreshes the tray view.
pub fn on_tick(app: &mut AppCore) -> bool {
    app.reconnect_unavailable()
}
