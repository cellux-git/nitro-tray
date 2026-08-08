//! Shared entry-point wiring (ticket 09 candidate 6): the production adapters
//! are connected across the core's seams here once, and both binaries (the
//! Windows tray app and the Linux stub entrypoint) consume the result. The
//! transports are the per-platform modules; the constructor is shared.

use crate::charge::SmartChargeAdapter;
use crate::hid::HidAdapter;
use crate::log;
use crate::wmi::WmiAdapter;

/// Wire the production adapters across the core's seams; failures degrade to
/// `None` (never crash). The struct defaults pin the production transports
/// (`WmiAdapter<MiConnection>`, `SmartChargeAdapter<MiConnection>`,
/// `HidAdapter<RealHidTransport>`); both entry points consume this.
pub fn connect_adapters() -> (
    Option<WmiAdapter>,
    Option<SmartChargeAdapter>,
    Option<HidAdapter>,
) {
    let wmi = match WmiAdapter::connect() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!(
                "wmi: adapter unavailable; running degraded: {err:?}"
            ));
            None
        }
    };
    let charge = match SmartChargeAdapter::connect() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!("charge: smart-charge adapter unavailable: {err:?}"));
            None
        }
    };
    let hid = match HidAdapter::open() {
        Ok(adapter) => Some(adapter),
        Err(err) => {
            log::warn(format!("hid: usage-mode adapter unavailable: {err:?}"));
            None
        }
    };
    (wmi, charge, hid)
}
