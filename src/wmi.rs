//! In-process raw COM/WMI control of the Acer gaming firmware
//! (`AcerGamingFunction` in `ROOT\WMI`, instance `ACPI\PNP0C14\APGe_0`).
//! Opcode/method encodings match the proven AeroForge tables (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No PowerShell/CIM fallback
//! exists — everything is raw COM `ExecMethod`.

/// Acer firmware platform profile values (prior art, spec-confirmed).
pub const PROFILE_QUIET: u32 = 0;
pub const PROFILE_BALANCED: u32 = 1;
pub const PROFILE_PERFORMANCE: u32 = 4;
pub const PROFILE_TURBO: u32 = 5;
pub const PROFILE_ECO: u32 = 6;

/// `SetGamingMiscSetting` setting id for the platform profile (0x0B).
pub const SETTING_PLATFORM_PROFILE: u32 = 0x0B;

/// `SetGamingFanBehavior` value for fan mode AUTO.
pub const FAN_AUTO: u32 = 0x0041_0009;

/// Errors from the WMI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WmiError {
    /// A COM/WMI call failed.
    Com { hr: i32, op: &'static str },
    /// The Acer WMI instance/class was not found (interface unavailable).
    NotAvailable,
    /// An unexpected response shape.
    Unexpected(String),
}

/// Encodes a `SetGamingMiscSetting` request as `(setting, value)` — pure
/// encoding helper, unit-tested against the prior-art table.
pub fn misc_setting_request(setting: u32, value: u32) -> (u32, u32) {
    let _ = (setting, value);
    todo!("ticket 05: implement")
}

/// Encodes a `SetGamingFanBehavior` request value — pure encoding helper,
/// unit-tested (e.g. auto = 0x00410009).
pub fn fan_behavior_request(auto: bool) -> u32 {
    let _ = auto;
    todo!("ticket 05: implement")
}

pub struct WmiAdapter {
    // opaque: IWbemServices for ROOT\WMI
}

impl WmiAdapter {
    /// Connect to `ROOT\WMI` in-process (CoInitializeEx + CoCreateInstance
    /// CLSID_WbemLocator + ConnectServer). Fails with `NotAvailable` when the
    /// Acer WMI interface is unreachable.
    pub fn connect() -> Result<Self, WmiError> {
        todo!("ticket 05: implement")
    }

    /// Set the firmware platform profile (write via `SetGamingMiscSetting`).
    pub fn set_platform_profile(&self, value: u32) -> Result<(), WmiError> {
        let _ = value;
        todo!("ticket 05: implement")
    }

    /// Read back the platform profile (`GetGamingMiscSetting`).
    pub fn get_platform_profile(&self) -> Result<u32, WmiError> {
        todo!("ticket 05: implement")
    }

    /// Set fan behavior to auto (`SetGamingFanBehavior`).
    pub fn set_fan_auto(&self) -> Result<(), WmiError> {
        todo!("ticket 05: implement")
    }

    /// Read back the fan behavior value.
    pub fn get_fan_behavior(&self) -> Result<u32, WmiError> {
        todo!("ticket 05: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 05: opcode/encoding unit tests for the profile and fan tables.
}
