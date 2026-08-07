# 07 — Smart charge adapter

**What to build:** in-process control of the 80% charge cap via the `BatteryControl` WMI health-status toggle, using the AMD direct-trust write path for this SKU class, with readback of the current state. No interpreter is spawned. Verified on-device with a probe.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [x] Smart charge (80% cap) can be toggled on and off via in-process COM/WMI `BatteryControl`.
- [x] The current smart-charge state can be read back.
- [x] Uses the AMD direct-trust write path for the target SKU class.
- [ ] On-device probe verification exercises a real toggle + readback round trip.
- [x] No PowerShell or other interpreter is ever spawned.

## Comments

Implemented in `src/charge.rs` + `src/bin/probe_charge.rs` (ticket 07 wave).

- `connect()`: `CoInitializeEx(COINIT_MULTITHREADED)` (tolerates
  `RPC_E_CHANGED_MODE`), `CoCreateInstance(CLSID_WBEM_LOCATOR)`,
  `ConnectServer(ROOT\WMI)`, `GetObject(BatteryControl)`. Any connect-stage
  failure maps to `NotAvailable`. COM is initialized once per connect and
  never uninitialized (process-lifetime adapter, like AeroForge).
- `set_enabled()`: status = `enabled as u8`; attempts `direct_trust_tuple`
  (battery 1, mask 1, 5-zero reserved) first, then the simplified fallback
  list (0,1), (1,2), (0,2); 250 ms sleep between attempts; first successful
  `ExecMethod` wins, no readback verification. Success = `ExecMethod` S_OK +
  provider `ReturnValue` truthy and not an error code
  (`rv != 0 && rv < 0x80000000`), matching prior art's `if ($setAnv.ReturnValue)`.
  All-fail maps to `ChargeError::Com`/`Unexpected`.
- `is_enabled()`: sweeps `uBatteryNo 0..=4` x `uFunctionQuery 0..=6`; per-query
  provider rejections are skipped (prior-art sweep semantics); rows decoded by
  the pure `desired_status_from_rows` helper (prefer `uFunctionList & 2` row's
  `uFunctionStatus[1]`, else any 0/1 byte, else max non-255 byte). `1 => true`.
- Encoding: `Put` uses `VT_UI1` variants with `CIM_UINT8` for the scalar
  params and `VT_ARRAY|VT_UI1` SAFEARRAYs for `uReservedIn[5]` /
  `uReserved[2]`. Out params read from the method output object
  (`ReturnValue`, `uFunctionList` as scalar; `uFunctionStatus` as byte array).
- `ExecMethod` is invoked on the object path of the first `BatteryControl`
  instance (enumerated via `CreateInstanceEnum` + `__PATH`), mirroring prior
  art's `Get-CimInstance` first-instance binding.
- windows-sys 0.61 has WMI constants but no `IWbem*` interfaces and no
  `VARIANT`/`SafeArrayCreate` (feature not enabled, Cargo.toml is not ours to
  edit), so this module hand-declares the canonical vtables
  (`IWbemLocator`/`IWbemServices`/`IWbemClassObject`/`IEnumWbemClassObject`),
  a minimal C-layout `VARIANT`, and `SafeArrayCreate` (oleaut32 extern).
  `CoSetProxyBlanket` is skipped: it needs `Win32::System::Rpc` types that are
  not enabled, and the elevated manifest makes local WMI default security
  sufficient.
- Tests (7, all passing via `cargo test --lib charge::`): tuple encodings for
  `direct_trust_tuple`/`fallback_tuples`, `desired_status_from_rows` decode
  preferences, `method_succeeded` ReturnValue semantics. `cargo build
  --all-targets` is clean; no warnings in owned files.

Needs on-device verification:
- Probe round trip: `target\debug\probe_charge.exe` (elevated) reads current
  state, toggles ON, reads back, toggles OFF, reads back, restores original.
- Confirm the first-instance `__PATH` binding and that `ExecMethod` succeeds
  on this SKU with the direct-trust tuple (battery 1, mask 1).
- Confirm out-param variant types (`uFunctionList`/`uFunctionStatus`) match
  the expected `VT_UI4` scalar + `VT_ARRAY|VT_UI1` byte array shapes.
- The readback sweep uses `IEnumWbemClassObject::Next` timeout 0; if the
  provider ever returns an empty first fetch, switch to `WBEM_INFINITE`.

## Comments (code review)

2026-08-07: Review fixes: (1) a SetBatteryHealthControl attempt now only counts as success when the provider's ReturnValue is present, truthy and not an error code (prior-art if ($setAnv.ReturnValue) semantics; missing ReturnValue => attempt failed); (2) fallback tuple list extended with mask-3 variants (1,3)/(0,3) per prior-art 3.3; (3) COM plumbing extracted into the shared src/comwbem.rs module (canonical vtables, one definition - previously duplicated in wmi.rs/charge.rs with drifted vtable tails); (4) CoUninitialize now happens exactly once when the apartment guard owns the init. Documented deviation: the SetBatteryFunctionData mask sweep (prior-art 3.3) is not ported - it is a fallback for unknown SKUs and the direct-trust path is the target path for this SKU; noted on device if the sweep is ever needed.

## Comments (crash debugging + hardening 2026-08-07, late)

Profile-selection crash (AV in oleaut32 at SafeArrayDestroy+0x2A) root-caused and fixed in the shared comwbem/charge layer:

- **Variant::Drop alternation bug**: `match self.vt { VT_UI1 | VT_ARRAY => ... }` — in pattern position `|` is alternation, not bitwise OR — so the arm matched scalar vt==17 (VT_UI1) and fed the u8 union member (garbage pointer) to SafeArrayDestroy; real arrays (vt==0x2011) silently leaked. Fixed with a `VT_UI1_ARRAY` const (expression position = bitwise OR). After this, probe_charge completed the full 35-call sweep + ON/OFF/restore toggles with exit 0, no crashes.
- **Variant sized 24 bytes** (real oleaut layout: 8-byte header + DECIMAL union 16 bytes) so WMI's `IWbemClassObject::Get` can never write past the struct. `size_tests::variant_is_24_bytes_like_oleaut` added.
- **ComApartment drop order**: `_com` was the first field, so CoUninitialize ran before the COM interface Releases -> teardown AV. Reordered to drop last in both adapters.
- **uint8_array**: `SafeArrayCreate` wants the base element type (VT_UI1); the VT_ARRAY-flagged combo (0x2011) returns NULL.
- **CIMTYPE for array params**: `uReservedIn`/`uReserved` must be put as `CIM_UINT8 | CIM_FLAG_ARRAY` (0x2011); plain CIM_UINT8 -> WBEM_E_INVALID_PARAMETER.
- **Circuit breaker**: adapter self-disables after 5 consecutive failures (`failures`/`dead` cells, `guarded()`), app gates readbacks on `is_available()`; a flapping provider now degrades to warnings instead of destabilizing in-proc WbemCore.

On-device verification on the AN16S-61 (probe_charge) still required; this machine's WMI provider flapping makes the COM path unverifiable here (see ticket 05).
