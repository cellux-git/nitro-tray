# Firmware notes — AN16S-61

Durable, implementation-verified knowledge about the Acer firmware surface this
app controls. **Source of truth: the implementation** (`src/wmi.rs`,
`src/charge.rs`, `src/hid.rs`, `src/policy.rs` and their tests — scripted fakes
pin every wire shape under `cargo test`). Earlier prior-art notes (for a
different Acer model) caused real bugs when copied from — e.g. the mask-2
smart-charge write that only echoed the byte while the battery charged past 90%
— and are deleted; do not reintroduce encodings that are not in the code.

The firmware facts below are OS-independent: a Linux port (`.scratch/linux`)
reuses the opcode tables and the readback-verified write semantics and swaps
only the transport (WMI → chardev/kernel driver, HID → `/dev/hidraw`).

## Target machine

Single-SKU target: the Acer Nitro AN16S-61. Everything below was confirmed
on-device on that machine; other models may differ (discovery/sweeping is
deliberately absent — a new machine needing it is a config/decision point).

## WMI protocol (`ROOT\WMI`)

- Transport: in-process MI (`mi.dll`) via `src/mi.rs`; no PowerShell/CIM
  interpreter, no COM. Class-level invocation is rejected by the provider —
  every call is bound to the provider-enumerated **first instance** of the
  class (the `-InputObject` shape).
- Input typing is **explicit per method** (`src/transport.rs` `MiValue`), never
  inferred from the method name — the Set-vs-Get name-dispatch heuristic was a
  real bug class (wrong type silently mis-wrote).
- Out params: `gmOutput` is `UInt64`. An out instance **without** `gmOutput` is
  a protocol anomaly — reported as an error, never treated as silent success.

### `AcerGamingFunction` — profile, fan, keyboard backlight

`gmInput` type differs per method (MOF): **Set\* = UInt64, Get\* = UInt32**.
`gmInput` value encoding for settings: `setting | (value << 8)`.

| Operation | Input value | Notes |
|---|---|---|
| `SetGamingMiscSetting` | setting id 0x0B (platform profile) | write profile |
| `GetGamingMiscSetting` | setting id 0x0B | readback decode: second byte wins when nonzero or value > 0xFF, else low byte |
| `SetGamingFanBehavior` | `0x0041_0009` = fan auto | non-auto `0x0082_0009` (unused) |
| `GetGamingFanBehavior` | 0 | raw value readback |
| `SetGamingKBBacklight` | 16-byte UInt8Array | off config: mode 0, brightness 0, byte 9 = apply flag, rest zero |

Platform profile values: quiet 0, balanced 1, performance 4, eco 6 (turbo 5 is
deliberately unmapped — not a user profile). Single source:
`Profile::firmware_value` / `from_firmware_value` in `src/policy.rs`.

### `BatteryControl` — smart charge (80% cap)

- **Write** (`SetBatteryHealthControl`): tuple `(uBatteryNo=1, uFunctionMask=1,
  uFunctionStatus=status, uReservedIn=[0;5])`. Status 1 = cap at 80%, status 0
  = charge to full. Success requires the provider `ReturnValue` present,
  truthy and `< 0x80000000` **and** a readback that matches the requested
  status — a lying or rejected write is an error.
- **Readback** (`GetBatteryHealthControlStatus`): single pair
  `(uBatteryNo=1, uFunctionQuery=1, uReserved=[0;2])`. Live row with the cap in
  effect: `uFunctionList=3`, `uFunctionStatus=[1,0,0,0,0]` — index 0 is the
  health-status byte. Battery 0 returns an empty row; query value is
  irrelevant. **No sweeping, ever.**
- `uFunctionList` bit 1 (`& 2`) does **not** gate charging on this SKU — the
  mask-2 tuple and bit-1 preference from prior art are wrong here.

## HID (`src/hid.rs`)

