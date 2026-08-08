//! Pure tray menu model, shared across platforms (linux-port ticket 09): the
//! view the app pushes after each state change, the neutral menu-item model,
//! the tooltip text, and the id constants that route picks back into the app.
//! No window, no Win32 — `tray.rs` (Windows plumbing) and the ksni tray
//! (ticket 07, Linux) both derive their menus from this. `TrayEvent::SelectPlan`
//! fires only on Windows (the degraded-mode plan section is Windows-only);
//! the Linux tray never emits it.

use crate::app::EffectiveState;
use crate::policy::{profiles_for, PowerState, Profile};

/// Events raised by the tray window (menu picks, power messages, timers).
/// The main loop drains the channel and drives the app core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// The user chose Quit.
    Quit,
    /// The user toggled the "Start at logon" checkbox.
    ToggleLogonTask,
    /// The user picked a profile in the menu.
    SelectProfile(Profile),
    /// Power status changed (WM_POWERBROADCAST or slow poll fallback).
    PowerChanged,
    /// The machine resumed from sleep (WM_POWERBROADCAST).
    Resume,
    /// The global profile-cycle hotkey was pressed (WM_HOTKEY).
    HotkeyPressed,
    /// The reapply timer ticked (WM_TIMER, timers::REAPPLY_TIMER_ID).
    ReapplyTick,
    /// The recovery timer ticked (WM_TIMER, timers::RECOVERY_TIMER_ID):
    /// retry adapters that failed their circuit breaker.
    RecoveryTick,
    /// The periodic readback timer ticked (WM_TIMER, timers::READBACK_TIMER_ID).
    ReadbackTick,
    /// The user picked a Windows plan in the degraded-mode plan section.
    SelectPlan(Profile),
}

/// The view the main loop pushes into the tray after each state change:
/// read-back effective-state facts. The tray derives the menu contents
/// (profile list per power state, greys, degraded "Windows plan" section,
/// smart-charge intent fallback) from these facts in `menu_items`.
#[derive(Clone, Debug)]
pub struct TrayView {
    pub power: PowerState,
    pub percent: u8,
    /// Current effective profile (checked in the menu); `None` when unknown.
    pub profile: Option<Profile>,
    /// Show the degraded "Hardware unavailable" banner. Drives ONLY that
    /// banner line — profile greying and the "Windows plan" section have
    /// their own flags (`profiles_greyed`, `plan_section`).
    pub degraded: bool,
    /// Grey out the profile items. Windows sets it to `degraded`; Linux keeps
    /// the profile section usable (sysfs tuning applies independently of the
    /// firmware seam) and shows the banner via `degraded` alone.
    pub profiles_greyed: bool,
    /// Show the degraded-mode "Windows plan" section (Windows only): the
    /// power state's profiles offered through OS plans.
    pub plan_section: bool,
    /// Grey out just the eco entry (firmware rejected profile 6).
    pub eco_disabled: bool,
    /// Read-back smart-charge state; `None` when unavailable. The menu treats
    /// `None` as the intent — smart charge is always intended on — showing
    /// the "Smart charge (80% cap)" line checked unless the readback says off.
    pub smart_charge: Option<bool>,
    /// Active Windows plan name; `None` when unknown.
    pub plan: Option<String>,
    /// Ephemeral status line at the bottom of the menu: last apply outcome
    /// ("Applied" / "Failed: ..."), shown only until the menu is dismissed.
    /// `None` = no line.
    pub status: Option<String>,
    /// "Start at logon" checkbox state (scheduled task on Windows, `.desktop`
    /// toggle on Linux).
    pub start_at_logon: bool,
}

/// One neutral menu item, backend-agnostic: Windows appends the `MF_*` /
/// `MFT_*` flag bits computed from `separator`/`enabled`/`checked` at append
/// time (tray.rs `append_flags`); ksni (ticket 07) maps the neutral flags
/// natively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// Routing id (`AppendMenuW` `uID`); 0 for non-routable items.
    pub id: usize,
    /// Item text; empty for separators.
    pub label: String,
    /// Separator line (rendered as such; label and the other flags ignored).
    pub separator: bool,
    /// Usable? Disabled items are greyed by the backends.
    pub enabled: bool,
    /// Checkmark/radio selection state.
    pub checked: bool,
}

