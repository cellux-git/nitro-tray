//! Optional TOML config beside the exe. Every key has a baked-in default; a
//! missing file means defaults, a partial file fills the gaps, and invalid
//! values are rejected gracefully (the app still starts on defaults).

use std::path::Path;

use crate::log;

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

fn bad_value(key: &str, reason: &str, default: impl std::fmt::Display) -> String {
    format!("config: bad value for '{key}' ({reason}), using default {default}")
}

/// Parse TOML contents into a `Config`, filling unspecified keys with
/// defaults. Invalid values are dropped with a diagnostic message appended to
/// the returned `Vec` — parsing never fails.
pub fn parse(contents: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut diagnostics = Vec::new();

    let value: toml::Value = match toml::from_str(contents) {
        Ok(value) => value,
        Err(err) => {
            diagnostics.push(format!("config: malformed TOML: {err}"));
            return (cfg, diagnostics);
        }
    };

    let Some(table) = value.as_table() else {
        diagnostics.push("config: malformed TOML: expected a table".to_string());
        return (cfg, diagnostics);
    };

    if let Some(value) = table.get("smart_charge") {
        match value.as_bool() {
            Some(b) => cfg.smart_charge = b,
            None => diagnostics.push(bad_value("smart_charge", "expected boolean", cfg.smart_charge)),
        }
    }

    if let Some(value) = table.get("ac_profile") {
        match value.as_str() {
            Some(s) => cfg.ac_profile = s.to_string(),
            None => diagnostics.push(bad_value(
                "ac_profile",
                "expected string",
                format!("{:?}", cfg.ac_profile),
            )),
        }
    }

    if let Some(value) = table.get("battery_profile") {
        match value.as_str() {
            Some(s) => cfg.battery_profile = s.to_string(),
            None => diagnostics.push(bad_value(
                "battery_profile",
                "expected string",
                format!("{:?}", cfg.battery_profile),
            )),
        }
    }

    if let Some(value) = table.get("auto_switch") {
        match value.as_bool() {
            Some(b) => cfg.auto_switch = b,
            None => diagnostics.push(bad_value("auto_switch", "expected boolean", cfg.auto_switch)),
        }
    }

    if let Some(value) = table.get("reapply") {
        match value.as_bool() {
            Some(b) => cfg.reapply = b,
            None => diagnostics.push(bad_value("reapply", "expected boolean", cfg.reapply)),
        }
    }

    if let Some(value) = table.get("reapply_interval_secs") {
        match value.as_integer() {
            Some(secs) if secs >= 1 => cfg.reapply_interval_secs = secs as u64,
            Some(_) => diagnostics.push(bad_value(
                "reapply_interval_secs",
                "must be >= 1",
                cfg.reapply_interval_secs,
            )),
            None => diagnostics.push(bad_value(
                "reapply_interval_secs",
                "expected integer",
                cfg.reapply_interval_secs,
            )),
        }
    }

    if let Some(value) = table.get("hotkey") {
        match value.as_str() {
            Some(s) => cfg.hotkey = s.to_string(),
            None => diagnostics.push(bad_value(
                "hotkey",
                "expected string",
                format!("{:?}", cfg.hotkey),
            )),
        }
    }

    (cfg, diagnostics)
}

