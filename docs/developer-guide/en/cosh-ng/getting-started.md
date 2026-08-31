# Developing cosh-ng

[中文版](../../zh/cosh-ng/getting-started.md)

This guide gets a new contributor from checkout to a focused, validated change.
Read the repository `AGENTS.md`, `src/cosh-ng/AGENTS.md`, and this page before
editing code; those files contain constraints that are intentionally not
duplicated here.

## 1. Prepare the workspace

cosh-ng is a Linux-first Rust workspace that also builds on macOS. The minimum
Rust version is 1.74, and `rust-toolchain.toml` selects stable Rust with rustfmt
and Clippy.

```bash
cd src/cosh-ng
rustup show
cargo build --workspace
```

Do not let tests mutate packages, services, or other host state. Use unit tests,
mocks, or an explicitly isolated environment.

## 2. Understand the runtime boundary

There are six crates, three main runtime processes, and one short-lived internal
audit utility:

| Area | Start reading | Boundary |
|---|---|---|
| Agent runtime | `crates/cosh-core/src/main.rs` | JSONL/registry input to provider, tools, and session state |
| Interactive terminal | `crates/cosh-shell/src/main.rs` | terminal input, PTY events, cards, and a child cosh-core process |
| Local Task gateway | `crates/cosh-gateway/src/main.rs` | local Task API, durable state, and ACP adapter entrypoints |
| Internal audit utility | `crates/cosh-platform/src/bin/cosh-audit.rs` | bounded audit status, query, trace, export, retention preview, and policy operations |
| Shared platform code | `crates/cosh-platform/src/lib.rs` | audit persistence and process-group support |
| Shared types | `crates/cosh-types/src/lib.rs` | side-effect-free audit/error types and the retained ws-ckpt wire contract |
| Gateway contracts | `crates/cosh-gateway-contracts/src/lib.rs` | side-effect-free Task, Runtime, capability, identity, and error contracts |

`cosh-shell` does not link to the other workspace crates. It launches
`cosh-core` and communicates over the versioned JSONL/control protocol. That
process boundary is a compatibility contract, not an implementation detail.
`cosh-audit` is an internal, single-purpose utility used by the bounded `/audit`
Shell surface. It is not a replacement catch-all CLI.

See [Architecture](architecture.md) for ownership and data flow.

## 3. Find the owner before editing

For `cosh-shell`, new production behavior belongs under an existing owner
directory; do not add implementation files directly under `src/`.

| Change | Primary owner | Typical test target |
|---|---|---|
| PTY, OSC, bash/zsh integration | `shell_host/` | `shell_host` |
| Input routing and multiline entry | `raw_input/`, `input/`, `slash/` | `raw_cli` or `logic` |
| Agent lifecycle and event policy | `agent/` | `logic` |
| Core adapter/control messages | `adapter/` | `protocol` |
| Approval and question cards | `approval/`, `question/`, `ui/` | `raw_cli` |
| Hooks | `hooks/` | library tests or `logic` |
| Runtime orchestration/state mutation | `runtime/` | library tests, then relevant integration target |
| Agent tools and risk rules | `tools/` | library tests and adversarial regressions |

Run the layout audit after moving or adding shell code:

```bash
crates/cosh-shell/scripts/check-layout.sh
```

## 4. Use the narrowest feedback loop

```bash
# Shared types and platform support
cargo test --locked -p cosh-types
cargo test --locked -p cosh-platform
cargo test --locked -p cosh-platform --test cosh_audit_cli

# Core
cargo test --locked -p cosh-core --lib
cargo test --locked -p cosh-core --test jsonl_protocol

# Shell: fast logic before process-heavy tests
cargo test --locked -p cosh-shell --lib
cargo test --locked -p cosh-shell --test logic
cargo test --locked -p cosh-shell --test protocol
```

Choose `raw_cli` when the behavior spawns `cosh-shell`, renders cards, or
crosses the provider handoff. Choose `shell_host` for PTY, OSC, termios,
foreground programs, or native bash/zsh behavior.

## 5. Validate the final change

Match validation to the change:

- Documentation-only changes: check links, Markdown formatting, commands, and
  bilingual parity. Rust tests and builds are unnecessary.
- Ordinary code changes: run formatting and the tests closest to the changed
  crate or behavior. Add targeted Clippy or integration checks when they can
  catch a relevant failure.
- Large or cross-cutting code changes: run full local gates, persistent ECS, or
  manual-grade validation only when the current task explicitly requests that
  depth. Otherwise CI owns broad regression coverage.

When public API or rustdoc changes, also run:

```bash
cargo doc --workspace --no-deps
```

See [Testing](testing.md) for target selection and optional gate profiles.

## 6. Keep contracts explicit

- Never reorder the retained ws-ckpt protocol enum variants without coordinating
  the daemon; their indexes remain a compatibility contract. COSH exposes no
  checkpoint command or client. Checkpoint execution belongs to a future
  State/Recovery Provider rather than `cosh-platform`.
- A cosh-core protocol change must update protocol types, both producer and
  consumer, fixtures, and protocol tests together.
- Security allow rules must tokenize first, reject shell metacharacters, and
  fail closed. Add tab, newline, and unspaced-metacharacter regressions.
- Tests must not depend on a real LLM provider or mutate host system state.
- Do not weaken assertions, inventory floors, or registered layout debt to make
  a check pass.

## Where to go next

- [Testing strategy](testing.md)
- [IPC protocols](ipc-protocol.md)
- [Security heuristics](security-heuristics.md)
- [Component contribution rules](../../../../src/cosh-ng/CONTRIBUTING.md)
