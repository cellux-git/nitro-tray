# 15 — WMI: the WBEM-COM path is broken on the prod machine; stop degrading, stop sweeping

**What to build:** the app talks to the Acer firmware only through the in-process WBEM COM stack (`IWbemServices`). On the prod machine (an AN16S-61 — the same machine this ticket was written on) that stack is unusable while the MI stack (`Invoke-CimMethod`, used by PowerShell) works reliably — so the app must not treat "WMI broken" as a hardware/provider outage, and must not hammer the provider with futile calls. Decide: (a) no sweeping ever — the 35-call `GetBatteryHealthControlStatus` sweep must be removed, (b) the readback path must use the shapes proven to work, (c) "Hardware unavailable" must not be permanent — the app should retry/recover.

**Machine identity correction (2026-08-07, late):** this machine IS the AN16S-61. Earlier sessions assumed the AN16S-61 was a separate target needing "on-device verification"; they were wrong — every observation below is on-device, on the production machine.

**Observed (live, 2026-08-07, on this machine):**

- `Invoke-CimMethod` (instance-bound via `Get-CimInstance` + `-InputObject`, like AeroForge's proven fallback) works for EVERY method right now: `SetGamingMiscSetting` (u64 and u32 inputs both accepted), `GetGamingMiscSetting`, `SetGamingFanBehavior`, and the readback round trip is consistent: `Set 0x40B` (performance) → `Get 0x0B` → `gmOutput=0x400` → decodes to profile 4. The provider is healthy and type-tolerant; the app's opcode encoding is verified correct.
- The WBEM COM stack fails where MI succeeds: `IWbemServices::GetObject(AcerGamingFunction)` → `WBEM_E_PROVIDER_NOT_FOUND` (0x8004103A) at times when `Get-CimClass`/`Get-CimInstance` work fine; when GetObject does succeed, `IWbemClassObject::Put(gmInput)` fails with `WBEM_E_TYPE_MISMATCH` for every shape tried (VT_UI8+CIM_UINT64, VT_UI4+CIM_UINT32, decimal BSTR+CIM_UINT64).
- An earlier "all 35 readback pairs rejected" observation was a usage error (Invoke-CimMethod with `-ClassName` instead of the instance) — retested correctly, the same calls succeed. This supports the general finding: failures attributed to "provider flapping" were wrong-input/transport issues, not the provider.

**Traces in the tickets (prior findings that pointed here):**

- prior-art-aeroforge.md: AeroForge's native COM path puts gmInput as CIM_UINT32 first, then CIM_UINT64-as-decimal-BSTR — and uses VARENUM values as the CIMTYPE constants (CIM_UINT32=19, CIM_UINT64=21, which are actually VT_UI4/VT_UI8), not the canonical wbemcli values (21/23). Their native path fails on this machine exactly like ours; their working path here is the PowerShell CIM fallback (MI stack).
- Ticket 05 comments: canonical CIMTYPE constants + UI8-first put order landed there; "flapping registration while Acer services are disabled" was observed, and registration stabilized after re-enabling AcerDeviceEnablingServiceV2 — the remaining failures are in the COM input layer, not the provider.

**Decisions (user, 2026-08-07):**

- **No sweeping ever.** The 35-call readback sweep in `charge.rs::is_enabled` must go. It is churn against a stack that does not answer it, and even on a healthy machine it is 35 round trips to find one row. Replace with a single direct-trust readback (`uBatteryNo=1, uFunctionQuery=<learned or fixed>`), degrade to `None` on failure.
- **Dev machine is the prod machine** — all verification happens here; there is no separate target to defer to.

**Status:** needs-triage

**Blocked by:** None — can start immediately.

- [ ] `charge.rs`: remove the 5×7 sweep; single-pair readback (battery 1 + the query that answers, learned or fixed), full-sweep fallback only if a future machine needs discovery — flag as a config/decision point.
- [ ] `WmiAdapter`/`SmartChargeAdapter`: a failed or breaker-disabled adapter is retried periodically (bounded backoff) instead of being terminal for the process lifetime; on (re)connect, enforcement re-runs and the tray view refreshes ("Hardware unavailable" clears by itself).
- [ ] Investigate the WBEM-COM transport on this machine (provider registration serving MI but not WBEM; `Put` TYPE_MISMATCH with correct shapes) — and record whether a working in-process COM path exists at all here, or whether the app needs a different transport decision (out of scope: no interpreter spawning per design decision 7).
- [ ] Repeated connect/read failures do not log-spam.

**Design questions for triage:**

- Reconnect cadence: align with the existing reapply timer (default 30 s, off by default) vs. a dedicated reconnect timer (e.g. every 30–60 s while degraded)?
- Should the periodic state readback become a real loop? Today `effective()` (WMI profile readback, smart-charge readback, plan readback) runs only on events — startup, menu actions, power change, resume, hotkey, plan select — and **never** on the reapply tick or the 10 s power poll (the poll raises `PowerChanged` only when the AC/battery *state* changed). A quiet session therefore does no readbacks at all, which is part of why stale "degraded" UI can persist.
- Breaker semantics on retry: reset `dead` only after a fully successful adapter reconnect (fresh `connect()`), not on a single successful call?
- Smart-charge readback without the sweep: which `uFunctionQuery` answers on this SKU? (Verify via MI while it works; candidate: the query matching the direct-trust write path.)

## Comments

2026-08-07: Filed from the debug session. Current query cadence, for reference: power/battery snapshot every 10 s (`power_state::SLOW_POLL_MS`, tray timer) but it only triggers a full readback on AC/battery state change; `effective()` runs on startup, menu actions (profile/plan/hotkey), power-change and resume events only. The reapply loop (off by default) re-asserts firmware items but does not rebuild the tray view.

2026-08-07 (late): machine-identity correction + evidence above. Live MI verification on this machine: `SetGamingMiscSetting`/`GetGamingMiscSetting`/`SetGamingFanBehavior` all OK instance-bound (u32 and u64 inputs); WBEM COM GetObject → PROVIDER_NOT_FOUND while MI works; Put(gmInput) TYPE_MISMATCH with canonical CIMTYPE and correct VT shapes when GetObject succeeds. "No sweeping ever" + "dev machine is the prod machine" recorded as decisions.
