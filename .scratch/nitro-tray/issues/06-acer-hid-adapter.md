# 06 — Acer HID adapter

**What to build:** writes to the Acer HID device (vendor 0x1025) to set the system usage mode (Quiet/Normal/Performance) via feature reports, matching each profile's usage mode. A HID write failure must never take the app down: it is logged, and the WMI profile plus the Windows plan are still applied. Verified on-device with a probe.

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [x] Usage mode feature reports are written on the vendor 0x1025 device for Quiet, Normal, and Performance.
- [x] Each profile maps to its usage mode (quiet → Quiet, balanced → Normal, performance → Performance, eco → Quiet).
- [x] A HID write failure is non-fatal: logged, and WMI profile + plan application continue.
- [ ] On-device probe verification exercises real HID feature-report writes.

## Comments

Implemented by the ticket-06 agent. `src/hid.rs` and `src/bin/probe_hid.rs` are
complete; build and unit tests are green (see below).

### Checklist status

- Done and unit-verified: `usage_mode_report` (exact prior-art prefixes,
  Performance=1/Normal=2/Quiet=3), `usage_mode_from_selector` (3→Quiet,
  2→Normal, 1→Performance, else None), `HidAdapter::open` (SetupDi
  enumeration by `hid#1025174b&col01#` path marker + `HidD_GetAttributes`
  VendorID == 0x1025, `CreateFileW(GENERIC_READ|GENERIC_WRITE,
  FILE_SHARE_READ|FILE_SHARE_WRITE, OPEN_EXISTING)`), `set_usage_mode`
  (65-byte `HidD_SetFeature`), `read_usage_mode` (best-effort status probe,
  selector 1), `Drop` closes the handle. Eco maps to Quiet via
  `policy::Profile::hid_mode()` (ticket 03 owns that mapping; hid.rs only
  encodes `HidMode`).
- Not verifiable here (off-device): "On-device probe verification exercises
  real HID feature-report writes" — `src/bin/probe_hid.rs` is built but must
  be run elevated on the target laptop.

### `CreateFileW` location (findings)

`CreateFileW` in windows-sys 0.61.2 lives ONLY in
`Win32::Storage::FileSystem` (feature `Win32_Storage_FileSystem`), which is
NOT enabled in `Cargo.toml`; `Win32::System::IO` does not contain it either.
`Cargo.toml` is off-limits for this ticket, so `src/hid.rs` declares
`CreateFileW` itself via an `unsafe extern "system"` block (kernel32.lib is
linked by default on the MSVC target) and defines the small constants
locally (`FILE_SHARE_READ=0x1`, `FILE_SHARE_WRITE=0x2`, `OPEN_EXISTING=0x3`);
`GENERIC_READ`/`GENERIC_WRITE` come from the enabled
`Win32::Foundation`. The probe reuses the same local declaration.

### `read_usage_mode` honesty note

Prior art has NO usage-mode readback; the only HID status readback is the
sensor probe (selectors 1=CPU temp, 2=CPU fan RPM, 3=system temp, 6=GPU fan
RPM). `read_usage_mode` sends the status request with selector 1 and decodes
the raw u16 only when it exactly matches a known mode byte (1/2/3);
otherwise it returns `HidError::Io` with an explicit "readback unsupported
by the device protocol" message. Documented in the function doc comment.
Expected behavior on-device: selector 1 returns a CPU temperature, so
`read_usage_mode` will return `HidError::Io` — this is correct and honest.

### Test results

- `cargo build`: clean for hid.rs and probe_hid.rs (only remaining warning
  is `PolicyEngine` dead fields in src/policy.rs, owned by ticket 03).
- `cargo test --lib hid::`: 3 passed (report prefixes, selector map, unknown
  selectors).
- Full `cargo test`: 15 passed, 0 failed.
- `cargo build --bin probe_hid`: clean.

### Needs on-device verification

1. Run `probe_hid` elevated on the Nitro: open succeeds, each mode's
   `HidD_SetFeature` succeeds, raw 65-byte readback response shows the
   expected prefix, Quiet is restored at exit.
2. Confirm the `hid#1025174b&col01#` path marker matches the device path on
   the target machine (device paths differ per collection; prior art is
   AeroForge-proven for the AN16S-61).
3. Observe `read_usage_mode` failing with `HidError::Io` (sensor value, not
   a mode byte) — expected by design.
