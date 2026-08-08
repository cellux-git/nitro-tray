//! Acer HID adapter: feature-report writes on the vendor 0x1025 device for
//! the system usage mode. A HID write failure is never fatal — callers log it
//! and continue with WMI profile + plan.
//!
//! Encodings match the proven AeroForge tables (see `docs/firmware-notes.md`):
//! 65-byte feature reports with a 9-byte prefix `A0 00 A0 01 00 01 <mode> 00 00`
//! on the device whose path contains `hid#1025174b&col01#` (VID 0x1025).
//!
//! Platform split (linux-port ticket 02): the encoding helpers, the
//! `HidTransport` seam, and the `HidAdapter` retry logic are OS-independent.
//! Windows: discovery + `RealHidTransport` over SetupDi/`HidD_SetFeature`/
//! `HidD_GetFeature` (in `win.rs`). Linux: `RealHidTransport` is a stub whose
//! seam reports "unavailable" — the real `/dev/hidraw` transport (same
//! 65-byte reports, `HIDIOCSFEATURE`/`HIDIOCGFEATURE`) lands in ticket 04
//! (in `linux.rs`).

#[cfg(windows)]
mod win;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
pub use win::RealHidTransport;
#[cfg(target_os = "linux")]
pub use linux::RealHidTransport;

use crate::policy::HidMode;

/// Acer vendor id.
pub const ACER_VID: u16 = 0x1025;

/// Feature-report length in bytes (report id byte + 64-byte report).
const REPORT_LEN: u32 = 65;

/// `HidD_SetFeature` write attempts and delay between them: the first write
/// right after logon can be rejected while the Acer services initialize the
/// device (observed on the AN16S-61, 2026-08-07).
const WRITE_ATTEMPTS: u32 = 4;
const WRITE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(750);

/// Usage-mode feature report prefix (`A0 00 A0 01 00 01 <mode> 00 00`;
/// Performance=1, Normal=2, Quiet=3). Pure encoding, unit-tested.
pub fn usage_mode_report(mode: HidMode) -> [u8; 9] {
    let mode_byte = match mode {
        HidMode::Performance => 0x01,
        HidMode::Normal => 0x02,
        HidMode::Quiet => 0x03,
    };
    [0xA0, 0x00, 0xA0, 0x01, 0x00, 0x01, mode_byte, 0x00, 0x00]
}

/// Map a mode/selector byte to a `HidMode`; `None` for unknown values. Pure
/// function, unit-tested.
pub fn usage_mode_from_selector(selector: u8) -> Option<HidMode> {
    match selector {
        3 => Some(HidMode::Quiet),
        2 => Some(HidMode::Normal),
        1 => Some(HidMode::Performance),
        _ => None,
    }
}

/// Errors from the HID layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HidError {
    /// No compatible Acer HID device found.
    NotFound,
    /// A Win32/HID call failed.
    Io { message: String },
}

/// The HID transport seam: one 65-byte feature report per call (report id
/// byte + 64-byte report). The retry policy lives in the adapter, not the
/// transport, so tests exercise it through the seam.
pub trait HidTransport {
    /// Write a 65-byte feature report (`HidD_SetFeature`).
    fn set_feature(&self, report: &[u8; 65]) -> Result<(), HidError>;
    /// Read a 65-byte feature report into `buffer` (`HidD_GetFeature`).
    fn get_feature(&self, buffer: &mut [u8; 65]) -> Result<(), HidError>;
}

/// Acer HID adapter over a `HidTransport` seam. Production uses
/// `HidAdapter::open()` (a `RealHidTransport`); tests use
/// `HidAdapter::with_transport(fake)`.
pub struct HidAdapter<T: HidTransport = RealHidTransport> {
    transport: T,
}

