# AGENTS.md — AW Core

> Common Rust, documentation, commit, and review rules are defined in the
> [root AGENTS.md](../../AGENTS.md). This file contains AW-specific rules.

## Architecture

The workspace currently contains four crates:

- `aw-contracts`: side-effect-free public identities and versioned Contracts.
- `aw-provider-host`: headless Provider discovery, admission, graph projection,
  process supervision, and invocation.
- `aw-core`: execution-context ownership, exact Capability routing, invocation
  policy, and Provider candidate validation.
- `aw-cosh-hook`: COSH-specific `PostToolUse` wire adapter over AW Core.

Dependency direction is:

```text
aw-cosh-hook -> aw-core -> aw-provider-host -> aw-contracts
          |              `------------------------------->|
          `---------------------------------------------->|
```

No AW crate may depend on `src/cosh-ng/` or a concrete Provider. The COSH
adapter owns the COSH wire shape locally so Core remains Agent Environment
independent. Other Agent Environments must add their own boundary adapter
rather than teach Core their native protocol.

ACP remains a COSH-owned protocol integration until a non-COSH consumer creates
a concrete need for a shared integration.

## Contract Boundaries

- Contracts contain no I/O, storage, transport, process, or UI implementation.
- Provider discovery is explicit. Do not search ambient `PATH` or implicit user
  directories across a trust boundary.
- A Provider outcome is not an AW final decision. The Host returns typed
  outcomes and receipts; Core validates a candidate, and the Agent Environment
  still decides whether the candidate reaches the next model request.
- `aw-core` owns canonical identities, scope, routing, deadlines, budgets,
  and candidate validation. It must not parse COSH Hook envelopes or emit COSH
  Hook responses.
- `aw-cosh-hook` may translate COSH fields and choose COSH presentation, but
  must not select behavior by concrete Provider ID.
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
