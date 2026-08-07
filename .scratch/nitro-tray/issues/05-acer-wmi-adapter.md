# 05 — Acer WMI adapter

**What to build:** in-process raw COM/WMI control of the Acer gaming firmware: set and read back the platform profile (quiet 0, balanced 1, performance 4, eco 6) and set/read fan behavior to auto, against `AcerGamingFunction`. All opcode/method encodings match the proven AeroForge tables and carry unit tests. Readback is verified on the target machine with a probe, because the interface cannot be meaningfully mocked.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] Platform profile can be set and read back via in-process COM/WMI (write + readback round trip).
- [ ] Fan behavior can be set to auto (0x00410009) and read back.
- [x] Opcode/method encoding unit tests cover the profile and fan tables (prior art: AeroForge's encoding tests).
- [ ] On-device probe verification exercises real WMI writes and readbacks (elevated, test-time only; the app itself never spawns processes).
- [x] No PowerShell or other interpreter is ever spawned; no CIM/PowerShell fallback path exists.

## Comments

Implemented in `src/wmi.rs` + `src/bin/probe_wmi.rs`.

- Encoding helpers (`misc_setting_request`, `fan_behavior_request`,
  `decode_gm_output_byte`) are pure and unit-tested against the prior-art
  tables (7 decode cases, profile 4 => 0x40B, FAN_AUTO 0x00410009).
- `WmiAdapter::connect()` does the full COM setup once (CoInitializeEx
  COINIT_MULTITHREADED, CoInitializeSecurity DEFAULT/IMPERSONATE with
  RPC_E_CHANGED_MODE/RPC_E_TOO_LATE tolerated per AeroForge, CoCreateInstance
  CLSID_WbemLocator, ConnectServer(ROOT\WMI), CoSetProxyBlanket,
  GetObject(AcerGamingFunction)); `Drop` releases class/services/locator and
  CoUninitializes. `NotAvailable` maps REGDB_E_CLASSNOTREG (CoCreateInstance)
  and WBEM_E_INVALID_CLASS / WBEM_E_NOT_FOUND (GetObject).
- gmInput encoding: CIM_UINT32 (VT_UI4) first, falls back to CIM_UINT64 as a
  decimal BSTR (VT_BSTR) when the first Put fails — same two-path strategy as
  AeroForge's `put_u64` (acer_wmi.rs:685-742).
- ExecMethod: S_OK is success; WBEM_E_NOT_FOUND maps to `WmiError::Unexpected`
  with context (ticket decision); a missing gmOutput (null out-params or
  `Get(gmOutput)` WBEM_E_NOT_FOUND, per prior-art §1.1) maps to Unexpected for
  the read methods. Profile readback decodes via the AMD-shifted byte rule;
  fan readback returns the raw gmOutput as u32.
- **Deviation found during implementation**: windows-sys 0.61 does NOT ship
  the `IWbemLocator`/`IWbemServices`/`IWbemClassObject` interfaces (no
  ConnectServer/ExecMethod anywhere in the crate — only the CLSID `WbemLocator`
  and WBEM constants), and `VARIANT`/VT_* live only under the disabled
  `Win32_System_Variant` feature. Since Cargo.toml is not editable by this
  ticket, the three vtables (canonical wbemcli.h layouts, slot-for-slot
  identical to AeroForge's hand-rolled bindings) and a minimal VARIANT are
  declared locally in `src/wmi.rs`. BSTRs use `SysAllocString`/`SysFreeString`
  from `Win32::Foundation` (not Ole, where they do not exist). Runtime
  behavior is unaffected; only the plumbing differs from the ticket's
  expectation.
- Also implemented `WmiAdapter` as `Send`/`Sync` (unsafe markers) so the app
  core can share it; COM runs on the UI thread only.
- Verified here: `cargo build` clean for both files (no warnings) and all
  `wmi::` unit tests pass in an isolated scratch crate with the same
  windows-sys 0.61.2 feature set. The repo-wide build currently fails only on
  other tickets' in-flight files (charge.rs etc.), not on these.

Needs on-device verification (probe_wmi.exe, elevated, on the target):
- Profile write + readback round trip for 0/1/4/6, restore of the original
  profile value.
- Whether `SetGamingMiscSetting`/`SetGamingFanBehavior` return S_OK (expected)
  or WBEM_E_NOT_FOUND as their no-output acknowledgment; if NOT_FOUND, the
  current code maps it to Unexpected for sets as well (per the ticket), and
  the set would report failure — adjust the mapping to treat NOT_FOUND as a
  successful write for SET methods if the probe shows that.
- gmInput type acceptance: whether the UINT32 path works or the UINT64-BSTR
  fallback is exercised (probe prints whichever errors surface).
- `GetGamingFanBehavior` existence and raw readback value (never invoked by
  AeroForge; unproven encoding).