/// Load config from `exe_dir/nitro-tray.toml`. Missing file -> defaults.
pub fn load(exe_dir: &Path) -> Config {
    let path = exe_dir.join(CONFIG_FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents).0,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(err) => {
            log::warn(format!("config: cannot read {}: {err}", path.display()));
            Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nitro-tray-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn remove_dir(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_string_gives_defaults() {
        let (cfg, diags) = parse("");
        assert_eq!(cfg, Config::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn partial_file_fills_gaps() {
        let (cfg, diags) = parse("hotkey = \"ctrl-alt-shift-x\"");
        assert_eq!(cfg.hotkey, "ctrl-alt-shift-x");
        assert_eq!(cfg, Config {
            hotkey: "ctrl-alt-shift-x".to_string(),
            ..Config::default()
        });
        assert!(diags.is_empty());
    }

    #[test]
    fn full_file_overrides_everything() {
        let (cfg, diags) = parse(
            "smart_charge = false\n\
             ac_profile = \"performance\"\n\
             battery_profile = \"balanced\"\n\
             auto_switch = false\n\
             reapply = true\n\
             reapply_interval_secs = 120\n\
             hotkey = \"ctrl-alt-f\"\n",
        );
        assert_eq!(cfg.smart_charge, false);
        assert_eq!(cfg.ac_profile, "performance");
        assert_eq!(cfg.battery_profile, "balanced");
        assert_eq!(cfg.auto_switch, false);
        assert_eq!(cfg.reapply, true);
        assert_eq!(cfg.reapply_interval_secs, 120);
        assert_eq!(cfg.hotkey, "ctrl-alt-f");
        assert!(diags.is_empty());
    }

    #[test]
    fn wrong_type_keeps_default_and_diagnostic() {
        let (cfg, diags) = parse(
            "smart_charge = \"yes\"\n\
             ac_profile = 42\n\
             battery_profile = true\n\
             auto_switch = 1\n\
             reapply = \"on\"\n\
             reapply_interval_secs = \"30\"\n\
             hotkey = 7\n",
        );
        assert_eq!(cfg, Config::default());
        assert_eq!(diags.len(), 7);
        assert!(diags.iter().any(|d| d.contains("'smart_charge'") && d.contains("expected boolean") && d.contains("default true")));
        assert!(diags.iter().any(|d| d.contains("'ac_profile'") && d.contains("expected string") && d.contains("default \"balanced\"")));
        assert!(diags.iter().any(|d| d.contains("'battery_profile'") && d.contains("expected string") && d.contains("default \"eco\"")));
        assert!(diags.iter().any(|d| d.contains("'auto_switch'") && d.contains("expected boolean") && d.contains("default true")));
        assert!(diags.iter().any(|d| d.contains("'reapply'") && d.contains("expected boolean") && d.contains("default false")));
        assert!(diags.iter().any(|d| d.contains("'reapply_interval_secs'") && d.contains("expected integer") && d.contains("default 30")));
        assert!(diags.iter().any(|d| d.contains("'hotkey'") && d.contains("expected string") && d.contains("default \"ctrl-alt-p\"")));
    }

    #[test]
    fn zero_and_negative_interval_kept_at_default() {
        for bad in ["reapply_interval_secs = 0", "reapply_interval_secs = -5"] {
            let (cfg, diags) = parse(bad);
            assert_eq!(cfg.reapply_interval_secs, 30);
            assert_eq!(diags.len(), 1);
            assert!(diags[0].contains("'reapply_interval_secs'"));
            assert!(diags[0].contains("default 30"));
        }
    }

    #[test]
    fn valid_interval_is_kept() {
        let (cfg, diags) = parse("reapply_interval_secs = 1");
        assert_eq!(cfg.reapply_interval_secs, 1);
        assert!(diags.is_empty());
    }

    #[test]
    fn malformed_toml_gives_defaults_and_diagnostic() {
        let (cfg, diags) = parse("smart_charge = = true");
        assert_eq!(cfg, Config::default());
        assert_eq!(diags.len(), 1);
        assert!(diags[0].contains("malformed TOML"));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let (cfg, diags) = parse("fan_speed = 3\nunknown = \"x\"\n[section]\nkey = 1");
        assert_eq!(cfg, Config::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn load_missing_file_gives_defaults() {
        let dir = temp_dir("load-missing");
        let cfg = load(&dir);
        remove_dir(&dir);
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn load_partial_file_applies_overrides() {
        let dir = temp_dir("load-partial");
        std::fs::write(dir.join(CONFIG_FILE_NAME), "reapply = true\nhotkey = \"ctrl-alt-shift-x\"").unwrap();
        let cfg = load(&dir);
        remove_dir(&dir);
        assert_eq!(cfg.reapply, true);
        assert_eq!(cfg.hotkey, "ctrl-alt-shift-x");
        assert_eq!(cfg, Config {
            reapply: true,
            hotkey: "ctrl-alt-shift-x".to_string(),
            ..Config::default()
        });
    }

    #[test]
    fn load_full_file_applies_all() {
        let dir = temp_dir("load-full");
        std::fs::write(
            dir.join(CONFIG_FILE_NAME),
            "smart_charge = false\nac_profile = \"performance\"\nbattery_profile = \"balanced\"\nauto_switch = false\nreapply = true\nreapply_interval_secs = 45\nhotkey = \"ctrl-alt-g\"",
        )
        .unwrap();
        let cfg = load(&dir);
        remove_dir(&dir);
        assert_eq!(cfg.smart_charge, false);
        assert_eq!(cfg.ac_profile, "performance");
        assert_eq!(cfg.battery_profile, "balanced");
        assert_eq!(cfg.auto_switch, false);
        assert_eq!(cfg.reapply, true);
        assert_eq!(cfg.reapply_interval_secs, 45);
        assert_eq!(cfg.hotkey, "ctrl-alt-g");
    }

    #[test]
    fn load_invalid_values_still_yield_usable_config() {
        let dir = temp_dir("load-invalid");
        std::fs::write(dir.join(CONFIG_FILE_NAME), "reapply_interval_secs = -1\nsmart_charge = \"no\"").unwrap();
        let cfg = load(&dir);
        remove_dir(&dir);
        assert_eq!(cfg, Config::default());
    }
}
