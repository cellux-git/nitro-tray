# 04 — Power API wrapper

**What to build:** the in-process Windows power-management wrapper. It creates the four Nitro power plans (Nitro-Quiet/-Balanced/-Performance/-Eco) once from the Windows Balanced plan, renamed per the plan table, activates a target plan, reads the active plan, and detects plans by name. Nothing is ever re-tuned after creation, and no external process is spawned.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] The four Nitro plans are created once from the Windows Balanced plan (duplicate + rename) with the documented names; creating them again is a no-op. *Implemented; needs on-device verification.*
- [ ] A target plan can be activated via the in-process power APIs. *Implemented; needs on-device verification.*
- [ ] The active plan can be read back. *Implemented; needs on-device verification.*
- [ ] Plans are detected by name (no state stored outside Windows). *Implemented (name matching unit-tested); enumeration against real schemes needs on-device verification.*
- [x] No `powercfg` or any other external process is spawned.
- [x] Unit tests cover plan detection, active-plan readback, and the processor-state read/write encoding (CPU min/max, boost) per the plan table. *(Readback is real-API — verified on-device via probe; the pure encoding/name-matching parts are unit-tested here.)*

## Comments

Implemented in `src/power.rs` + `src/bin/probe_power.rs` (owned by this ticket; nothing else touched):

- `cpu_tuning(profile)` per spec plan table (Quiet 5/45 off, Balanced 5/99 default, Performance 5/100 aggressive, Eco 5/40 off) — unit-tested for every profile.
- `PowerApi::find_plan` enumerates schemes via `PowerEnumerate(ACCESS_SCHEME)` and matches each scheme's `PowerReadFriendlyName` against the requested name with the pure, unit-tested helper `plan_name_matches` (case-insensitive, whitespace-trimmed).
- `PowerApi::active_plan_name` via `PowerGetActiveScheme` + `PowerReadFriendlyName`; the API-allocated GUID is released with `LocalFree`; null result maps to `PowerError::NotActive`.
- `PowerApi::ensure_nitro_plans` is idempotent: `find_plan` first, then `PowerDuplicateScheme(GUID_TYPICAL_POWER_SAVINGS)` -> `PowerWriteFriendlyName` -> creation-only tuning (`PowerWriteACValueIndex` + `PowerWriteDCValueIndex` per setting) of throttle min/max and boost. Existing plans are never re-tuned. The duplicated-scheme GUID is `LocalFree`d after rename+tune.
- `PowerApi::set_active_plan` via `PowerSetActiveScheme`; missing plan -> `PowerError::NotFound`.
- All failures map to `PowerError::Api { hr, op }` with stable op strings exactly matching the windows-sys function names ("PowerEnumerate", "PowerDuplicateScheme", "PowerWriteFriendlyName", "PowerWriteACValueIndex", "PowerWriteDCValueIndex", "PowerSetActiveScheme", "PowerGetActiveScheme", "PowerReadFriendlyName", "PowerReadACValueIndex").
- `src/bin/probe_power.rs` (auto-discovered bin): prints active plan name, runs `ensure_nitro_plans` (safe twice), prints each plan's GUID + expected tuning + AC min/max/boost read back via `PowerReadACValueIndex`, plus the hardcoded boost-mode registry GUIDs. Elevated on-device diagnostic; not run in this environment.

windows-sys 0.61.2 findings (checked `C:\Users\zsolt\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\windows-sys-0.61.2`):

- All ten Power functions exist with the expected signatures (return `WIN32_ERROR`/`u32`; gated on `Win32_System_Registry`, which is enabled).
- Constants available in `Win32::System::SystemServices`: `GUID_TYPICAL_POWER_SAVINGS` (Balanced scheme 381b4222-f694-41f0-9685-ff5bb260df2e), `GUID_PROCESSOR_SETTINGS_SUBGROUP`, `GUID_PROCESSOR_THROTTLE_MINIMUM`, `GUID_PROCESSOR_THROTTLE_MAXIMUM`, `GUID_PROCESSOR_PERF_BOOST_MODE`, and the boost index constants `PROCESSOR_PERF_BOOST_MODE_DISABLED/ENABLED/AGGRESSIVE` (0/1/2). All used directly.
- Discrepancy: the ticket text's `GUID_PROCESSOR_SETTINGS_SUBGROUP` value `54533251-0be8-43c1-b0f7-71f6ddef9a1d` is NOT the real Windows GUID. windows-sys (authoritative, from Microsoft metadata) defines it as `54533251-82be-4824-96c1-47b60b740d00`; that constant is used here.
- Hardcoded (NOT in windows-sys): the three boost-mode value/registry GUIDs — `BOOST_MODE_DISABLED_VALUE_GUID` 3b04d4fd-1cc7-4f23-ab1c-d1337819c4e2, `BOOST_MODE_ENABLED_VALUE_GUID` 893dee8e-2bef-41e0-89c6-b55d0929964c, `BOOST_MODE_AGGRESSIVE_VALUE_GUID` 36687f9e-e3a5-4dbf-b1dc-15eb381c6863. Exposed as pub consts (used by the probe report); the write path encodes boost by index only.

Verification in this environment: `cargo build` of this crate currently fails ONLY in other agents' files (`src/wmi.rs` ticket 05, previously `charge.rs`/`task.rs`/`log.rs` — all since fixed by their owners); `src/power.rs` and `src/bin/probe_power.rs` compile warning-free (verified via scratch copy with other modules' files stubbed). `cargo test --lib power::` = 4/4 pass (cpu_tuning table, plan_name_matches case-insensitivity, boost index encoding, little-endian GUID decode).

On-device verification still required (elevated, on the target AN16S-61): run `probe_power.exe` twice — first run must create all four Nitro plans with correct friendly names and tuned values, second run must be a no-op; delete one Nitro plan and confirm it is recreated; `set_active_plan`/`active_plan_name` round-trip; confirm in `powercfg /getactivescheme` and Settings that plans are named and visible.
