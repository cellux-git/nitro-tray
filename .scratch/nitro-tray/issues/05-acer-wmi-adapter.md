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

## Comments (debug session 2026-08-07)

Found and fixed the silent startup crash (AV 0xC0000005 in release, no panic): every COM method call in src/comwbem.rs dereferenced the OBJECT pointer as the vtable pointer instead of reading the vtable pointer THROUGH the object (*(obj)); Release (in ComRef::Drop) was correct, all method calls were not. Fixed at all call sites (locator, services, class object, enumerator). Also: windows-sys 0.61 ships CIM_* constants with oleaut VARENUM values (CIM_UINT32=19, CIM_UINT64=21) which differ from wbemcli.h (21/23); canonical values are now defined locally in comwbem.rs. gmInput put now tries UInt64 then UInt32 then decimal-BSTR (reference path: AeroForge uses UI4-then-BSTR only). Verified against the AeroForge native COM path on the dev machine: byte-identical behavior (this machine's stripped provider rejects Put with WBEM_E_INVALID_PARAMETER while CIM cmdlets work via the MI stack); the reference path is proven on the target SKU. On-device verification on the AN16S-61 remains required.

## Comments (reboot verification 2026-08-07)

After reboot: AcerGamingFunction is registered/unregistered by a user-mode component - the class was missing from ROOT\WMI right after logon and reappeared minutes later (flapping registration while Acer services are disabled). AcerDeviceEnablingServiceV2 was re-enabled per user hint: class registration stabilized, but Put(gmInput) on the method-input instance is STILL rejected with WBEM_E_INVALID_PARAMETER - conclusive that the native COM path cannot run on this machine (AeroForge's identical native path fails the same way here; their working path on this machine is the PowerShell CIM fallback). HID (probe-hid: open + Quiet/Normal/Performance writes ok) and power plans (probe-power: all four Nitro plans exist with correct tuning, active = Nitro-Balanced) fully verified on-device. On-device WMI verification for the AN16S-61 still required.
