//! Elevated diagnostic probe for the Acer HID usage-mode device (ticket 06).
//!
//! Test-time only — run ON the target laptop from an elevated shell (the
//! manifest requires administrator). Never shipped or run by the app itself.
//!
//! Flow: open the vendor 0x1025 device, read the current usage mode
//! (best-effort), then for each of Quiet/Normal/Performance write the
//! usage-mode report, read back a raw feature report and print the 65-byte
//! response hex, and finally restore Quiet.

use nitro_tray::hid::{HidAdapter, ACER_VID};
use nitro_tray::policy::HidMode;
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HIDD_ATTRIBUTES,
};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};

const REPORT_LEN: u32 = 65;
const DEVICE_PATH_MARKER: &str = "hid#1025174b&col01#";
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

fn main() {
    let adapter = match HidAdapter::open() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("open failed: {e:?}");
            std::process::exit(1);
        }
    };
    println!("device opened");

    println!("current usage mode (best-effort):");
    match adapter.read_usage_mode() {
        Ok(mode) => println!("  {mode:?}"),
        Err(e) => println!("  read failed (expected off-device protocol gap): {e:?}"),
    }

    let raw = match RawDevice::open() {
        Some(r) => r,
        None => {
            eprintln!("raw readback handle open failed");
            std::process::exit(1);
        }
    };

    let modes = [HidMode::Quiet, HidMode::Normal, HidMode::Performance];
    for mode in modes {
        match adapter.set_usage_mode(mode) {
            Ok(()) => println!("set usage mode {mode:?}: ok"),
            Err(e) => {
                println!("set usage mode {mode:?}: FAILED {e:?}");
                continue;
            }
        }
        let response = raw.readback();
        println!("  {mode:?} raw response prefix hex:");
        println!("  {}", hex_line(&response));
    }

    println!("restoring Quiet");
    match adapter.set_usage_mode(HidMode::Quiet) {
        Ok(()) => println!("restore ok"),
        Err(e) => eprintln!("restore FAILED {e:?}"),
    }
}

fn hex_line(buf: &[u8]) -> String {
    buf.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// Direct device handle for raw `HidD_GetFeature` readback (the adapter's
/// handle is private; the probe is a test-time diagnostic, so it re-opens).
struct RawDevice {
    handle: HANDLE,
}

impl RawDevice {
    fn open() -> Option<Self> {
        let mut hid_guid = windows_sys::core::GUID::from_u128(0);
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
            return None;
        }
        let mut found = None;
        let mut index = 0u32;
        loop {
            let mut if_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            let ok = unsafe {
                SetupDiEnumDeviceInterfaces(info_set, core::ptr::null(), &hid_guid, index, &mut if_data)
            };
            if ok == 0 {
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
                Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
                VendorID: 0,
                ProductID: 0,
                VersionNumber: 0,
            };
            let verified = unsafe { HidD_GetAttributes(handle, &mut attrs) } && attrs.VendorID == ACER_VID;
            if verified {
                found = Some(Self { handle });
                break;
            }
            unsafe { CloseHandle(handle) };
        }
        unsafe { SetupDiDestroyDeviceInfoList(info_set) };
        found
    }

    /// Write-path readback (prior art): feature request with `[0]=0xA0`,
    /// 65 bytes; returns the raw response.
    fn readback(&self) -> [u8; 65] {
        let mut buf = [0u8; 65];
        buf[0] = 0xA0;
        let ok = unsafe { HidD_GetFeature(self.handle, buf.as_mut_ptr().cast(), REPORT_LEN) };
        if !ok {
            println!("  HidD_GetFeature failed");
        }
        buf
    }
}

impl Drop for RawDevice {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

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
        (*detail_ptr).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
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