impl<T: HidTransport> HidAdapter<T> {
    /// Wrap any `HidTransport` (the test seam).
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }

    /// Write the usage-mode feature report for the given mode
    /// (`HidD_SetFeature`, 65 bytes; prefix `A0 00 A0 01 00 01 <mode> 00 00`,
    /// rest zero). Retried on failure: at logon the Acer services are still
    /// initializing the device and the first write can be rejected (observed
    /// transiently on the AN16S-61, 2026-08-07). Failure is returned as
    /// `HidError::Io` — never fatal.
    pub fn set_usage_mode(&self, mode: HidMode) -> Result<(), HidError> {
        let mut buf = [0u8; REPORT_LEN as usize];
        buf[..9].copy_from_slice(&usage_mode_report(mode));
        let mut attempt = 0;
        loop {
            match self.transport.set_feature(&buf) {
                Ok(()) => return Ok(()),
                Err(_) if attempt + 1 < WRITE_ATTEMPTS => {
                    std::thread::sleep(WRITE_RETRY_DELAY);
                    attempt += 1;
                }
                // The transport reports the device path and Win32 error; the
                // mode context lives here in the adapter (the report byte is
                // an encoding detail the transport does not decode).
                Err(HidError::Io { message }) => {
                    return Err(HidError::Io {
                        message: format!("for {mode:?}: {}", message),
                    });
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Raw 65-byte feature-report readback: sends a feature request with
    /// `[0]=0xA0` (`HidD_GetFeature`) and returns the device's raw response —
    /// the probe's write-path readback. Diagnostic surface: the probe prints
    /// the response hex, the adapter decodes nothing.
    pub fn raw_readback(&self) -> Result<[u8; 65], HidError> {
        let mut buf = [0u8; REPORT_LEN as usize];
        buf[0] = 0xA0;
        self.transport.get_feature(&mut buf)?;
        Ok(buf)
    }

    /// Best-effort usage-mode readback. The device protocol exposes NO
    /// usage-mode status readback; this sends the status request with selector
    /// 1 and decodes the raw u16 as a mode byte only when it exactly matches a
    /// known mode (1/2/3). Any other value means the device answered with a
    /// sensor reading — returned as `HidError::Io`.
    pub fn read_usage_mode(&self) -> Result<HidMode, HidError> {
        let mut buf = [0u8; REPORT_LEN as usize];
        buf[0] = 0xA0;
        buf[2] = 0xA0;
        buf[3] = 0x08;
        buf[5] = 0x02;
        buf[6] = 0x01;
        self.transport.get_feature(&mut buf)?;
        let value = u16::from_le_bytes([buf[8], buf[9]]);
        match usage_mode_from_selector(value as u8) {
            Some(mode) => Ok(mode),
            None => Err(HidError::Io {
                message: format!(
                    "usage-mode readback unsupported by the device protocol (status probe returned {value})"
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHidTransport;

    #[test]
    fn usage_mode_report_matches_prior_art_prefixes() {
        assert_eq!(
            usage_mode_report(HidMode::Performance),
            [0xA0, 0x00, 0xA0, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00]
        );
        assert_eq!(
            usage_mode_report(HidMode::Normal),
            [0xA0, 0x00, 0xA0, 0x01, 0x00, 0x01, 0x02, 0x00, 0x00]
        );
        assert_eq!(
            usage_mode_report(HidMode::Quiet),
            [0xA0, 0x00, 0xA0, 0x01, 0x00, 0x01, 0x03, 0x00, 0x00]
        );
    }

    #[test]
    fn usage_mode_from_selector_maps_known_modes() {
        assert_eq!(usage_mode_from_selector(1), Some(HidMode::Performance));
        assert_eq!(usage_mode_from_selector(2), Some(HidMode::Normal));
        assert_eq!(usage_mode_from_selector(3), Some(HidMode::Quiet));
    }

    #[test]
    fn usage_mode_from_selector_rejects_unknown_values() {
        assert_eq!(usage_mode_from_selector(0), None);
        assert_eq!(usage_mode_from_selector(6), None);
        assert_eq!(usage_mode_from_selector(0xFF), None);
    }

    #[test]
    fn set_usage_mode_sends_the_prior_art_report() {
        let fake = FakeHidTransport::new();
        fake.script_set(vec![Ok(())]);
        let adapter = HidAdapter::with_transport(fake.clone());
        adapter.set_usage_mode(HidMode::Quiet).unwrap();
        let mut expected = [0u8; REPORT_LEN as usize];
        expected[..9].copy_from_slice(&usage_mode_report(HidMode::Quiet));
        assert_eq!(fake.sent_reports(), vec![expected]);
    }

    #[test]
    fn set_usage_mode_retries_after_transient_failures() {
        let fake = FakeHidTransport::new();
        let err = HidError::Io { message: "transient".into() };
        fake.script_set(vec![Err(err.clone()), Err(err), Ok(())]);
        let adapter = HidAdapter::with_transport(fake.clone());
        adapter.set_usage_mode(HidMode::Performance).unwrap();
        assert_eq!(fake.sent_reports().len(), 3);
    }

    #[test]
    fn set_usage_mode_fails_after_four_attempts() {
        let fake = FakeHidTransport::new();
        let err = HidError::Io { message: "persistent".into() };
        fake.script_set(vec![Err(err)]);
        let adapter = HidAdapter::with_transport(fake.clone());
        let result = adapter.set_usage_mode(HidMode::Normal);
        assert!(matches!(&result, Err(HidError::Io { message }) if message.contains("for Normal:")));
        assert_eq!(fake.sent_reports().len(), WRITE_ATTEMPTS as usize);
    }

    #[test]
    fn read_usage_mode_decodes_a_scripted_response() {
        let fake = FakeHidTransport::new();
        let mut payload = [0u8; REPORT_LEN as usize];
        payload[8] = 2;
        fake.script_get(vec![payload]);
        let adapter = HidAdapter::with_transport(fake.clone());
        assert_eq!(adapter.read_usage_mode(), Ok(HidMode::Normal));
    }

    #[test]
    fn raw_readback_returns_the_scripted_feature_payload() {
        let fake = FakeHidTransport::new();
        let mut payload = [0u8; REPORT_LEN as usize];
        payload[8] = 2;
        payload[64] = 0xAB;
        fake.script_get(vec![payload]);
        let adapter = HidAdapter::with_transport(fake.clone());
        assert_eq!(adapter.raw_readback(), Ok(payload));
    }

    #[test]
    fn raw_readback_propagates_transport_errors() {
        let fake = FakeHidTransport::new();
        let adapter = HidAdapter::with_transport(fake.clone());
        assert!(adapter.raw_readback().is_err());
    }
}
