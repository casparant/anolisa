# AGENTS.md — AW Core

> Common Rust, documentation, commit, and review rules are defined in the
> [root AGENTS.md](../../AGENTS.md). This file contains AW-specific rules.

## Architecture

The workspace currently contains two crates:

- `aw-contracts`: side-effect-free public identities and versioned Contracts.
- `aw-provider-host`: headless Provider discovery, admission, graph projection,
  process supervision, and invocation.

Dependency direction is `aw-provider-host -> aw-contracts`. Neither crate
may depend on `src/cosh-ng/` or a concrete Provider. COSH and other Agent
Environments consume AW APIs; they are not part of the Provider call path.

ACP remains a COSH-owned protocol integration until a non-COSH consumer creates
a concrete need for a shared integration.

## Contract Boundaries

- Contracts contain no I/O, storage, transport, process, or UI implementation.
- Provider discovery is explicit. Do not search ambient `PATH` or implicit user
  directories across a trust boundary.
- A Provider outcome is not an AW final decision. The Host returns typed
  outcomes and receipts; a future Core policy owner decides adoption.
- Keep content out of receipts and diagnostics. Retain bounded metadata and
  digests unless the Contract explicitly requires content.
- Add a new Driver only with lifecycle, deadline, cancellation, output-bound,
  health, and failure semantics.

## Build and Test

```bash
cd src/aw
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

Use commit scope `aw` for files under `src/aw/`.
