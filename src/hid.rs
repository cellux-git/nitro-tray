//! Acer HID adapter: feature-report writes on the vendor 0x1025 device for
//! the system usage mode. A HID write failure is never fatal — callers log it
//! and continue with WMI profile + plan.

use crate::policy::HidMode;

/// Acer vendor id.
pub const ACER_VID: u16 = 0x1025;

/// Usage-mode feature report bytes (prior art: 65-byte reports with prefix
/// `A0 00 A0 01 00 01 <mode> 00 00`; Quiet=3, Normal=2, Performance=1).
/// Pure encoding function, unit-tested.
pub fn usage_mode_report(mode: HidMode) -> [u8; 9] {
    let _ = mode;
    todo!("ticket 06: implement")
}

/// Map a read-back usage-mode selector (prior art selectors 1/2/3/6) to a
/// `HidMode`; `None` for unknown selectors. Pure function, unit-tested.
pub fn usage_mode_from_selector(selector: u8) -> Option<HidMode> {
    let _ = selector;
    todo!("ticket 06: implement")
}

/// Errors from the HID layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HidError {
    /// No compatible Acer HID device found.
    NotFound,
    /// A Win32/HID call failed.
    Io { message: String },
}

pub struct HidAdapter {
    // opaque: open handle + preparsed caps for the 0x1025 device
}

impl HidAdapter {
    /// Open the Acer HID device (vendor 0x1025, usage-mode collection).
    pub fn open() -> Result<Self, HidError> {
        todo!("ticket 06: implement")
    }

    /// Write the usage-mode feature report for the given mode.
    pub fn set_usage_mode(&self, mode: HidMode) -> Result<(), HidError> {
        let _ = mode;
        todo!("ticket 06: implement")
    }

    /// Read back the current usage mode.
    pub fn read_usage_mode(&self) -> Result<HidMode, HidError> {
        todo!("ticket 06: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 06: report encoding + selector decoding tests.
}
