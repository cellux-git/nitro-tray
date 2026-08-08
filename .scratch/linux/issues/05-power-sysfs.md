# 05 — Power layer on sysfs

**What to build:** Per-profile CPU tuning on Linux: the PlanApi seam implemented with in-process sysfs writes — governor, energy-perf-preference, and boost per profile, mapping the existing plan table 1:1 — with an active-plan readback analog. No external processes are spawned (design decision 7 holds). If a write is refused (EACCES), the adapter degrades with a warning and applies what it can. power-profiles-daemon conflict is detected and logged (ppd rewrites the same cpufreq attributes at startup and on every profile switch; the app never fights it silently). The one-time root setup grants group write access to the cpufreq attributes via a udev RUN chmod rule (elevation decision: single unprivileged process). On-device probe verifies governor/EPP/boost transitions per profile.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Each profile produces its expected governor/EPP/boost state on the AN16S-61, verified by readback and probe
- [ ] Readback analog confirms the active plan matches intent across profile switches
- [ ] EACCES path: warning logged, remaining writes still applied, app continues
- [ ] ppd detected → conflict warning logged; no external processes spawned
- [ ] Setup script applies the cpufreq udev chmod rule