/// Menu id of the first profile item. The routing contract between the model
/// and the Windows `open_menu`: ids are 0-based offsets into `profiles_for`.
pub const MENU_PROFILE_BASE: usize = 1;
/// Menu id of the first degraded-mode "Windows plan" item.
pub const MENU_PLAN_BASE: usize = 300;
/// Menu id of the Quit item.
pub const MENU_QUIT: usize = 200;
/// Menu id of the "Start at logon" checkbox item.
pub const MENU_LOGON_TASK: usize = 400;

/// Label of the smart-charge status line. The Windows append mapping keys
/// its one special case (disabled but never greyed, see `append_flags`) on
/// this label — the neutral model cannot distinguish that line from the
/// other disabled lines (banner, plan line, status line).
pub const SMART_CHARGE_LABEL: &str = "Smart charge (80% cap)";

/// The degraded-mode "Windows plan" section: the power state's profiles when
/// the plan section is shown (Windows degraded mode), else empty. Single
/// derivation — the menu model and the id routing both go through this, so
/// the plan ids cannot drift apart.
pub fn plans_for(view: &TrayView) -> &'static [Profile] {
    if view.plan_section {
        profiles_for(view.power)
    } else {
        &[]
    }
}

/// Derive the full popup-menu model from the effective-state facts: the
/// logon checkbox, the degraded "Hardware unavailable" banner, the profile
/// items for the current power state (radio, checked for the effective
/// profile, greyed when `profiles_greyed` or the eco entry is
/// firmware-rejected), the smart-charge intent-fallback status line, the
/// plan line, the degraded-mode "Windows plan" section, Quit, and the
/// ephemeral status line. Pure — no window, no Win32 — so the menu is
/// unit-testable directly on both platforms. Radio semantics (checkmark
/// group vs. plain check) are a backend concern; the model only expresses
/// enabled/checked.
pub fn menu_items(view: &TrayView) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let logon_label = if view.start_at_logon {
        "\u{2611} Start at logon" // ☑
    } else {
        "\u{2610} Start at logon" // ☐
    };
    items.push(entry(MENU_LOGON_TASK, logon_label, true, false));
    items.push(separator());
    if view.degraded {
        items.push(entry(0, "Hardware unavailable", false, false));
        items.push(separator());
    }
    for (i, profile) in profiles_for(view.power).iter().enumerate() {
        let mut enabled = !view.profiles_greyed;
        if view.eco_disabled && *profile == Profile::Eco {
            enabled = false;
        }
        items.push(entry(
            MENU_PROFILE_BASE + i,
            profile_label(*profile),
            enabled,
            Some(*profile) == view.profile,
        ));
    }
    items.push(separator());
    // Smart charge is always intended on and cannot be disabled in the app;
    // the item is a static status line (checked unless the readback says the
    // cap is not in effect — a `None` readback means the intent holds).
    items.push(entry(0, SMART_CHARGE_LABEL, false, view.smart_charge != Some(false)));
    if let Some(plan) = &view.plan {
        items.push(entry(0, &format!("Plan: {plan}"), false, false));
    }
    // Degraded mode: the firmware profile section is unusable, so the
    // "Windows plan" section offers the same profiles through OS plans.
    let plans: &'static [Profile] = plans_for(view);
    if !plans.is_empty() {
        items.push(separator());
        items.push(entry(0, "Windows plan", false, false));
        for (i, profile) in plans.iter().enumerate() {
            items.push(entry(
                MENU_PLAN_BASE + i,
                profile_label(*profile),
                true,
                Some(*profile) == view.profile,
            ));
        }
    }
    items.push(separator());
    items.push(entry(MENU_QUIT, "Quit", true, false));
    // Ephemeral status line: the last apply outcome; cleared on dismissal
    // (no history is kept).
    if let Some(status) = &view.status {
        items.push(separator());
        items.push(entry(0, status, false, false));
    }
    items
}

