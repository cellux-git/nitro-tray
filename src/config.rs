//! Optional TOML config beside the exe. Every key has a baked-in default; a
//! missing file means defaults, a partial file fills the gaps, and invalid
//! values are rejected gracefully (the app still starts on defaults).

use std::path::Path;

/// Filename of the config file, resolved beside the exe.
pub const CONFIG_FILE_NAME: &str = "nitro-tray.toml";

/// Documented config keys and their baked-in defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Smart charge (80% charge cap) intent; default `true`.
    pub smart_charge: bool,
    /// Default AC profile name; default `"balanced"`.
    pub ac_profile: String,
    /// Default battery profile name; default `"eco"`.
    pub battery_profile: String,
    /// Automatically switch profile on AC <-> battery transitions; default `true`.
    pub auto_switch: bool,
    /// Periodic firmware re-assertion loop; default `false`.
    pub reapply: bool,
    /// Re-assertion interval in seconds; default `30`.
    pub reapply_interval_secs: u64,
    /// Global hotkey spec; default `"ctrl-alt-p"`.
    pub hotkey: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            smart_charge: true,
            ac_profile: "balanced".to_string(),
            battery_profile: "eco".to_string(),
            auto_switch: true,
            reapply: false,
            reapply_interval_secs: 30,
            hotkey: "ctrl-alt-p".to_string(),
        }
    }
}

/// Parse TOML contents into a `Config`, filling unspecified keys with
/// defaults. Invalid values are dropped with a diagnostic message appended to
/// the returned `Vec` — parsing never fails.
pub fn parse(contents: &str) -> (Config, Vec<String>) {
    let _ = contents;
    todo!("ticket 02: implement")
}

/// Load config from `exe_dir/nitro-tray.toml`. Missing file -> defaults.
pub fn load(exe_dir: &Path) -> Config {
    let _ = exe_dir;
    todo!("ticket 02: implement")
}

#[cfg(test)]
mod tests {
    // ticket 02: unit tests for no-file defaults, partial files, invalid values
}
