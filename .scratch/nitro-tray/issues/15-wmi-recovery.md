# 15 — WMI provider flapping: recover from late/absent registration instead of staying degraded

**What to build:** the app currently connects the `WmiAdapter` (and `SmartChargeAdapter`) exactly once at startup and keeps that decision for the process lifetime. On the dev machine the `AcerGamingFunction`/`BatteryControl` classes in `ROOT\WMI` register late and flap (a user-mode Acer component registers them minutes after logon), so the app can start degraded ("Hardware unavailable") and stay degraded until the user restarts it — even though the provider comes up a few minutes later. Make the app adapt: retry the adapter when it is missing or disabled, and recover the full feature set without a restart.

**Observed (2026-08-07, dev machine):**

- App started at logon while the classes were absent → `GetObject` failed → `wmi: None` → permanently degraded. No reconnect exists.
- App restarted later, when the classes were up → adapter connected; repeated `Put(gmInput)` failures (WBEM_E_INVALID_PARAMETER/TYPE_MISMATCH on this machine's provider) tripped the circuit breaker after 5 consecutive failures → degraded again, permanently (`dead: Cell<bool>` never resets).
- Features only came back after another manual restart at a moment the provider accepted calls.
- On the target AN16S-61 the provider is expected to work; this machine's flapping makes the recovery path the difference between a usable and a dead tray.

**Status:** needs-triage

**Blocked by:** None — can start immediately.

- [ ] A failed or breaker-disabled WMI adapter is retried periodically (bounded backoff) instead of being terminal for the process lifetime.
- [ ] On successful (re)connect, startup enforcement re-runs (profile, HID, fan auto, smart charge, plan) so the machine converges without a restart.
- [ ] The tray view updates after recovery: "Hardware unavailable" clears and checkmarks/profile items re-enable by themselves.
- [ ] Smart-charge adapter follows the same recovery rule.
- [ ] Repeated connect failures do not log-spam (per-attempt noise is bounded).

**Design questions for triage:**

- Reconnect cadence: align with the existing reapply timer (default 30 s, off by default) vs. a dedicated reconnect timer (e.g. every 30–60 s while degraded)?
- Should the periodic state readback become a real loop? Today `effective()` (WMI profile readback, smart-charge readback, plan readback) runs only on events — startup, menu actions, power change, resume, hotkey, plan select — and **never** on the reapply tick or the 10 s power poll (the poll raises `PowerChanged` only when the AC/battery *state* changed). A quiet session therefore does no readbacks at all, which is part of why stale "degraded" UI can persist.
- Breaker semantics on retry: reset `dead` only after a fully successful adapter reconnect (fresh `connect()`), not on a single successful call?

## Comments

2026-08-07: Filed from the debug session. Current query cadence, for reference: power/battery snapshot every 10 s (`power_state::SLOW_POLL_MS`, tray timer) but it only triggers a full readback on AC/battery state change; `effective()` runs on startup, menu actions (profile/plan/hotkey), power-change and resume events only. The reapply loop (off by default) re-asserts firmware items but does not rebuild the tray view.
