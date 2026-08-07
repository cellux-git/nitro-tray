# 04 — Power API wrapper

**What to build:** the in-process Windows power-management wrapper. It creates the four Nitro power plans (Nitro-Quiet/-Balanced/-Performance/-Eco) once from the Windows Balanced plan, renamed per the plan table, activates a target plan, reads the active plan, and detects plans by name. Nothing is ever re-tuned after creation, and no external process is spawned.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] The four Nitro plans are created once from the Windows Balanced plan (duplicate + rename) with the documented names; creating them again is a no-op.
- [ ] A target plan can be activated via the in-process power APIs.
- [ ] The active plan can be read back.
- [ ] Plans are detected by name (no state stored outside Windows).
- [ ] No `powercfg` or any other external process is spawned.
- [ ] Unit tests cover plan detection, active-plan readback, and the processor-state read/write encoding (CPU min/max, boost) per the plan table.
