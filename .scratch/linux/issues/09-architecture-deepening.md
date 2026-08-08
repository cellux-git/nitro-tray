# 09 — Architecture deepening: platform-gate review candidates

**What to build:** Decide and implement the deepening opportunities surfaced by the 2026-08-08 architecture review of the ticket-02 platform gate (windows/linux split). The review walked the seams (`MiTransport`/`HidTransport`/`PlanApi`), the per-platform stubs, the entry points (`main.rs` dispatcher + `windows_main.rs`/`linux_main.rs`), and where tickets 03–08 land on this shape. Six candidates were proposed, each with a recommendation strength; none has a designed interface yet — this ticket is the decision + implementation record. The top recommendation (candidate 1) should be resolved before tickets 03–05 replace the Linux stubs, while the widened interface still costs nothing to migrate.

**Blocked by:** 02 (the shape it reviews)

**Status:** ready-for-agent (all six candidates settled by grilling 2026-08-08; resolution below)

## Candidates

### 1. Deepen the plan seam — "plan" is Windows vocabulary leaking across it (Strong, ports & adapters)

Files: `src/power.rs` (`PlanApi`, `PowerError`), `src/app.rs` (`ensure_nitro_plans`, `apply_plan`, `effective`), `src/policy.rs` (`plan_name`/`from_plan_name`).

The seam's interface (`ensure_nitro_plans()`, `set_active_plan(name: &str)`, `active_plan_name() -> String`) is as complex as its Windows implementation and names the OS concept "plan" — Linux has no plans. Consequence: every enforce occasion on Linux logs `power: failed to ensure Nitro plans`, and ticket 05's sysfs backend must reverse-encode Nitro plan names from governor/EPP/boost to satisfy `active_plan_name` for the tray's `Profile::from_plan_name` fallback.

- [ ] Widen the seam to the abstraction both platforms vary over — per-profile CPU tuning state behind the plan table — pushing plan creation/activation/name readback behind the Windows adapter
- [ ] Linux enforce runs quiet; no plan failure on any occasion
- [ ] `NITRO_PLANS`/`plan_name_matches` stay shared anchors; plan-name round-trip constraint on the Linux backend removed
- [ ] `PowerError` gains the partial-failure vocabulary ticket 05's EACCES path needs (per-item failures, "warning logged, remaining writes applied")

### 2. Collapse the per-platform transport constructors onto the seam (Strong, in-process)

Files: `src/wmi.rs`, `src/charge.rs`, `src/hid.rs`, `src/mi.rs`, `src/app.rs` (default generics).

"Unavailable" is encoded twice per seam: the transport stub (`MiConnection`/`RealHidTransport`) and the adapter-constructor stubs (`WmiAdapter::connect()`, `SmartChargeAdapter::connect()`, `HidAdapter::open()` on Linux) that bypass the transport entirely. Tickets 03/04 must edit both layers; afterwards the two per-platform constructor impls become byte-identical. Latent trap: `MiConnection::connect()` resolves to the inherent method on Windows but to the `MiTransport` trait method on Linux once the chardev lands.

- [ ] One shared constructor impl per seam calling the trait's `connect()`, the transport the only per-platform module at the seam
- [ ] Per-seam cfg-split around constructors disappears; degrade message strings single-sourced
- [ ] Recovery loop's `M::connect()` exercises the real transport on reconnect

### 3. Extract the tray model from behind the Windows gate (Strong, ports & adapters)

Files: `src/tray.rs` (`cfg(windows)`), `src/windows_main.rs` (`view_from`).

The pure menu model ticket 07's ksni tray must reuse — `TrayView`, `TrayEvent`, `menu_items`, `tooltip_text`, `plans_for`, `profile_label`, and the view-from-effective-state derivation — sits inside the Windows-only tray module. Linux would either reimplement it (a second, driftable menu; the repo already fought id drift) or extract it mid-ticket.

- [ ] Shared pure module for the tray model; tray.rs keeps only window plumbing
- [ ] Menu tests run from both platforms
- [ ] `view_from` (incl. the plan-name fallback) becomes shared

### 4. Give the sysfs reader a directory seam (Worth exploring, local-substitutable)

Files: `src/power_state.rs`.

`snapshot_from_sysfs` is unit-tested; the directory-walking fall-through glue (`read_sysfs_value`, hardcoded `/sys/class/power_supply`) is tested nowhere — the ticket-02 review already fixed a bug there once, and the pattern "pure encoding tested, I/O glue probe-only" repeats across the split.

- [ ] Supply directory becomes a parameter so the fall-through logic is unit-testable against fixture temp dirs on both platforms

### 5. Rehome per-platform bodies into per-platform files (Worth exploring, in-process)

