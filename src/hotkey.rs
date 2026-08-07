//! Global hotkey (Ctrl+Alt+P default, configurable): forward-wrap profile
//! cycling with a brief notification. Automatic power-transition switching
//! stays silent — the hotkey is the only notification path.

use windows_sys::Win32::Foundation::HWND;

/// Default hotkey spec (config key `hotkey`).
pub const DEFAULT_SPEC: &str = "ctrl-alt-p";

/// Parse a hotkey spec like `"ctrl-alt-p"` / `"shift-alt-x"` into
/// `(modifiers, virtual_key_code)`. Supports `ctrl`, `alt`, `shift`, `win`,
/// and a single key name (`a`-`z`, `0`-`9`, `f1`-`f24`). Pure function,
/// unit-tested. `None` for unparseable specs.
pub fn parse_spec(spec: &str) -> Option<(u32, u32)> {
    let _ = spec;
    todo!("ticket 11: implement")
}

/// Errors from hotkey registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    Parse,
    Register { id: i32 },
}

pub struct Hotkey {
    // opaque
}

impl Hotkey {
    /// Parse + `RegisterHotKey` on the tray window. Fails with `Parse` on an
    /// invalid spec and `Register` when the combination is taken.
    pub fn register(hwnd: HWND, spec: &str) -> Result<Self, HotkeyError> {
        let _ = (hwnd, spec);
        todo!("ticket 11: implement")
    }

    /// The hotkey id used by `WM_HOTKEY`.
    pub fn id(&self) -> i32 {
        todo!("ticket 11: implement")
    }
}

#[cfg(test)]
mod tests {
    // ticket 11: parse_spec cases (default, custom, invalid).
}
