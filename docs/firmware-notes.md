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

## Diagnostics

`src/bin/probe_wmi.rs`, `probe_charge.rs`, `probe_charge_read.rs`,
`probe_hid.rs`, `probe_mi.rs`, `probe_power.rs` drive the real transports
directly — the on-device ground truth for any firmware question.