Files: `src/mi.rs` (1037 lines: shared surface + ~700 lines Win32 FFI + 30-line Linux stub), `src/power.rs` (85% Win32), `src/transport.rs` (seam module carries the Windows `MiTransport for MiConnection` impl + `OUT_PROBES` while the Linux impl lives in mi.rs).

The platform split is vertical; ticket 03's chardev lands inside a 1037-line module and power.rs grows a second real backend into Win32 code. The two trait impls of one seam are not co-located.

- [ ] Per-module platform files (e.g. `mi/win.rs` + `mi/linux.rs`) with the seam module shared-only; trait impls co-located with their types
- [ ] Tickets 03–05 each edit one file

### 6. Share the entry-point boot (Speculative, in-process)

Files: `src/windows_main.rs`, `src/linux_main.rs`, `src/log.rs`.

The panic hook and the ~22-line adapter-wiring degrade block are verbatim copies in both mains; ticket 07 rewires exactly these blocks on Linux.

- [ ] Panic hook into `log`; a shared connect-adapters helper; both binaries keep only platform-specific wiring

## Sequencing

Top recommendation: **candidate 1 first** (the only seam whose interface doesn't fit one of its platforms, and the one every Linux enforce occasion crosses); it pairs with candidate 2 (quickest win for ticket 03) and shapes candidate 3 (the plan-name fallback lives in `view_from`). Candidates 4–6 are independent and cheap; candidate 5 is a pure rehome (no behavior change) that makes 1–3 easier to review. No interfaces are proposed here by design — each candidate gets the `/grilling` treatment before implementation.

**Out of scope:** none of these candidates changes Windows behavior; the 176-test suite stays green and `cargo check --target x86_64-unknown-linux-gnu` stays clean after each.

## Resolution (2026-08-08 grilling, all six candidates accepted)

**Scope and acceptance:** all six candidates. Design attention went to 1–3; 4 and 6 go straight to implementation, 5 is a pure rehome. Acceptance is the Windows-side bar only — 176 tests green, clippy clean on both targets, linux-target check clean — a preparatory step; no on-device Linux verification in this ticket.

**Sequencing:** 5 → 1+2 → 3 → 4+6. Rehome first so 1–3 land directly in their final files (tickets 03/04/05 already assume `mi/linux.rs`, `power/linux.rs`, `hid/linux.rs`). Implementation is recorded as one per-candidate checklist block under this ticket.

### 1. Deepen the plan seam — profile-typed `PlanApi`

- Replace the seam with `ensure_support()`, `set_profile(profile: Profile)`, `active_profile() -> Result<Option<Profile>, PowerError>`. Plan creation/activation/name readback move behind the Windows adapter (the plan-name ↔ profile mapping lives there, using the shared `Profile::from_plan_name`). Linux runs quiet: no plan failures on any enforce occasion.
- `effective()` reads `active_profile()`; `EffectiveState.plan` becomes `power_readback.map(Profile::plan_name())` — the tray's `"Plan: Nitro-X"` line survives on Windows (canonical name), is absent on Linux (readback `None`, no reverse-encoding). The `view_from` fallback (`e.plan.and_then(Profile::from_plan_name)`) stays as-is and works unchanged.
- `NITRO_PLANS`, `plan_name_matches`, `Profile::plan_name`/`from_plan_name` stay shared pure anchors.
- `PowerError` gains `Partial { failed: Vec<&'static str> }` for ticket 05's EACCES path; app.rs maps the items verbatim into `ApplyReport.failed` (granular tray status). `Unavailable` stays.

### 2. Collapse the per-platform transport constructors

- One shared constructor per seam: `impl<M: MiTransport> WmiAdapter<M>::connect()` and `SmartChargeAdapter<M>::connect()` calling `<M as MiTransport>::connect().map_err(map_mi)`; the per-platform impl blocks (and the two hardcoded `NotAvailable` stubs) are deleted.
- The Windows inherent `MiConnection::connect()` body folds into the `MiTransport for MiConnection` trait impl (deleting the inherent kills the inherent-vs-trait resolution trap); probe_mi is unaffected (it calls the adapter constructors). The recovery loop's `M::connect()` exercises the real transport.
- `HidAdapter::open()` stays per-platform (discovery is platform-specific).

### 3. Extract the tray model into a shared module

- New flat shared module `src/tray_model.rs` (ungated): `TrayEvent`, `TrayView`, `MenuItem`, `menu_items`, `tooltip_text`, `plans_for`, `profile_label`, and the shared `view_from` core (incl. the plan-name fallback). tray.rs keeps only Win32 plumbing. Menu tests run from both platforms.
- `TrayView` gains `profiles_greyed: bool` and `plan_section: bool`; `degraded` drives only the "Hardware unavailable" banner. Windows builder sets both to `degraded` (menu byte-identical); Linux sets both to `false` (sysfs tuning applies independently of the firmware seam) and still shows the banner when firmware is down.
- `MenuItem` is neutralized: `{ id, label, separator, enabled, checked }` — Windows maps to `MF_*`/`MFT_*` at append time; ksni (ticket 07) maps natively.
- `start_at_logon` keeps its shared meaning (scheduled task on Windows, `.desktop` toggle on Linux); `TrayEvent::SelectPlan` stays in the model but fires only on Windows. The battery glyph (`make_battery_icon`/`draw_battery_pixels`) stays Windows-only; ticket 07 owns the ksni icon decision.

### 4. Sysfs directory seam

- `read_sysfs_value` takes the supply directory as a parameter and is ungated (`target_os`-independent) so the fall-through logic is unit-testable against fixture temp dirs on both platforms; the Linux `read()` keeps the hardcoded `/sys/class/power_supply` + prefix constants.

### 5. Rehome per-platform bodies into per-platform files

- Directory modules: `mi/` → `mod.rs` (shared surface) + `win.rs` (Win32 FFI, `MiConnection` + its trait impl co-located) + `linux.rs` (stub; ticket 03's chardev lands here); `power/` → `mod.rs` (trait, errors, tuning table, anchors) + `win.rs` (Win32 backend) + `linux.rs` (stub; ticket 05's sysfs backend lands here); `hid/` → `mod.rs` (trait, error, `HidAdapter`) + `win.rs` + `linux.rs` (ticket 04's hidraw lands here). `transport.rs` becomes shared-only (Windows impl + `OUT_PROBES`/`out_elements` move to `mi/win.rs`). Tests move with their code; tickets 03–05 each edit one file.

### 6. Share the entry-point boot

- Panic hook moves into `log` as `log::install_panic_hook()`; a shared connect-adapters helper (small new wiring module) replaces the two verbatim adapter-degrade blocks; both binaries keep only platform-specific wiring.

## Implementation (2026-08-08, all six candidates)

**Status:** ready-for-human (Windows-side bar met: 186 tests green, clippy clean on x86_64-pc-windows-msvc and x86_64-unknown-linux-gnu, linux-target check clean; on-device Linux verification is tickets 03–07's job)

### 5. Rehome per-platform bodies (done first, pure move — no behavior change)

- `src/mi.rs` → `src/mi/` = `mod.rs` (shared surface + `MiError`/`MiResult`/`MiType`/`MiArray`/`coerce_u64`/`wide`) + `win.rs` (FFI raw structs, `MiConnection` + inherent ops, `MiInstance`/`MiOperation`, `instance_clone`) + `linux.rs` (stub `MiConnection` + `linux_unavailable`); `MiConnection` re-exported per platform so `AppCore`'s default generic is unchanged.
- `src/power.rs` → `src/power/` = `mod.rs` (trait/errors/tuning table/anchors) + `win.rs` (Win32 backend) + `linux.rs` (stub).
- `src/hid.rs` → `src/hid/` = `mod.rs` (trait/error/`HidAdapter`) + `win.rs` (SetupDi + `RealHidTransport`) + `linux.rs` (stub); `RealHidTransport` re-exported per platform.
- `src/transport.rs` shared-only: the Windows `MiTransport for MiConnection` impl + `OUT_PROBES`/`OutProbe`/`out_elements` moved to `mi/win.rs`. Module-root public API byte-identical (probes + app.rs compile unchanged).

### 1. Deepen the plan seam — profile-typed `PlanApi`

- Seam is now `ensure_support()` / `set_profile(profile: Profile)` / `active_profile() -> Result<Option<Profile>, PowerError>`; plan creation/activation/name readback live behind the Windows adapter (`power/win.rs`), which maps the active scheme's friendly name through `NITRO_PLANS` + `plan_name_matches` + `Profile::from_plan_name` (pure helper `profile_from_active_name`, unit-tested).
- Linux stub runs quiet: `Ok(())`/`Ok(())`/`Ok(None)` — no plan failure on any enforce occasion, no plan-name reverse-encoding.
- `effective()` reads `active_profile()`; `EffectiveState.plan` = `readback.map(Profile::plan_name())` — "Plan: Nitro-X" survives on Windows (canonical name; `None` now when the active scheme isn't a Nitro plan), absent on Linux. The `view_from` plan-name fallback unchanged.
- `PowerError::Partial { failed: Vec<&'static str> }` added for ticket 05's EACCES path (unit-tested construction).
- `NITRO_PLANS`/`plan_name_matches`/`Profile::plan_name`/`from_plan_name` stay shared anchors; `IntendedState.plan` untouched (call site maps canonical name → `Profile`).

### 2. Collapse the per-platform transport constructors

- One shared constructor per seam: `impl<M: MiTransport> WmiAdapter<M>::connect()` / `SmartChargeAdapter<M>::connect()` = `<M as MiTransport>::connect().map_err(map_mi).map(Self::with_transport)`; the two per-platform impl blocks and the hardcoded `NotAvailable` stubs deleted.
- The Windows inherent `MiConnection::connect()` body folded into the `MiTransport for MiConnection` trait impl (inherent deleted — the inherent-vs-trait trap is gone); the recovery loop's `M::connect()` exercises the real transport. `HidAdapter::open()` stays per-platform (discovery).

### 3. Extract the tray model into a shared module

- New flat ungated `src/tray_model.rs`: `TrayEvent` (incl. `SelectPlan`, fires only on Windows), `TrayView` (gains `profiles_greyed` + `plan_section`; `degraded` drives only the "Hardware unavailable" banner), neutral `MenuItem { id, label, separator, enabled, checked }`, the `MENU_*` id consts, `menu_items`, `tooltip_text`, `plans_for` (keyed off `plan_section`), `profile_label`, and the shared `view_from(app, profiles_greyed, plan_section)` core incl. the plan-name fallback. Menu/tooltip tests moved here and run on both platforms.
- `src/tray.rs` keeps only Win32 plumbing; a pure `append_flags` helper maps the neutral model to `MF_*`/`MFT_*` at append time (radio = id in profile/plan ranges), pinned byte-identical by test. Windows builder passes `degraded, degraded`; Linux will pass `false, false` (banner still shown when firmware is down).

### 4. Sysfs directory seam

- `read_sysfs_value(dir, prefix, file)` — ungated and parameterized over the supply directory; the Linux `read()` keeps the hardcoded `/sys/class/power_supply` + `AC`/`BAT` prefixes. Five ungated fall-through tests (first match wins, unreadable/unparseable falls through, missing supply → `None`) run against fixture temp dirs on both platforms.

### 6. Share the entry-point boot

- Panic hook moved into `log::install_panic_hook()` (single copy); new small `src/wiring.rs` with `connect_adapters() -> (Option<WmiAdapter>, Option<SmartChargeAdapter>, Option<HidAdapter>)` single-sources the three adapter-degrade WARN strings; both `windows_main.rs` and `linux_main.rs` consume both helpers, keeping only platform-specific wiring.

### Code-review fixes (2026-08-08)

- `PowerError::Partial` is consumed: `apply_plan` and the `apply_intended` plan item map the per-item failures verbatim into `ApplyReport.failed` (pinned by a new app.rs test) — the ticket-05 backend now has its granular-status path in place.
- The Windows append mapping's one special case (smart-charge line disabled but never greyed, pre-ticket `MF_DISABLED` only) is restored via the shared `SMART_CHARGE_LABEL` and pinned by a new test — the smart-charge-readback-off view stays byte-identical (the neutral model cannot distinguish that line from the banner/plan/status lines).
- Shared `view_from` takes `&EffectiveState` + flags (pure function of read-back facts; one `effective()` read per view push instead of two); stale `power/mod.rs` docs corrected (Linux stub runs quiet; `Unavailable` reserved); `read_sysfs_value`'s allow-attr uses positive `cfg_attr(windows, ...)` spelling.

## Comments

2026-08-08: Filed from the improve-codebase-architecture review of commit 43316bf (ticket 02). The explore pass found the seam shape itself is good — the shared `MiTransport`/`HidTransport`/`PlanApi` seams make the core fully testable through `FakeTransport`/`FakeHidTransport`/`FakePlanApi` on both platforms — so the candidates target the shallow scaffolding around it: constructor stubs that bypass the seam, a Windows-named seam vocabulary, the Windows-trapped tray model, untested sysfs glue, vertically-split modules, and duplicated entry-point boot. The review also recorded non-candidates: the `MiConnection`/`RealHidTransport`/`PowerApi` stub names are load-bearing for `AppCore`'s default generics and are deliberately kept; `power_state.rs`'s two `read()`s are fine at their size; `#[cfg(windows)]`/`#[cfg(target_os = "linux")]` spelling is unified and stays.

2026-08-08: Grilled all six candidates (design tree resolved, full record in the Resolution section). Settled: all six in scope; Windows-side verification only; order 5 → 1+2 → 3 → 4+6; one implementation record here. Candidate 1 becomes a profile-typed `PlanApi` (`ensure_support`/`set_profile`/`active_profile`) with `PowerError::Partial { failed }`; candidate 2 collapses the MI adapters onto shared trait-calling constructors and folds the inherent `MiConnection::connect()` into the trait impl; candidate 3 extracts a flat `tray_model.rs` with split `profiles_greyed`/`plan_section` view flags and a neutral `MenuItem`; candidates 4–6 as per the Resolution. Status flipped to ready-for-agent.
