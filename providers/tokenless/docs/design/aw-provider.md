# Tokenless as an AW Provider

[中文版](aw-provider_zh.md)

Tokenless exposes its existing one-shot compression protocol as the first
`context.projection.prepare/v1` Provider for AW. The Provider package is a
thin declaration around the `tokenless compress` binary: Tokenless remains an
independent program and does not link to COSH or Gateway crates.

## What the Capability does

`context.projection.prepare/v1` accepts one model-visible Tool result or other
context artifact and returns a smaller candidate plus facts about how it was
produced. Its authority is `advise`: it may propose a candidate, but it cannot
decide that the candidate is sent to a model.

The PoC keeps the system contract and the implementation protocol separate:

| Boundary | Input | Output |
| --- | --- | --- |
| AW Capability | Context Artifact, boundary, and constraints | Context Projection candidate |
| Tokenless native protocol | `CompressionRequest` v1 | `CompressionResponse` v1 |

The machine-readable JSON Schemas live in
[`provider/schemas/`](../../provider/schemas/). The manifest's generic
`json-map/v1` codec maps the canonical input into `CompressionRequest`, runs
`tokenless compress`, and maps `CompressionResponse` back into the canonical
output and meters. The Host interprets only the mapping declaration; it has no
Tokenless-specific branch and never sends the complete Core invocation to the
binary.

This boundary lets Tokenless keep its stable protocol for existing adapters,
while another implementation can serve the same AW Capability with a
different native protocol.

## Why `applied` maps to `produced`

In Tokenless's native adapter protocol, `applied` means the returned `output`
is a valid replacement candidate. At the AW Core boundary, the Provider
has only prepared that candidate: Core has not yet selected it or inserted it
into a model request. The manifest therefore maps native `applied` to the Core
receipt disposition `produced`.

A later Core decision records whether the candidate was actually delivered.
This distinction prevents a generated candidate from being counted as an
effective context saving when an Agent Environment ignores it, replaces it, or
never sends the corresponding model request.

The other native dispositions map as follows:

| Tokenless disposition | Core Provider disposition | Meaning |
| --- | --- | --- |
| `applied` | `produced` | A smaller candidate exists; delivery is not yet proven |
| `dry_run`, `passthrough`, `no_savings`, `reversibility_unavailable` | `bypassed` | No candidate should replace the authoritative input |
| `timeout`, `error` | `failed` | The Provider reached a known terminal failure |

Process timeout, non-zero exit, malformed JSON, and oversized output are
Driver failures. They are handled by the generic `exec-json/v1` Driver rather
than by Tokenless-specific Gateway code.

## Package layout

A source checkout keeps the implementation, manifest, schemas, and fixtures
together:

```text
providers/tokenless/
├── provider/
│   ├── provider.toml
│   ├── schemas/
│   └── fixtures/
└── target/{debug,release}/tokenless
```

`make install`, RPM packages, and raw packages preserve the same Provider
directory under their data prefix and install the executable under their
binary prefix:

```text
<prefix>/bin/tokenless
<data-prefix>/aw/providers/tokenless/provider.toml
<data-prefix>/aw/providers/tokenless/schemas/...
```

The npm distribution remains a binary-and-Agent-adapter package. It does not
write into the host-wide Provider discovery root, so installing Tokenless from
npm alone does not register an AW Provider Host.

The embedding system supplies explicit trusted roots to the Provider Host. The
Host resolves the bare `tokenless` command only below an executable root; the
manifest cannot inherit the caller's `PATH` or environment. Schema resources
must remain below the Provider package and match the SHA-256 digests pinned in
the manifest.

The manifest disables statistics export and SLS for Provider invocations. This
keeps `ProviderReceipt` as the canonical invocation fact and avoids creating a
second accounting path. It also disables retrieval publication, so this
`advise` Capability neither opens a Stash database nor retains input content.
The manifest therefore declares no filesystem write, network, telemetry, or
retention permission.

## What the end-to-end PoC proves

The committed canonical fixture is submitted as a `CapabilityInvocation` to
the headless Provider Host. The Host admits the manifest and schema digests,
maps the request, invokes the real Tokenless binary, and returns two separate
objects:

- a transient Context Projection body for the immediate caller; and
- a content-free `ProviderReceipt` containing identity, disposition, digests,
  byte size, timing, and token estimates.

For the fixture, Tokenless reports native `applied`; the Host returns Core
`produced`, with fewer estimated prepared tokens than source tokens. The
original Tool content is absent from the durable receipt. Provider crash,
timeout, oversized output, and malformed JSON settle as bounded failure facts
after invocation acceptance rather than leaking native output into the event
ledger.

This first manifest intentionally exposes only candidate preparation.
`context.projection.commit` and `context.retrieve` require Core-owned delivery,
lease, and authorization semantics and are not advertised by this PoC.
