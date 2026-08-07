# 10 — Smart charge toggle

**What to build:** the tray lets the user toggle smart charge (the 80% charge cap) on and off. The toggle applies immediately, updates intent, and the tray's read-back display reflects the change right away.

**Blocked by:** 07 — Smart charge adapter; 08 — Tray effective state.

**Status:** ready-for-agent

- [x] A tray menu item toggles smart charge on and off, applying the change immediately.
- [x] The read-back smart-charge display updates immediately after a toggle.
- [x] The toggle updates the intended state so startup enforcement keeps the user's choice.

## Comments

The toggle logic itself landed in tickets 08/09: tray.rs renders the
"Smart charge (80% cap)" item (check mark from `TrayView.smart_charge`,
greyed via `smart_charge_greyed`) and raises `TrayEvent::ToggleSmartCharge`
(ticket 08); `AppCore::toggle_smart_charge()` flips intent, persists it to
`nitro-tray.state.toml`, and applies immediately (ticket 09). This ticket
wires the menu event to that logic in `src/main.rs`:

- `TrayEvent::ToggleSmartCharge` arm calls `app.toggle_smart_charge()`, logs
  the new intent via `app.smart_charge_intent()`, then
  `tray.update(&view_from(app))` with warn-logging on error (mirrors the
  `SelectProfile` arm).
- Checklist item 1: menu item (08) + toggle (09) + this arm — verified by
  reading, no wiring gaps.
- Checklist item 2: the arm re-pushes the view, which re-reads via
  `app.effective()` (charge adapter readback); on a machine where the charge
  adapter is unavailable the item is greyed and the toggle still flips intent.
- Checklist item 3: `write_state_file` persists `smart_charge` and
  `AppCore::new` reloads it via `load_state` (app.rs) — verified by reading.

On-device verification is still needed for the real adapter round trip
(wmi/hid/charge adapters cannot run in this environment).