fn separator() -> MenuItem {
    MenuItem {
        id: 0,
        label: String::new(),
        separator: true,
        enabled: false,
        checked: false,
    }
}

fn entry(id: usize, label: &str, enabled: bool, checked: bool) -> MenuItem {
    MenuItem {
        id,
        label: label.to_string(),
        separator: false,
        enabled,
        checked,
    }
}

/// The display name of a profile (menu items and the tooltip share this).
pub fn profile_label(profile: Profile) -> &'static str {
    match profile {
        Profile::Quiet => "Quiet",
        Profile::Balanced => "Balanced",
        Profile::Performance => "Performance",
        Profile::Eco => "Eco",
    }
}

/// The tooltip text: first line is the state summary, second line the plan and
/// smart-charge state. `\n\n\n` is the standard two-line tooltip trick.
pub fn tooltip_text(view: &TrayView) -> String {
    let status = if view.degraded {
        "Hardware unavailable"
    } else {
        "Nitro Tray"
    };
    let power = match view.power {
        PowerState::Ac => "AC",
        PowerState::Battery => "Battery",
    };
    let profile = view.profile.map(|p| p.as_str()).unwrap_or("unknown");
    let smart = match view.smart_charge {
        Some(true) => "on",
        Some(false) => "off",
        None => "unknown",
    };
    let plan = view.plan.as_deref().unwrap_or("unknown");
    format!(
        "{status} — {percent}% | {profile} | {power}\n\n\n{plan} | Smart charge {smart}",
        percent = view.percent,
    )
}

