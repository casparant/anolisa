# AW Ledger

[中文版](ledger_zh.md)

The Ledger is the durable, tamper-evident record of what AW decided at each
Agent boundary. It answers one question an operator cannot otherwise answer
after the fact: *what did the system observe and decide, and can I prove the
record was not altered since?*

## Content-Freedom Is the Design Constraint

The Ledger stores bounded metadata, digests, and IDs. It never stores the tool
response, the command text, the projection candidate, or the value a rule
matched.

This is not a privacy nicety bolted on afterwards — it is the constraint that
shapes the record model. An audit log that copies the content it describes
becomes a second, less-guarded store of exactly the secrets the Observe
Capabilities were installed to find. A Ledger row that says
`shim.aliyun_ak, severity high, count 1` is actionable; a row that also
carries `LTAI5t...` is a liability.

Admission enforces this rather than trusting the writer. Every candidate body
is walked at every depth and rejected if any object carries a forbidden key:

```text
command  tool_input  tool_response  matched  content  payload
```

The check is case-insensitive and applies inside nested objects and array
elements. A body that survives admission has been mechanically checked, not
merely reviewed.

The sharpest instance is the Advise step. A `ContextProjectionCandidate`
carries `content: String` — the model-visible representation itself. The plan
record therefore keeps only the candidate's digest and bounded shape metadata:

```json
"projection": {
  "candidate_offered": true,
  "media_type": "text/plain",
  "reversibility": "lossless",
  "transform_chain": ["shim"],
  "invocation": { "output_digest": "f63c8554...", "...": "..." }
}
```

A reader who needs the representation resolves it from the Artifact store by
digest. The Ledger proves *which* candidate was meant without becoming a copy
of it.

## Record Model

Every record has a header and a body. The header is schema, the body is
governed by that schema.

```text
LedgerRecordHeader
  id            evt_<uuid>        stable record identity
  sequence      u64               monotonic, gap-free
  timestamp_ms  u64               writer wall clock
  kind          enum              event taxonomy
  schema        string            revision governing `body`
  parent        {id, digest}?     absent only at sequence 0
  body_digest   Digest            canonical JSON v1 digest of `body`
```

`parent` bundles the preceding record's identity *and* digest in one struct.
Splitting them into two optional fields would let a header reference a parent
without committing to its bytes, which is precisely the state a tamperer wants.

Two body schemas exist today, one per Core boundary:

| Kind | Schema | Records |
| --- | --- | --- |
| `pre_tool_use_gate` | `aw.ledger.pre_tool_use_gate/v1` | The Mediate gate for one pending Tool Call |
| `post_tool_use_plan` | `aw.ledger.post_tool_use_plan/v1` | The full Observe + Advise plan for one tool result |

The taxonomy has three further variants (`provider_invoked`,
`evidence_stored`, `receipt_stored`) with no writer yet. They are declared
because the enum is additive: appending a variant later does not invalidate
stored records, since each one already names its own schema revision.

## Hash Chain

Each record commits twice: to its own body, and to the previous record.

```text
record N-1                          record N
  body ──digest──► body_digest        body ──digest──► body_digest
  header+body ──digest──► D(N-1)      header.parent.digest = D(N-1)
                                      header+body ──digest──► D(N)
```

Both digests are SHA-256 over canonical JSON v1 — recursively sorted keys,
compact separators, UTF-8. Determinism is what makes recomputation meaningful:
a reader re-encodes the stored value and must arrive at the same bytes.

`verify_chain` walks records in sequence order and, per record:

1. checks the sequence is adjacent to the previous one,
2. checks `parent` matches the previous record's identity and digest,
3. recomputes the body digest from the stored canonical body bytes,
4. recomputes the record digest from the stored canonical record bytes,
5. decodes the canonical record bytes and re-digests the embedded body.

Step 5 is the one that is easy to omit and expensive to omit. Steps 3 and 4
each compare a stored digest against bytes stored beside it, so an attacker who
rewrites `body_canonical` and `body_digest` together passes both. Step 5 catches
that, because `record_canonical` still contains the original body.

Cost is linear in record count. There is no incremental or checkpointed mode
yet, so a very large Ledger will want one.

## Storage

SQLite, one file per Ledger root, opened in WAL mode. Records live in a STRICT
table so the database enforces column types rather than trusting the writer.

Each append runs in an `IMMEDIATE` transaction that inserts the record row and
its scope row together, and the in-memory chain tip advances only after the
commit succeeds. A failed append therefore leaves the chain exactly where it
was, rather than leaving a tip that points at a row nobody stored.

`sequence` carries a `UNIQUE` constraint. Admission already rejects a
non-monotonic sequence, so this is defense in depth: it is the constraint that
holds when two writers each believe they own the tip.

