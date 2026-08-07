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
