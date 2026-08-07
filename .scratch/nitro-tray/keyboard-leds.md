# Keyboard backlight control (Acer gaming WMI) — discovered encodings

On-device-verified discovery for the Nitro 16S AI (AN16S-61, BIOS V1.53),
2026-08-08. Not part of the AeroForge prior-art extraction (AeroForge has no
keyboard code); sources are the community `acer-predator-turbo-and-rgb-keyboard`
Linux module and `rafradek/Acer-Predator-Scripts`, plus live probing.

## Windows WMI surface (root\wmi, class `AcerGamingFunction`)

The class already bound by `src/wmi.rs` — same instance
(`ACPI\PNP0C14\APGe_0`), no new class needed. Method names and declared CIM
parameter types (from the live MOF via `Get-CimClass`):

| Method | gmInput | gmOutput |
|---|---|---|
| `SetGamingKBBacklight` | `UInt8Array` | `UInt32` |
| `GetGamingKBBacklight` | `UInt32` | `UInt8Array` |
| `SetGamingRgbKb` / `GetGamingRgbKb` | `UInt64` / `UInt32` | `UInt32` / `UInt64` |
| `SetGamingLEDBehavior` / `GetGamingLEDBehavior` | `UInt64` / `UInt32` | `UInt32` |
| `SetGamingLEDColor` / `GetGamingLEDColor` | `UInt64` / `UInt32` | `UInt32` |
| `SetGamingLED` / `GetGamingLED` | `UInt8Array` / `UInt32` | `UInt32` / `UInt8Array` |

Linux-side equivalents in the same ACPI WMI device (WMID GUID4
`7A4DDFE7-5B5D-40B4-8595-4408E0CC7F56`): method 20 = set keyboard backlight,
21 = get, 6 = static zone LED (`src/facer.c` of the acer-predator module).

## The 16-byte backlight config (`SetGamingKBBacklight`)

| byte | meaning |
|---|---|
| 0 | effect mode: 0 static, 1 breath, 2 neon, 3 wave, 4 shifting, 5 zoom |
| 1 | animation speed |
| 2 | brightness: 0 = off, 100 = max |
| 3 | 8 for wave mode, else 0 |
| 4 | animation direction |
| 5–7 | R, G, B |
| 8 | 0 |
| 9 | 1 = apply flag (write activates the config) |
| 10–15 | 0 |

**Off payload** (what the app writes): `[0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0]`.

Readback quirk observed on AN16S-61: `GetGamingKBBacklight(1).gmOutput` is a
15-byte array, and bytes 5–6 mirror the per-zone color state
(`GetGamingRgbKb`), i.e. the color fields in the backlight config are not
owned by `SetGamingKBBacklight`. Verified: writing the off payload turns the
backlight off; the effect/timeout state does not revert afterwards.

## Other observed methods

- `SetGamingRgbKb(zone-bitmask | color)` — per-zone static color; zone
  bitmask 1/2/4/8 for zones 1–4 (readback on AN16S-61: `0x00329000`).
- `SetGamingLEDBehavior(value)` / `SetGamingLEDColor(value)` — effect
  behavior and color selectors (community scripts OR the value with 1).
- `SetGamingLED(byte-array)` — zone enable control (the Linux module sends
  `8 | 15<<40` as a u64 to light all four zones; untested on AN16S-61).

## LED auto-off timeout — class `APGeAction` (root\wmi)

`SetFunction(u64)` / `GetFunction(u64)` -> `uiOutput`:

| Call | Effect |
|---|---|
| `SetFunction(0x88402)` | disable the auto-off timeout |
| `SetFunction(0x1E0000088402)` | 30-second timeout (0x1E = 30 in the top byte) |
| `GetFunction(0x88401)` = `0x80000` | timeout currently disabled |
| `GetFunction(0x88401)` = `0x1E0000080000` | timeout currently 30 s |

AN16S-61 shipped with the timeout disabled (0x80000), i.e. the backlight
stays on indefinitely — the reason the LEDs are "always disturbing". The app
currently only ever writes brightness-off via `SetGamingKBBacklight`; the
timeout path is documented for future use.

## Verification status

- `SetGamingKBBacklight` off payload: verified live on AN16S-61 (BIOS V1.53),
  2026-08-08 — LEDs went off, no rebound.
- Method names/types: verified via `Get-CimClass` on the same machine.
- Encodings for mode/speed/color (bytes 0–7) and `SetGamingRgbKb`/
  `LEDBehavior`/`LEDColor`: from community sources, not re-verified visually
  (the app never writes them).
