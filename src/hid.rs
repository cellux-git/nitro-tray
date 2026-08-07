//! Acer HID adapter: feature-report writes on the vendor 0x1025 device for
//! the system usage mode. A HID write failure is never fatal — callers log it
//! and continue with WMI profile + plan.
//!
//! Encodings match the proven AeroForge tables (prior art section 2): 65-byte
//! feature reports with a 9-byte prefix `A0 00 A0 01 00 01 <mode> 00 00` on
//! the device whose path contains `hid#1025174b&col01#` (VID 0x1025).

use std::mem::size_of;

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HidD_SetFeature, HIDD_ATTRIBUTES,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};

use crate::policy::HidMode;

/// Acer vendor id.
pub const ACER_VID: u16 = 0x1025;

/// Feature-report length in bytes (report id byte + 64-byte report).
const REPORT_LEN: u32 = 65;

/// Device-path marker for the usage-mode collection (lowercase, prior art).
const DEVICE_PATH_MARKER: &str = "hid#1025174b&col01#";

// windows-sys 0.61 puts `CreateFileW` in `Win32::Storage::FileSystem`
// (feature `Win32_Storage_FileSystem`, not enabled in Cargo.toml — the
// manifest is frozen). Declared here instead; kernel32.lib is linked by
// default on MSVC. The share/disposition constants are stable Win32 values.
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const OPEN_EXISTING: u32 = 0x3;

unsafe extern "system" {
    fn CreateFileW(
        lp_file_name: windows_sys::core::PCWSTR,
        dw_desired_access: u32,
        dw_share_mode: u32,
        lp_security_attributes: *const core::ffi::c_void,
        dw_creation_disposition: u32,
        dw_flags_and_attributes: u32,
        h_template_file: HANDLE,
    ) -> HANDLE;
}

/// Usage-mode feature report prefix (prior art: `A0 00 A0 01 00 01 <mode>
/// 00 00`; Performance=1, Normal=2, Quiet=3). Pure encoding, unit-tested.
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

/// Open handle + device path of the vendor 0x1025 usage-mode collection.
pub struct HidAdapter {
    handle: HANDLE,
    path: String,
}

impl HidAdapter {
    /// Enumerate HID device interfaces (`SetupDiGetClassDevsW` with
    /// `HidD_GetHidGuid`, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE), match the
    /// path marker, open with `CreateFileW(GENERIC_READ|GENERIC_WRITE,
    /// FILE_SHARE_READ|FILE_SHARE_WRITE, OPEN_EXISTING)`, and verify the
    /// vendor id with `HidD_GetAttributes`.
    pub fn open() -> Result<Self, HidError> {
        let mut hid_guid = GUID::from_u128(0);
        unsafe { HidD_GetHidGuid(&mut hid_guid) };

        let info_set = unsafe {
            SetupDiGetClassDevsW(
                &hid_guid,
                core::ptr::null(),
                core::ptr::null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        };
        if info_set == INVALID_HANDLE_VALUE as isize {
            return Err(HidError::NotFound);
        }

        let mut result = Err(HidError::NotFound);
        let mut index = 0u32;
        loop {
            let mut if_data = SP_DEVICE_INTERFACE_DATA::default();
            if_data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            let found = unsafe {
                SetupDiEnumDeviceInterfaces(info_set, core::ptr::null(), &hid_guid, index, &mut if_data)
            };
            if found == 0 {
                break;
            }
            index += 1;

            let Some(path) = (unsafe { device_interface_path(info_set, &if_data) }) else {
                continue;
            };
            if !path.to_lowercase().contains(DEVICE_PATH_MARKER) {
                continue;
            }

            let Some(handle) = (unsafe { open_device(&path) }) else {
                continue;
            };
            let mut attrs = HIDD_ATTRIBUTES {
                Size: size_of::<HIDD_ATTRIBUTES>() as u32,
                VendorID: 0,
                ProductID: 0,
                VersionNumber: 0,
            };
            let verified = unsafe { HidD_GetAttributes(handle, &mut attrs) } && attrs.VendorID == ACER_VID;
            if verified {
                result = Ok(Self { handle, path });
                break;
            }
            unsafe { CloseHandle(handle) };
        }
        unsafe { SetupDiDestroyDeviceInfoList(info_set) };
        result
    }

    /// Write the usage-mode feature report for the given mode
    /// (`HidD_SetFeature`, 65 bytes; prefix `A0 00 A0 01 00 01 <mode> 00 00`,
    /// rest zero). Failure is returned as `HidError::Io` — never fatal.
    pub fn set_usage_mode(&self, mode: HidMode) -> Result<(), HidError> {
        let mut buf = [0u8; REPORT_LEN as usize];
        buf[..9].copy_from_slice(&usage_mode_report(mode));
        let ok = unsafe { HidD_SetFeature(self.handle, buf.as_ptr().cast(), REPORT_LEN) };
        if ok {
            Ok(())
        } else {
            Err(HidError::Io {
                message: format!("HidD_SetFeature failed for {mode:?} on {}", self.path),
            })
        }
    }

    /// Best-effort usage-mode readback. The device protocol exposes NO
    /// usage-mode status (prior art's only status readback is a sensor
    /// probe); this sends the status request with selector 1 and decodes the
    /// raw u16 as a mode byte only when it exactly matches a known mode
    /// (1/2/3). Any other value means the device answered with a sensor
    /// reading — returned as `HidError::Io`.
    pub fn read_usage_mode(&self) -> Result<HidMode, HidError> {
        let mut buf = [0u8; REPORT_LEN as usize];
        buf[0] = 0xA0;
        buf[2] = 0xA0;
        buf[3] = 0x08;
        buf[5] = 0x02;
        buf[6] = 0x01;
        let ok = unsafe { HidD_GetFeature(self.handle, buf.as_mut_ptr().cast(), REPORT_LEN) };
        if !ok {
            return Err(HidError::Io {
                message: "HidD_GetFeature failed".to_owned(),
            });
        }
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

impl Drop for HidAdapter {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// Fetch the device-interface path for one interface. Two-call pattern:
/// first with a null buffer to learn the required size, then into a buffer
/// whose `cbSize` is set to the struct size.
unsafe fn device_interface_path(
    info_set: HDEVINFO,
    if_data: *const SP_DEVICE_INTERFACE_DATA,
) -> Option<String> {
    let mut required = 0u32;
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(info_set, if_data, core::ptr::null_mut(), 0, &mut required, core::ptr::null_mut());
    }
    if required == 0 {
        return None;
    }
    let mut detail = vec![0u8; required as usize];
    let detail_ptr = detail.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    let ok = unsafe {
        (*detail_ptr).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        SetupDiGetDeviceInterfaceDetailW(
            info_set,
            if_data,
            detail_ptr,
            required,
            &mut required,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    let path = unsafe {
        let path_ptr = (*detail_ptr).DevicePath.as_ptr();
        let mut len = 0usize;
        while *path_ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(path_ptr, len))
    };
    Some(path)
}

/// Open a device path with the prior-art share modes. `None` on failure.
unsafe fn open_device(path: &str) -> Option<HANDLE> {
    let mut wide: Vec<u16> = path.encode_utf16().collect();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        None
    } else {
        Some(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
