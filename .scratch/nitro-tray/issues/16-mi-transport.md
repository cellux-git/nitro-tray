# 16 — Native MI transport for the Acer WMI classes (bypass the WBEM-COM route)

**What to build:** replace the WBEM-COM transport (the flapping layer) with a native MI (Management Infrastructure) client — hand-rolled Rust FFI to `C:\Windows\System32\mi.dll`, the same stack `Invoke-CimMethod`/PowerShell use. The two hardware adapters (`WmiAdapter`, `SmartChargeAdapter`) keep their public APIs and move onto the new transport; no interpreter is spawned (design decision 7 holds: `mi.dll` is a C API called in-process).

**Why (evidence, 2026-08-07, all on this machine):**

- The WBEM-COM route fails at EVERY stage, in rotating "bad windows", while the MI route works 100%: 275 `ExecMethod` failures with `WBEM_E_PROPAGATED_QUALIFIER` (0x8004102E) in 2 h, plus `PROVIDER_NOT_FOUND`, `INVALID_CLASS`, `INVALID_NAMESPACE`, `TYPE_MISMATCH` — and Acer's own `AcerHardwareService` logs the same failures, so this is not our app's doing.
- The underlying provider is healthy: `WmiProv` → `wmiprov.dll` (WMI-ACPI bridge) + `wmiacpi.sys` are present and running, and the same calls succeed via MI (`Invoke-CimMethod` instance-bound) every time — today's MI verifications: `GetGamingMiscSetting(0x0B)` → 0x400, `GetGamingFanBehavior` → 2, `SetGamingMiscSetting(0x60B)` set + readback 0x600, `GetBatteryHealthControlStatus` battery 1 → `list=3, status=[0,1,0,0,0]`.
- COM-only fixes already landed (ticket 15): `VT_I4 + CIMTYPE 0` gmInput shape and the provider-enumerated instance path — both proven to work *in good windows*, but the windows themselves remain unreliable because the failure is in the WbemCore→wmiprvse handoff, not in our encoding.
- The crash-era WER evidence (19 AVs in `oleaut32`/heap corruption, 19:14–20:05Z) was our own old `Variant::Drop` misuse — fixed; the current build has zero crashes. The residual flapping is provider-host/registration churn that MI does not go through.

**Status:** ready-for-agent

**Blocked by:** 15 — WMI recovery (the recovery loop stays; it now guards a transport that can actually succeed)

