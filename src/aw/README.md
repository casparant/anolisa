# AW

[中文版](README_zh.md)

AW gives Agent workloads shared system capabilities without coupling those
capabilities to one Agent or one user interface. This workspace owns the public
Contracts and headless runtime mechanisms used by Agent Environments, IDEs,
workflow engines, and individual Providers.

The current implementation is deliberately small:

| Crate | Responsibility |
| --- | --- |
| `aw-contracts` | Transport-independent identities and versioned Provider Contracts |
| `aw-provider-host` | Provider discovery, admission, capability projection, bounded invocation, and diagnostics |

COSH is the default interactive Agent Environment in the product architecture.
The current PoC proves the generic Host with tokenless; it does not yet change
COSH's normal launch flow. ACP currently ships with COSH because it is one of
COSH's Agent connection protocols. A shared integration is warranted only when
another Agent Environment needs the same implementation.

## Dependency Direction

```text
Agent Environment / headless caller
               │
               ▼
        aw-contracts
               ▲
               │
    aw-provider-host ──────► Provider package
                                  providers/<id>/
```

`aw-contracts` contains no transport, process management, persistence, or
Provider implementation. `aw-provider-host` may depend on the Contracts,
but neither crate may depend on COSH.

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
  --manifest /absolute/path/to/providers/tokenless/provider/provider.toml \
  --executable-root /absolute/path/to/tokenless/target/debug
```

Use `list` to print the Runtime Capability Graph and `invoke` to submit a
versioned `CapabilityInvocation` JSON document. The Host admits only explicit
absolute manifest roots and does not search ambient `PATH`.

## Planned Workspace Shape

The long-term workspace keeps Contracts, Provider hosting, system state, client
APIs, and service transport in distinct crates:

```text
src/aw/crates/
├── aw-contracts/       # present
├── aw-provider-host/   # present
├── aw-core/            # authoritative identity and state coordination
├── aw-client/          # public client protocol and SDK surface
└── aw-service/         # service transport and supervision
```

Names listed as planned are architectural destinations, not shipped crates.
