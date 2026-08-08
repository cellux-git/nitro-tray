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
    CloseHandle, GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};

use super::{ACER_VID, HidAdapter, HidError, HidTransport, REPORT_LEN};

/// Device-path marker for the usage-mode collection (lowercase).
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

/// Open handle + device path of the vendor 0x1025 usage-mode collection.
///
/// Discovery (SetupDi enumeration + `CreateFileW` + vendor verification)
/// stays in `open()`; the seam is the transport: `RealHidTransport` wraps
/// the opened handle in production, a scripted fake satisfies it in tests
/// (`HidAdapter<T>` is generic over the transport).
pub struct RealHidTransport {
    handle: HANDLE,
    path: String,
}

impl Drop for RealHidTransport {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

impl HidTransport for RealHidTransport {
    fn set_feature(&self, report: &[u8; 65]) -> Result<(), HidError> {
        let ok = unsafe { HidD_SetFeature(self.handle, report.as_ptr().cast(), REPORT_LEN) };
        if ok {
            Ok(())
        } else {
            Err(HidError::Io {
                message: format!(
                    "HidD_SetFeature failed on {} (Win32 error {})",
                    self.path,
                    unsafe { GetLastError() }
                ),
            })
        }
    }

    fn get_feature(&self, buffer: &mut [u8; 65]) -> Result<(), HidError> {
        let ok = unsafe { HidD_GetFeature(self.handle, buffer.as_mut_ptr().cast(), REPORT_LEN) };
        if ok {
            Ok(())
        } else {
            Err(HidError::Io { message: "HidD_GetFeature failed".to_owned() })
        }
    }
}

impl HidAdapter<RealHidTransport> {
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
            let mut if_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
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
                result = Ok(Self { transport: RealHidTransport { handle, path } });
                break;
            }
            unsafe { CloseHandle(handle) };
        }
        unsafe { SetupDiDestroyDeviceInfoList(info_set) };
        result
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

/// Open a device path with the documented share modes. `None` on failure.
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