- Vendor 0x1025; the usage-mode collection is matched by device-path marker
  `hid#1025174b&col01#` (SetupDi enumeration + `HidD_GetAttributes` vendor
  check).
- 65-byte feature reports (report id + 64). Usage-mode prefix
  `A0 00 A0 01 00 01 <mode> 00 00`, rest zero: 1 = Performance, 2 = Normal,
  3 = Quiet (eco drives Quiet).
- Writes retried 4x every 750 ms: right after logon the first write can be
  rejected while Acer services initialize the device (observed on-device).
- **No usage-mode status readback exists** — the status request answers with
  sensor readings; treat HID readback as diagnostic only.

## Reliability model

- Circuit breaker (`src/adapter.rs`): 5 consecutive failures trip the adapter
  dead; every call short-circuits with `NotAvailable` afterwards; a success
  resets the streak.
- Recovery loop reconnects with a fresh transport on success only (the breaker
  is never reset mid-instance); nothing is terminal — enforcement re-runs on
  reconnect and the tray view refreshes by itself.

## Keyboard RGB surface (verified on-device 2026-08-08)

The AN16S-61 has a 4-zone RGB keyboard with its own WMI surface beside the
plain `SetGamingKBBacklight` the app uses. All encodings below were verified by
writes plus readbacks. The readbacks are **stored config, not live state** —
they do not change when effects play or when the EC blinks the keyboard.

- **Zones** (`SetGamingRgbKb` / `GetGamingRgbKb`): per-zone color, four zones
  selected by mask bits 1/2/4/8. Write encoding: `mask | R << 8 | G << 16 |
  B << 24`. The stored zone color on this SKU is `0x329000` = `(R=0x90, G=0x32,
  B=0)` — orange, the same color the keyboard blinks on profile change.
  `GetGamingRgbKb` returns the same `0x329000` for masks 1/2/4/8; masks with
  higher bits (16..128) read back `0x1`, and `0xFF` reads back `0`.
- **Effect mode** (`SetGamingLEDBehavior` / `GetGamingLEDBehavior`): write
  `(mode << 8) | 1` (low bit = apply). Stored mode on this SKU: 1 (static).
  Modes 0..16 are all accepted (return 1), but the readback does not track the
  write.
- **Zone color apply** (`SetGamingLEDColor` / `GetGamingLEDColor`): write with
  the low bit set; low byte 1..8 is the valid range.
- **Set return codes are input-validity indicators, not success flags**: the
  working writes (profile, fan, keyboard-off) return 0; the LED-family methods
  return 1 for valid inputs and 2 for invalid (e.g. low byte 0 or > 8). A
  return of 1 does **not** mean the write applied.
- **`SetGamingKBBacklight`** (the app's keyboard-off path): the 16-byte write
  is `[mode, speed, intensity, direction, 0, R, G, B, 0, apply, ...]`. The
  15-byte readback tracks the intensity byte live (0 → 255 → 0 on write) but
  keeps the stored zone RGB — the array's R/G/B bytes do not overwrite the
  zone colors. Keyboard off = intensity 0 (what `keyboard_led_off` does).
- **`SetGamingLED`** (16-byte array) exists but every tested payload returned
  the invalid code 2; its function on this SKU is unknown. Its `GetGamingLED`
  readback record (15 bytes, `[0,0,0,0,15,213,0,0,...]` on this SKU) never
  changed through any write or the blink — treated as a static SKU descriptor.
- **The orange blink on profile change is the keyboard**, driven by the EC as
  feedback; the lid logo stays constant (see below).

## Lid logo backlight — investigated, not figured out

The lid logo backlight on this SKU is controlled by the EC, but no software
interface to set its state was found (investigated 2026-08-08 on-device).

## Diagnostics

`src/bin/probe_wmi.rs`, `probe_charge.rs`, `probe_charge_read.rs`,
`probe_hid.rs`, `probe_mi.rs`, `probe_power.rs` drive the real transports
directly — the on-device ground truth for any firmware question.
