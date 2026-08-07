//! Smart-charge adapter: in-process control of the 80% charge cap via the
//! `BatteryControl` WMI health-status toggle, using the AMD direct-trust
//! write path for the target SKU class (see
//! `.scratch/nitro-tray/prior-art-aeroforge.md`). No interpreter is spawned.

/// Errors from the smart-charge WMI layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChargeError {
    /// A COM/WMI call failed.
    Com { hr: i32, op: &'static str },
    /// The BatteryControl interface was not available.
    NotAvailable,
    /// Readback returned an unexpected shape.
    Unexpected(String),
}

pub struct SmartChargeAdapter {
    // opaque: IWbemServices for the BatteryControl class
}

impl SmartChargeAdapter {
    /// Connect to the `BatteryControl` WMI class in-process.
    pub fn connect() -> Result<Self, ChargeError> {
        todo!("ticket 07: implement")
    }

    /// Toggle the 80% charge cap via the AMD direct-trust write path
    /// (`SetBatteryHealthControl` with the proven tuple).
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ChargeError> {
        let _ = enabled;
        todo!("ticket 07: implement")
    }

    /// Read back the current smart-charge state (`GetBatteryHealthControlStatus`).
    pub fn is_enabled(&self) -> Result<bool, ChargeError> {
        todo!("ticket 07: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 07: encoding tests for the toggle tuple / status queries.
}
