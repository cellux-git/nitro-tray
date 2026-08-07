//! Global hotkey (Ctrl+Alt+P default, configurable): forward-wrap profile
//! cycling with a brief notification. Automatic power-transition switching
//! stays silent — the hotkey is the only notification path.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_DOWN,
    VK_ESCAPE, VK_F1, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

/// Default hotkey spec (config key `hotkey`).
pub const DEFAULT_SPEC: &str = "ctrl-alt-p";

/// The id `WM_HOTKEY` arrives with in `wParam`; shared with the tray window.
pub const HOTKEY_ID: i32 = 0x4E54;

/// Parse a hotkey spec like `"ctrl-alt-p"` / `"shift-alt-x"` into
/// `(modifiers, virtual_key_code)`. Supports `ctrl`, `alt`, `shift`, `win`,
/// and a single key name (`a`-`z`/`A`-`Z`, `0`-`9`, `f1`-`f24`, plus
/// `space`/`enter`/`tab`/`esc`/`escape`/`up`/`down`/`left`/`right`). Pure
/// function, unit-tested. `None` for unparseable specs.
pub fn parse_spec(spec: &str) -> Option<(u32, u32)> {
    let mut mods: u32 = 0;
    let mut vk: Option<u32> = None;
    for raw in spec.split('-') {
        let token = raw.to_ascii_lowercase();
        match token.as_str() {
            "ctrl" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            _ => {
                if vk.is_some() {
                    return None;
                }
                vk = Some(parse_key(&token)?);
            }
        }
    }
    vk.map(|vk| (mods, vk))
}

/// Parse a single key token into its virtual-key code; `None` when unknown.
fn parse_key(token: &str) -> Option<u32> {
    match token {
        "space" => return Some(VK_SPACE as u32),
        "enter" => return Some(VK_RETURN as u32),
        "tab" => return Some(VK_TAB as u32),
        "esc" | "escape" => return Some(VK_ESCAPE as u32),
        "up" => return Some(VK_UP as u32),
        "down" => return Some(VK_DOWN as u32),
        "left" => return Some(VK_LEFT as u32),
        "right" => return Some(VK_RIGHT as u32),
        _ => {}
    }
    if token.len() == 1 {
        let c = token.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
        return None;
    }
    if let Some(num) = token.strip_prefix('f') {
        let n: u32 = num.parse().ok()?;
        if (1..=24).contains(&n) {
            return Some(VK_F1 as u32 + (n - 1));
        }
    }
    None
}

/// Errors from hotkey registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    Parse,
    Register { id: i32 },
}

pub struct Hotkey {
    hwnd: HWND,
    id: i32,
}