Trace scope lives in a side table (`ledger_scope`) keyed by record ID, with
partial indexes on each axis. Keeping it out of the records table means the
columns the hash chain recomputes stay narrow, and a scope index can be added
later without touching a single committed byte.

> **Reading the database requires SQLite 3.37 or newer.** STRICT tables were
> introduced there. Hosts that ship an older `sqlite3` — including its Python
> module — refuse the schema outright. Use the `aw-ledger` binary, which links
> its own bundled SQLite.

## Bounded Queries

Every read path filters on an indexed column and returns at most what the index
selects. There is deliberately no unbounded scan.

| Accessor | Index used |
| --- | --- |
| `record_by_id` | primary key |
| `events_by_kind` | `idx_ledger_records_kind` |
| `events_for_attempt` | `idx_ledger_scope_attempt` |
| `record_body_bytes` | primary key |

The first three return `StoredRecord`, which carries the header, the trace
scope, and the digests — but not the body blob. Fetching the body is a separate,
explicit call. The common case is filtering, and filtering should not page
record bodies into memory.

## The Interim Hook Writer

`aw-cosh-hook` can write records itself, behind `--ledger` and `--ledger-mode`.
The module is named *interim* because the writer belongs in a daemon that owns
the database for the whole machine, and that daemon does not exist yet.

What the hook-side writer can promise: two concurrent hook processes contend on
one SQLite file, and WAL plus the `IMMEDIATE` transaction plus the `sequence`
`UNIQUE` constraint mean the loser of that race **fails its append rather than
corrupting the chain**. That is safe but lossy.

`--ledger-mode` is how a caller says whether losing a record matters:

| Mode | On append failure |
| --- | --- |
| `correlated` (default) | The boundary proceeds. The decision stands; the Ledger does not claim it. |
| `required` | The boundary fails. On PreToolUse the non-zero exit is what makes COSH fail closed, so an unrecorded gate blocks rather than passes. |

`correlated` is not a silent fallback. `CoshHookRun.ledger_unavailable` states
that a configured writer failed, which is a different fact from no writer being
configured, and the `LedgerUnavailable` variants already present in
`ObservationGapReason` and `GateDegradation` exist to express it downstream.

The append runs **before** the hook response is written. Ordering it the other
way would make `required` meaningless: COSH would already have been told what
to do.

## Inspecting a Ledger

The `aw-ledger` binary is a development and diagnostic surface. It is not
packaged as a public CLI and its command syntax is not a compatibility
contract.

```bash
cargo build --manifest-path src/aw/Cargo.toml -p aw-ledger --bin aw-ledger
LEDGER=src/aw/target/debug/aw-ledger

# Recompute every digest and parent link.
"$LEDGER" --ledger /path/to/ledger verify

# One line per record: sequence, id, kind, schema, record digest, tool use.
"$LEDGER" --ledger /path/to/ledger list
"$LEDGER" --ledger /path/to/ledger list --kind post-tool-use-plan
"$LEDGER" --ledger /path/to/ledger list --attempt atm_<uuid>

# Exactly the bytes that were stored.
"$LEDGER" --ledger /path/to/ledger body evt_<uuid>
```

`body` exists so content-freedom can be audited rather than asserted. Whatever
it prints is what the Ledger holds, so an operator can grep for content that
should not be there instead of trusting a claim that it is not.

`list` with no filter unions the per-kind queries and re-sorts by sequence. The
store exposes no unbounded scan, and adding one for a convenience view would
have undermined the reason the queries are bounded.

## Crate Boundaries

`aw-ledger` depends only on `aw-contracts` and must not learn Core outcome
types.

```text
aw-cosh-hook ──► aw-ledger ──► aw-contracts
      │                            ▲
      └──► aw-core ────────────────┘
```

Record body schemas live in `aw-contracts` because they are versioned
Contracts. The projections from a Core outcome into those bodies live in
`aw-core` because Core owns the outcome — `ToolResultOutcome::ledger_body` and
`ToolCallDecision::ledger_body`. `aw-ledger` is a dev-dependency of `aw-core`
so those projections can be proven content-free against real admission rather
than by inspection.

## Current Limitations

- **No daemon writer.** The hook process is the writer. Concurrent hooks lose
  appends under `correlated` and fail the boundary under `required`.
- **No incremental verification.** `verify_chain` is linear in record count.
- **No retention or compaction.** Records accumulate without bound.
- **No signature.** The chain is tamper-*evident* to a reader who has the
  bytes, not tamper-*proof* against someone who can rewrite the whole file and
  recompute every digest. Anchoring the tip outside the file is future work.
- **Three taxonomy variants have no writer.** `provider_invoked`,
  `evidence_stored`, and `receipt_stored` are declared but unwritten.

See [Multi-Provider Case](multi-provider-case.md) for an end-to-end trace of
three Capabilities across two Providers recorded at both boundaries.