- [x] `src/mi.rs`: RAII `MiConnection` (`MI_Application_Initialize`, `MI_Application_NewSession`); instance fetch (`MI_Session_EnumerateInstances`, first instance — PowerShell's `-InputObject` equivalent; class-level invocation is known to fail on this machine, so no `-ClassName`-style path); `MI_Session_Invoke` with a synchronous wrapper (callback + channel, or `MI_Operation_GetInstance` polling); `MI_Instance_SetElement` for inputs (`MI_Value` union, u8/u16/u32/u64 + u8 arrays); `MI_Instance_GetElement` for out params (gmOutput, uFunctionList, uFunctionStatus arrays, ReturnValue); `MI_Result` → error mapping; `MI_Operation_Close`/`MI_Application_Close` on drop. Follow the `comwbem.rs` house style (raw FFI, `# Safety` contracts).
  - Polling was chosen (documented synchronous mode): NULL callbacks + `MI_Operation_GetInstance` loop, which blocks until a result is available. Result instances are cloned (`MI_Instance_Clone`) before the loop advances.
  - Input bags use `MI_Instance_AddElement`, not `SetElement`: dynamic instances have no RTTI, so `SetElement` fails with `MI_RESULT_NO_SUCH_PROPERTY` (found on-device).
  - `mi.dll` exports ONLY `MI_Application_InitializeV1` (verified with `link /dump /exports`; the plain spelling from `mi.h` is a compile-time macro).
- [x] `src/wmi.rs` (`WmiAdapter`): same public API, transport swapped to `mi.rs`; keep the opcode encodings (`misc_setting_request`, `fan_behavior_request`, `decode_gm_output_byte`) and the circuit breaker unchanged. Instance re-enumerated per call (self-healing against provider registration churn).
- [x] `src/charge.rs` (`SmartChargeAdapter`): same public API; keep the single-pair readback (battery 1, query 1), the direct-trust + fallback write tuples, and the breaker.
- [x] `src/app.rs`: no changes expected (adapters are behind their existing seams); recovery loop and readback tick keep working as-is. Verified by a live `nitro-tray --log` startup run.
- [x] Probes: `probe_wmi`/`probe_charge` work unchanged (they call the adapter APIs); `probe_com_shapes` becomes obsolete — keep as a COM-side diagnostic or delete.
  - `probe_com_shapes` kept as the COM-side diagnostic. New `src/bin/probe_mi.rs` added as the MI-side diagnostic (drives `MiConnection` directly).
- [x] `comwbem.rs` disposition: once no adapter uses it, decide retire vs keep (it is owned by nobody; the task.rs COM code uses winapi, not comwbem).
  - **Kept**: it is still the shared COM/WMI plumbing behind `probe_com_shapes`; retiring it would delete the only remaining COM diagnostic for future transport investigations.
- [x] Verify on-device (this machine): repeated readback + write cycles succeed through multiple "bad-window" periods that previously broke COM; watch `nitro-tray.log` for a clean startup enforcement (WMI profile + fan + smart charge + plan all INFO, no degraded).

**Design questions:**

- MI primary only (recommended: it works on this machine and everywhere else too) vs COM-primary-with-MI-fallback (more code, keeps a broken route alive). **Resolved: MI only.**
- Instance binding: enumerate-then-invoke (proven PowerShell shape) vs `MI_Session_GetInstance` with the known `InstanceName` key (`ACPI\PNP0C14\APGe_0`). **Resolved: enumerate-then-invoke.**
- Does MI make the Acer services unnecessary? The hardware path (firmware ↔ `wmiacpi.sys` ↔ `wmiprov.dll`) has no user-mode Acer service in it, but ticket 05 observed the *registration* of these classes flapping while Acer services were disabled and stabilizing after re-enabling `AcerDeviceEnablingServiceV2`. MI reads the same repository, so it may still depend on that registration. **Verification step: with the app working on MI, stop the Acer services and confirm whether `Get-CimClass`/invocation still resolves — record the minimum required set.** The MI transport makes the app insensitive to the services' *stability* regardless (no more shared WbemCore state), but possibly not to their *presence*. **Not performed: stopping Acer services mid-session is a system change left for the user (non-blocking).**

**Comments**

2026-08-08: Implemented and verified on-device (this machine, elevated):

- Full MI stack transcribed from the SDK 10.0.26100 `um\mi.h` (packed 8): `MI_Result`/`MI_Type`/`MI_Value`/`MI_FLAG_*`, the `MI_Application`/`MI_Session`/`MI_Operation`/`MI_Instance` handles and all four function tables with exact slot order. Everything except `MI_Application_InitializeV1` goes through the FT, like the C inline wrappers.
- Live results (all through the new transport): `probe_wmi` — profile write+readback for quiet/balanced/performance/eco all correct, restore ok, fan auto + readback `2`; `probe_charge` — cap ON/OFF/ON round trip with correct readbacks; `probe_mi` — raw rows `list=3`; full app startup log clean: eco detection `6` accepted, profile 6, HID Quiet, fan auto, smart charge enabled, `Nitro-Eco` plan, no degraded.
- **Finding (charge writes):** on the AN16S-61 the prior-art direct-trust tuple `(1,1,status)` is *accepted* (`ReturnValue=1`) but does not apply — it writes the health byte's bit 0 and clears bit 1 (`[1,0,0,0,0]`), i.e. it actively disables the cap while reporting truthy (probe_mi + PowerShell both). The mask-2 tuple `(1,2,1)`/`(1,2,0)` is the one that applies. The success gate was therefore changed to the prior-art §3.4 shape: every truthy `ReturnValue` is confirmed by a 250 ms sleep + single-pair readback match before the attempt counts as success; a lying attempt falls through to the next tuple. Tuples and order unchanged; the direct tuple still wins on SKUs where it truly applies.
- COM-era workarounds left behind: `Variant::i4/i8` + CIMTYPE-0 shape and the provider-enumerated instance path are now dead code in `comwbem.rs` (kept only for `probe_com_shapes`).
- Follow-ups for the user: (1) charger behavioral check — cap ON, battery < 80%, plugged in: charging should stop at 80%; (2) Acer-services-presence test above.
