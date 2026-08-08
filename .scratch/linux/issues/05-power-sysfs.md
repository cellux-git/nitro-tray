# 05 — Power layer on sysfs

**What to build:** Per-profile CPU tuning on Linux: the PlanApi seam implemented with in-process sysfs writes — governor, energy-perf-preference, and boost per profile — behind the widened per-profile tuning-state seam (ticket 09 candidate 1): the profile→tuning encodings (governor/EPP included, which the shared `cpu_tuning` table lacks today) join the shared table, there is no plan-name readback or reverse-encoding on Linux, and `ensure_nitro_plans` gets a defined Linux behavior (no-op or writability probe — decided in 09's grilling). No external processes are spawned (design decision 7 holds). If a write is refused (EACCES), the adapter degrades with a warning and applies what it can — via the per-item partial-failure vocabulary `PowerError` gains in candidate 1. power-profiles-daemon conflict is detected and logged (ppd rewrites the same cpufreq attributes at startup and on every profile switch; the app never fights it silently). The one-time root setup grants group write access to the cpufreq attributes via a udev RUN chmod rule (elevation decision: single unprivileged process). On-device probe verifies governor/EPP/boost transitions per profile.

**Blocked by:** 02, 09 (candidates 1 and 5: widened plan seam + platform files)

**Status:** ready-for-agent

- [ ] Each profile produces its expected governor/EPP/boost state on the AN16S-61, verified by readback and probe
- [ ] Readback confirms the per-profile tuning state matches intent across profile switches (no plan-name round-trip)
- [ ] EACCES path: warning logged, remaining writes still applied, app continues
- [ ] ppd detected → conflict warning logged; no external processes spawned
- [ ] Setup script applies the cpufreq udev chmod rule

## Comments

2026-08-08: Adjusted for ticket 09 (done first) — this is the ticket candidate 1 changes most. The "mapping the existing plan table 1:1 — with an active-plan readback analog" premise is replaced by the widened tuning-state seam: no plan-name reverse-encoding to satisfy `active_plan_name`, no `NITRO_PLANS` semantics on Linux, and `ensure_nitro_plans` (which today errors on every enforce) becomes a defined behavior. The profile→tuning encodings become part of the shared table behind the seam (governor/EPP/boost per profile), so this ticket is about the sysfs I/O + ppd/EACCES handling, not the plan vocabulary. After candidate 09-5 the backend lands in the platform file (`power/linux.rs`).
