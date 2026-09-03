# AW

[中文版](README_zh.md)

AW gives Agent workloads shared system capabilities without coupling those
capabilities to one Agent or one user interface. This workspace owns the public
Contracts, Core policy, and headless Provider mechanisms used by Agent
Environments, IDEs, workflow engines, and individual Providers.

The current implementation contains four crates:

| Crate | Responsibility |
| --- | --- |
| `aw-contracts` | Transport-independent identities and versioned Capability and Provider Contracts |
| `aw-provider-host` | Provider discovery, admission, codec mapping, bounded invocation, and diagnostics |
| `aw-core` | Execution-context ownership, exact Capability routing, invocation policy, and candidate validation |
| `aw-cosh-hook` | COSH `PostToolUse` adapter for the Core context-projection path |

COSH is the default interactive Agent Environment in the product architecture.
The current PoC connects its built-in Agent's `PostToolUse` boundary to AW
Core and the generic Provider Host, with tokenless as the first real Provider.
It does not yet install that hook by default or cover arbitrary Agents launched
inside `cosh-shell`.

## Dependency Direction

```text
COSH / another Agent Environment
               |  stable execution and Tool Call scope
               v
          aw-core
               |  canonical CapabilityInvocation
               v
    aw-provider-host ------> Provider package
               |                  providers/<id>/
               v
     candidate + content-free receipt

All three runtime crates depend on the leaf aw-contracts crate.
```

`aw-contracts` contains no transport, process management, persistence, or
Provider implementation. `aw-core` depends on the Contracts and Provider
Host, but contains no COSH-specific wire format. The COSH adapter depends on
Core; AW Core and Provider Host never depend on COSH.

## Build and Test

```bash
cd src/aw
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

## Inspect a Provider

The current binary is a development and diagnostic surface. It is not packaged
or installed as a public CLI, and its command syntax is not a compatibility
contract:

```bash
cargo run -p aw-provider-host -- \
  doctor \
  --manifest /absolute/path/to/providers/tokenless/provider.toml \
  --executable-root /absolute/path/to/src/tokenless/target/debug
```

Use `list` to print the Runtime Capability Graph and `invoke` to submit a
versioned `CapabilityInvocation` JSON document. The Host admits only explicit
absolute manifest roots and does not search ambient `PATH`.

## Run the Source PoC

The end-to-end source PoC is:

```text
COSH PostToolUse -> aw-cosh-hook -> AW Core -> Provider Host -> tokenless
```

Build tokenless and the adapter from the repository root, then feed the checked
COSH-shaped Hook fixture to the adapter:

```bash
cargo build --manifest-path src/tokenless/Cargo.toml --bin tokenless
cargo build --manifest-path src/aw/Cargo.toml -p aw-cosh-hook

AW_REPO_ROOT="$(pwd)"
"$AW_REPO_ROOT/src/aw/target/debug/aw-cosh-hook" \
  --manifest "$AW_REPO_ROOT/providers/tokenless/provider.toml" \
  --executable-root "$AW_REPO_ROOT/src/tokenless/target/debug" \
  --target-id local-source-poc \
  --allow-unenforced-provider \
  < "$AW_REPO_ROOT/src/aw/crates/aw-cosh-hook/fixtures/post-tool-use.json"
```

For the current fixture, the response requests a lossless replacement for the
next model context and reports an estimated reduction from 359 to 110 tokens.
`--allow-unenforced-provider` is intentionally explicit: the PoC validates
Provider declarations but does not yet enforce them with an OS sandbox.

See [Context Projection](docs/design/context-projection.md) for the data model,
user-visible behavior, receipt semantics, and current limitations. See
[COSH AW Correlation](../cosh-ng/docs/design/aw-hook-correlation.md) for
the exact Hook wire boundary.

## Planned Workspace Shape

The long-term workspace keeps Contracts, Provider hosting, system state, client
APIs, and service transport in distinct crates:

```text
src/aw/crates/
|-- aw-contracts/       # present
|-- aw-provider-host/   # present
|-- aw-core/            # present: context and Provider policy coordination
|-- aw-cosh-hook/       # present: COSH-specific boundary adapter
|-- aw-client/          # public client protocol and SDK surface
`-- aw-service/         # service transport and supervision
```

Names listed as planned are architectural destinations, not shipped crates.
