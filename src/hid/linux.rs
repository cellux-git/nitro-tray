use super::{HidAdapter, HidError, HidTransport};

/// Linux stub transport (linux-port ticket 02): reports "unavailable" until
/// the `/dev/hidraw` transport lands (ticket 04). Keeps the same name so
/// `AppCore`'s default generic argument stays platform-agnostic; ticket 04
/// replaces this body only.
pub struct RealHidTransport;

impl HidTransport for RealHidTransport {
    fn set_feature(&self, _report: &[u8; 65]) -> Result<(), HidError> {
        Err(HidError::NotFound)
    }

    fn get_feature(&self, _buffer: &mut [u8; 65]) -> Result<(), HidError> {
        Err(HidError::NotFound)
    }
}

/// Linux stub (linux-port ticket 02): no HID device open on Linux yet — the
/// `/dev/hidraw` discovery (VID 0x1025 across hidraw nodes) lands in ticket
/// 04. `NotFound` makes the entry point log "usage-mode adapter unavailable"
/// exactly like the Windows no-device path.
impl HidAdapter<RealHidTransport> {
    pub fn open() -> Result<Self, HidError> {
        Err(HidError::NotFound)
    }
}
