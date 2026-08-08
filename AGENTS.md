## Toolchain

Rust 1.97 stable, MSVC target via rustup at `~/.cargo/bin`; VS Build Tools 2022 + Windows SDK 10.0.26100 pre-installed. Build and verify with `cargo build` / `cargo test` — the toolchain is already provisioned.

## Agent skills

### Issue tracker

Issues and specs for this repo live as markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Firmware knowledge

Durable, implementation-verified facts about the Acer firmware surface live in `docs/firmware-notes.md`. The implementation (src/ + its tests) is the source of truth; earlier prior-art notes (for a different model) misled agents before and are deleted — do not trust any encoding that is not in the code or the notes.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