/// Build the tray view from the app's read-back effective state — the shared
/// core of the Windows `view_from`, incl. the plan-name fallback (when the
/// firmware readback is unavailable, the active Windows plan is still
/// OS-truth and identifies the profile in effect). Pure function of the
/// read-back facts: `effective()` is called once by the caller, not here.
/// Windows passes `degraded, degraded` for `profiles_greyed, plan_section`
/// (menu byte-identical to the pre-ticket build); Linux passes `false, false`
/// — sysfs tuning applies independently of the firmware seam, and the
/// "Hardware unavailable" banner still shows when the firmware is down
/// (driven by `degraded` alone).
pub fn view_from(
    e: &EffectiveState,
    profiles_greyed: bool,
    plan_section: bool,
    start_at_logon: bool,
) -> TrayView {
    TrayView {
        power: e.power,
        percent: e.percent,
        // Read-back firmware profile; when WMI can't report it, the active
        // Windows plan is still OS-truth and identifies the profile in effect.
        profile: e
            .profile
            .or_else(|| e.plan.as_deref().and_then(Profile::from_plan_name)),
        degraded: !e.wmi_available,
        profiles_greyed,
        plan_section,
        eco_disabled: e.eco_disabled,
        // Raw read-back; the tray applies the "always intended on" fallback
        // for the smart-charge menu check mark when the adapter can't report.
        smart_charge: e.smart_charge,
        plan: e.plan.clone(),
        // The status line is set by the user-action handlers only.
        status: None,
        start_at_logon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(power: PowerState, percent: u8, profile: Option<Profile>, plan: Option<&str>) -> TrayView {
        TrayView {
            power,
            percent,
            profile,
            degraded: false,
            profiles_greyed: false,
            plan_section: false,
            eco_disabled: false,
            smart_charge: None,
            plan: plan.map(String::from),
            status: None,
            start_at_logon: false,
        }
    }

    #[test]
    fn tooltip_contains_all_read_back_values() {
        let v = TrayView {
            power: PowerState::Ac,
            percent: 87,
            profile: Some(Profile::Balanced),
            degraded: false,
            profiles_greyed: false,
            plan_section: false,
            eco_disabled: false,
            smart_charge: Some(true),
            plan: Some("Nitro-Balanced".to_string()),
            status: None,
            start_at_logon: false,
        };
        let text = tooltip_text(&v);
        assert!(text.contains("Nitro Tray"));
        assert!(text.contains("87%"));
        assert!(text.contains("balanced"));
        assert!(text.contains("AC"));
        assert!(text.contains("Nitro-Balanced"));
        assert!(text.contains("Smart charge on"));
        assert!(text.contains("\n\n\n"));
    }

    #[test]
    fn tooltip_battery_power_state() {
        let v = view(PowerState::Battery, 42, Some(Profile::Eco), Some("Nitro-Eco"));
        let text = tooltip_text(&v);
        assert!(text.contains("Battery"));
        assert!(text.contains("eco"));
        assert!(text.contains("Nitro-Eco"));
    }

    #[test]
    fn tooltip_smart_charge_off() {
        let mut v = view(PowerState::Ac, 50, None, None);
        v.smart_charge = Some(false);
        assert!(tooltip_text(&v).contains("Smart charge off"));
    }

    #[test]
    fn tooltip_unknowns_and_degraded() {
        let mut v = view(PowerState::Battery, 255, None, None);
        v.degraded = true;
        let text = tooltip_text(&v);
        assert!(text.starts_with("Hardware unavailable"));
        assert!(text.contains("unknown"));
    }

    #[test]
    /// Byte-identical to the pre-ticket Windows menu after the append-time
    /// flag mapping (tray.rs `append_flags`): same ids, labels, order, and
    /// enabled/checked state, item for item.
    fn menu_exact_normal_ac_view() {
        let v = TrayView {
            power: PowerState::Ac,
            percent: 87,
            profile: Some(Profile::Balanced),
            degraded: false,
            profiles_greyed: false,
            plan_section: false,
            eco_disabled: false,
            smart_charge: Some(true),
            plan: Some("Nitro-Balanced".to_string()),
            status: None,
            start_at_logon: false,
        };
        assert_eq!(
            menu_items(&v),
            vec![
                entry(MENU_LOGON_TASK, "\u{2610} Start at logon", true, false),
                separator(),
                entry(MENU_PROFILE_BASE, "Quiet", true, false),
                entry(MENU_PROFILE_BASE + 1, "Balanced", true, true),
                entry(MENU_PROFILE_BASE + 2, "Performance", true, false),
                separator(),
                entry(0, "Smart charge (80% cap)", false, true),
                entry(0, "Plan: Nitro-Balanced", false, false),
                separator(),
                entry(MENU_QUIT, "Quit", true, false),
            ]
        );
    }

    #[test]
    /// Byte-identical to the pre-ticket Windows menu after the append-time
    /// flag mapping (tray.rs `append_flags`): same ids, labels, order, and
    /// enabled/checked state, item for item.
    fn menu_exact_degraded_battery_view() {
        let v = TrayView {
            power: PowerState::Battery,
            percent: 42,
            profile: Some(Profile::Eco),
            degraded: true,
            profiles_greyed: true,
            plan_section: true,
            eco_disabled: false,
            smart_charge: None,
            plan: Some("Nitro-Eco".to_string()),
            status: Some("Applied".to_string()),
            start_at_logon: true,
        };
        assert_eq!(
            menu_items(&v),
            vec![
                entry(MENU_LOGON_TASK, "\u{2611} Start at logon", true, false),
                separator(),
                entry(0, "Hardware unavailable", false, false),
                separator(),
                entry(MENU_PROFILE_BASE, "Eco", false, true),
                entry(MENU_PROFILE_BASE + 1, "Balanced", false, false),
                separator(),
                entry(0, "Smart charge (80% cap)", false, true),
                entry(0, "Plan: Nitro-Eco", false, false),
                separator(),
                entry(0, "Windows plan", false, false),
                entry(MENU_PLAN_BASE, "Eco", true, true),
                entry(MENU_PLAN_BASE + 1, "Balanced", true, false),
                separator(),
                entry(MENU_QUIT, "Quit", true, false),
                separator(),
                entry(0, "Applied", false, false),
            ]
        );
    }

    #[test]
    fn menu_smart_charge_checked_unless_readback_says_off() {
        for (readback, expect_checked) in [(None, true), (Some(true), true), (Some(false), false)] {
            let mut v = view(PowerState::Ac, 50, None, None);
            v.smart_charge = readback;
            let item = menu_items(&v)
                .into_iter()
                .find(|i| i.label == "Smart charge (80% cap)")
                .expect("smart-charge line present");
            assert_eq!(item.id, 0, "readback {readback:?}");
            assert!(!item.enabled, "readback {readback:?}");
            assert_eq!(item.checked, expect_checked, "readback {readback:?}");
        }
    }

    #[test]
    fn menu_eco_disabled_greys_eco_entry_only_when_present() {
        let mut battery = view(PowerState::Battery, 40, Some(Profile::Eco), None);
        battery.eco_disabled = true;
        let items = menu_items(&battery);
        let eco = items.iter().find(|i| i.label == "Eco").expect("eco offered on battery");
        assert!(!eco.enabled);
        let balanced = items.iter().find(|i| i.label == "Balanced").expect("balanced offered");
        assert!(balanced.enabled, "only eco is greyed");

        let mut ac = view(PowerState::Ac, 60, Some(Profile::Quiet), None);
        ac.eco_disabled = true;
        let items = menu_items(&ac);
        assert!(
            items.iter().all(|i| i.label != "Eco"),
            "eco is not offered on AC"
        );
        for item in items {
            assert!(
                item.separator || item.enabled || item.checked,
                "no greyed item on AC with eco_disabled: {item:?}"
            );
        }
    }

    #[test]
    fn menu_plan_line_only_when_plan_known() {
        let mut v = view(PowerState::Ac, 50, Some(Profile::Quiet), Some("Nitro-Quiet"));
        assert!(menu_items(&v).iter().any(|i| i.label == "Plan: Nitro-Quiet"));
        v.plan = None;
        assert!(!menu_items(&v).iter().any(|i| i.label.starts_with("Plan: ")));
    }

    #[test]
    fn menu_status_line_preceded_by_separator() {
        let mut v = view(PowerState::Ac, 50, None, None);
        assert!(!menu_items(&v).iter().any(|i| i.label == "Applied"));
        v.status = Some("Applied".to_string());
        let items = menu_items(&v);
        let idx = items.iter().position(|i| i.label == "Applied").expect("status line present");
        assert!(items[idx - 1].separator);
        assert_eq!(items[idx].id, 0);
        assert!(!items[idx].enabled);
    }

    #[test]
    fn menu_logon_glyph_follows_flag() {
        let mut v = view(PowerState::Ac, 50, None, None);
        v.start_at_logon = false;
        assert_eq!(
            menu_items(&v)[0],
            entry(MENU_LOGON_TASK, "\u{2610} Start at logon", true, false)
        );
        v.start_at_logon = true;
        assert_eq!(
            menu_items(&v)[0],
            entry(MENU_LOGON_TASK, "\u{2611} Start at logon", true, false)
        );
    }

    #[test]
    fn menu_quit_is_last_item() {
        let v = view(PowerState::Ac, 50, None, None);
        let items = menu_items(&v);
        assert_eq!(
            items[items.len() - 1],
            entry(MENU_QUIT, "Quit", true, false)
        );
    }

    #[test]
    fn menu_unknown_profile_checks_no_profile_item() {
        let v = view(PowerState::Ac, 50, None, None);
        for item in menu_items(&v) {
            let is_profile = ["Quiet", "Balanced", "Performance", "Eco"].contains(&item.label.as_str());
            if is_profile {
                assert!(!item.checked, "no checkmark expected in {item:?}");
            }
        }
    }
}