impl Hotkey {
    /// Parse + `RegisterHotKey` on the tray window. Fails with `Parse` on an
    /// invalid spec and `Register` when the combination is taken.
    // The HWND is an opaque window handle (not a dereferenceable pointer);
    // it is only stored by user32 for later WM_HOTKEY delivery.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn register(hwnd: HWND, spec: &str) -> Result<Self, HotkeyError> {
        let (mods, vk) = parse_spec(spec).ok_or(HotkeyError::Parse)?;
        let id = HOTKEY_ID;
        unsafe {
            if RegisterHotKey(hwnd, id, mods, vk) == 0 {
                return Err(HotkeyError::Register { id });
            }
        }
        Ok(Hotkey { hwnd, id })
    }

    /// The hotkey id used by `WM_HOTKEY`.
    pub fn id(&self) -> i32 {
        self.id
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        unsafe {
            UnregisterHotKey(self.hwnd, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_parses_to_ctrl_alt_p() {
        assert_eq!(
            parse_spec(DEFAULT_SPEC),
            Some((MOD_CONTROL | MOD_ALT, 'P' as u32))
        );
    }

    #[test]
    fn custom_spec_shift_alt_x() {
        assert_eq!(
            parse_spec("shift-alt-x"),
            Some((MOD_SHIFT | MOD_ALT, 'X' as u32))
        );
    }

    #[test]
    fn modifiers_alone_are_rejected() {
        for spec in ["ctrl", "alt", "shift", "win", "ctrl-alt", "alt-shift-win"] {
            assert_eq!(parse_spec(spec), None, "{spec:?}");
        }
    }

    #[test]
    fn empty_and_blank_specs_are_rejected() {
        for spec in ["", "---", "ctrl--p", "ctrl-alt-", "-"] {
            assert_eq!(parse_spec(spec), None, "{spec:?}");
        }
    }

    #[test]
    fn unknown_key_tokens_are_rejected() {
        for spec in ["ctrl-foo", "ctrl-zx", "ctrl-hyphen", "shift-gr", "ctrl-f25", "ctrl-f0"] {
            assert_eq!(parse_spec(spec), None, "{spec:?}");
        }
    }

    #[test]
    fn bare_key_without_modifiers_is_allowed() {
        assert_eq!(parse_spec("p"), Some((0, 'P' as u32)));
        assert_eq!(parse_spec("a"), Some((0, 'A' as u32)));
        assert_eq!(parse_spec("5"), Some((0, '5' as u32)));
    }

    #[test]
    fn f_keys_parse() {
        assert_eq!(parse_spec("f1"), Some((0, 0x70)));
        assert_eq!(parse_spec("f24"), Some((0, 0x87)));
        assert_eq!(parse_spec("ctrl-f12"), Some((MOD_CONTROL, 0x7B)));
        assert_eq!(parse_spec("win-f24"), Some((MOD_WIN, 0x87)));
    }

    #[test]
    fn digits_parse() {
        assert_eq!(parse_spec("ctrl-0"), Some((MOD_CONTROL, '0' as u32)));
        assert_eq!(parse_spec("ctrl-9"), Some((MOD_CONTROL, '9' as u32)));
        assert_eq!(parse_spec("shift-5"), Some((MOD_SHIFT, '5' as u32)));
    }

    #[test]
    fn uppercase_letters_parse() {
        assert_eq!(
            parse_spec("ctrl-alt-P"),
            Some((MOD_CONTROL | MOD_ALT, 'P' as u32))
        );
        assert_eq!(parse_spec("shift-X"), Some((MOD_SHIFT, 'X' as u32)));
    }

    #[test]
    fn uppercase_tokens_parse() {
        assert_eq!(
            parse_spec("CTRL-ALT-P"),
            Some((MOD_CONTROL | MOD_ALT, 'P' as u32))
        );
        assert_eq!(
            parse_spec("Shift-Alt-F1"),
            Some((MOD_SHIFT | MOD_ALT, 0x70))
        );
    }

    #[test]
    fn named_keys_parse() {
        assert_eq!(parse_spec("space"), Some((0, VK_SPACE as u32)));
        assert_eq!(parse_spec("ctrl-enter"), Some((MOD_CONTROL, VK_RETURN as u32)));
        assert_eq!(parse_spec("alt-tab"), Some((MOD_ALT, VK_TAB as u32)));
        assert_eq!(parse_spec("shift-esc"), Some((MOD_SHIFT, VK_ESCAPE as u32)));
        assert_eq!(parse_spec("shift-escape"), Some((MOD_SHIFT, VK_ESCAPE as u32)));
        assert_eq!(parse_spec("up"), Some((0, VK_UP as u32)));
        assert_eq!(parse_spec("down"), Some((0, VK_DOWN as u32)));
        assert_eq!(parse_spec("left"), Some((0, VK_LEFT as u32)));
        assert_eq!(parse_spec("right"), Some((0, VK_RIGHT as u32)));
        assert_eq!(parse_spec("ctrl-down"), Some((MOD_CONTROL, VK_DOWN as u32)));
    }

    #[test]
    fn multiple_key_tokens_are_rejected() {
        for spec in ["ctrl-p-x", "p-x", "f1-f2", "ctrl-ctrl"] {
            assert_eq!(parse_spec(spec), None, "{spec:?}");
        }
    }
}
