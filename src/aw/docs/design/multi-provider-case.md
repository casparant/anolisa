# Multi-Provider Case

[中文版](multi-provider-case_zh.md)

This is an end-to-end trace of three Capabilities served by two Providers at
two Agent boundaries, with every decision recorded in one verifiable Ledger
chain. It exists to answer a specific question: does the Provider abstraction
actually hold when more than one Provider and more than one authority are in
play at once?

## What the Case Covers

| Boundary | Capability | Authority | Provider | Selection |
| --- | --- | --- | --- | --- |
| PreToolUse | `security.command.inspect/v1` | Mediate | agent-sec-core | exactly one |
| PostToolUse | `security.content.inspect/v1` | Observe | agent-sec-core | all distinct Providers |
| PostToolUse | `security.code.inspect/v1` | Observe | agent-sec-core | all distinct Providers |
| PostToolUse | `context.projection.prepare/v1` | Advise | tokenless | exactly one |

Two things this exercises that a single-Provider path cannot:

- **Two Providers in one plan.** The PostToolUse plan fans out to
  agent-sec-core twice and tokenless once, in that order, and records all three
  invocations in a single Ledger row.
- **Three authorities with different failure policies.** Observe steps record a
  gap and continue. The Advise step rejects the plan. The Mediate step applies
  the mediation default. Nothing in the routing selects behavior by
  `provider_id`.

Observe precedes Advise deliberately. Facts about the original artifact are
recorded before any derived representation exists, so the data flow already
runs in the direction a future "do not compress content holding a secret"
policy would need.

## Validation Scope — Read This First

The run below was executed on Linux
(`5.10.134-011.ali5000.al8.x86_64`, cargo 1.94.1) and exercised:

- both real Provider manifests, unmodified, including digest-checked contracts
- real Provider admission, `json-map/v1` codec mapping, and process supervision
- real Core routing, plan ordering, and failure policy
- real Ledger admission, hash chain, SQLite storage, and verification

**The leaf executables were shims, not the real Providers.** `agent-sec-cli`
requires `uv` and Python 3.11.6, neither present on the validation host, and
installing a toolchain on a shared host was out of scope. The shims speak the
same native protocols the real Providers speak and their responses are
validated against the same `native_output` schemas.

So this validates the AW side of the boundary end to end. It does **not**
validate agent-sec-core's own detection rules or tokenless's own compression —
those are each component's responsibility and have their own tests.

## Reproducing It

```bash
cargo build --manifest-path src/aw/Cargo.toml \
  -p aw-cosh-hook --bin aw-cosh-hook \
  -p aw-ledger --bin aw-ledger

V=$(mktemp -d); mkdir -p "$V/bin" "$V/ledger"
```

Two shims speaking the Providers' native protocols:

```bash
cat > "$V/bin/agent-sec-cli" <<'SH'
#!/bin/sh
payload=$(cat)
case "$payload" in
  *command_inspect*) printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"deny","reasons":["shim.recursive_delete"],"findings":[{"rule_id":"shim.recursive_delete","category":"dangerous_pattern","severity":"critical","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":9,"truncated":false}' ;;
  *code_inspect*)    printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"suspicious","findings":[{"rule_id":"shim.download_exec","category":"dangerous_pattern","severity":"medium","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":48,"truncated":false,"language_detected":"bash"}' ;;
  *)                 printf '%s' '{"protocol_version":1,"disposition":"completed","verdict":"sensitive","findings":[{"rule_id":"shim.aliyun_ak","category":"secret","severity":"high","confidence":"high","count":1}],"findings_total":1,"scanned_bytes":48,"truncated":false}' ;;
esac
SH

cat > "$V/bin/tokenless" <<'SH'
#!/bin/sh
cat > /dev/null
printf '%s' '{"protocol_version":1,"output":"builds[2] compressed","disposition":"applied","reversibility":"lossless","before_tokens":120,"after_tokens":30,"tokenizer_id":"shim-v1","compressor_chain":["shim"]}'
SH

chmod +x "$V/bin/agent-sec-cli" "$V/bin/tokenless"
```

Both boundaries, one Ledger, `required` assurance:

```bash
HOOK=src/aw/target/debug/aw-cosh-hook
COMMON="--manifest-dir $PWD/providers --executable-root $V/bin
        --target-id case-host --allow-unenforced-provider
        --provider-wall-time-ms 30000
        --ledger $V/ledger --ledger-mode required"

$HOOK --event PreToolUse  $COMMON < pre.json
$HOOK --event PostToolUse $COMMON < post.json
```

`--manifest-dir` discovers `providers/<provider-id>/provider.toml` packages.
The Host admits only explicit absolute roots and never searches ambient `PATH`.

## Observed Result

### 1. PreToolUse — the gate blocks

Input command: `rm -rf / --no-preserve-root`.

```json
{"decision":"block","reason":"AW · security · blocked · shim.recursive_delete"}
```

The reason carries a rule code, not the command. `SecurityRuleId` is restricted
to `[a-z0-9._-]`, so a gate notice structurally cannot echo the argument it
refused.

### 2. PostToolUse — three invocations, two Providers

Input tool response contained an Aliyun access key and a pipe-to-shell.

```json
{"suppressOutput":true,
 "systemMessage":"AW · tokenless · estimated context 120→30 tokens · saved 75%\nAW · security · 2 findings · peak high",
 "hookSpecificOutput":{"hookEventName":"PostToolUse","updatedToolResponse":"builds[2] compressed"}}
```

One response reports both authorities: tokenless's projection and
agent-sec-core's findings. The summary counts findings without naming them —
unlike a refusal, an observation does not need to be actionable at the notice.

### 3. The chain verifies

```console
$ aw-ledger --ledger "$V/ledger" verify
verified 2 record(s); chain intact

$ aw-ledger --ledger "$V/ledger" list
     0  evt_f2b22530-...  pre_tool_use_gate   aw.ledger.pre_tool_use_gate/v1   8af7ed2a...  tool_use=tol_6666...
     1  evt_39f41a62-...  post_tool_use_plan  aw.ledger.post_tool_use_plan/v1  8d26d4a7...  tool_use=tol_6666...
2 record(s)
```

Both boundaries land in one chain under one `tool_use_id`. Record 1's
`parent.digest` equals record 0's `record_digest`, which is what `verify`
recomputed rather than read.

### 4. The plan record holds the whole multi-Provider trace

`aw-ledger body evt_39f41a62-...`, reformatted:

```json
{
  "source_artifact_id": "art_77eb2165-...",
  "source_digest": "54cf4c59387e1c67...",
  "observations": [
    { "capability": {"id": "security.content.inspect", "version": 1},
      "verdict": "sensitive",
      "findings": [{"rule_id":"shim.aliyun_ak","category":"secret","severity":"high","confidence":"high","count":1}],
      "scanned_bytes": 48, "truncated": false,
      "invocation": {"provider_id":"agent-sec-core","provider_version":"0.11.0",
                     "invocation_id":"pvi_ab9a0e47-...","disposition":"produced",
                     "output_digest":"b138bb236db9734f..."} },
    { "capability": {"id": "security.code.inspect", "version": 1},
      "verdict": "suspicious",
      "findings": [{"rule_id":"shim.download_exec","category":"dangerous_pattern","severity":"medium","confidence":"high","count":1}],
      "language_detected": "bash", "scanned_bytes": 48, "truncated": false,
      "invocation": {"provider_id":"agent-sec-core","provider_version":"0.11.0",
                     "invocation_id":"pvi_d551d7f4-...","disposition":"produced",
                     "output_digest":"e38353b71aa7e324..."} }
  ],
  "observation_gaps": [],
  "projection": {
    "candidate_offered": true, "media_type": "text/plain",
    "reversibility": "lossless", "transform_chain": ["shim"],
    "invocation": {"provider_id":"tokenless","provider_version":"0.7.14",
                   "invocation_id":"pvi_2cf7bf72-...","disposition":"produced",
                   "output_digest":"f63c8554639274a5..."}
  }
}
```

Every claim is attributable: each fact names the Capability that produced it,
the Provider and version that served it, and the invocation whose receipt backs
it. `observation_gaps` is empty here; had a scanner been missing or failed, it
would name the Capability and the reason, so a reader can tell "nothing was
found" from "nobody looked".

### 5. Nothing sensitive reached the database

Probing the raw database file for every piece of content that passed through:

```console
$ for n in 'LTAI5tSecretValue' 'rm -rf' 'no-preserve-root' \
           'builds[2] compressed' 'curl http'; do
    strings "$V/ledger/ledger.db" | grep -qF -- "$n" \
      && echo "LEAK: [$n]" || echo "OK: [$n] absent"
  done
OK: [LTAI5tSecretValue] absent
OK: [rm -rf] absent
OK: [no-preserve-root] absent
OK: [builds[2] compressed] absent
OK: [curl http] absent
```

Five needles: the secret the scanner found, the command the gate refused (twice),
the projection candidate the Advise step produced, and the dangerous pattern the
code scanner flagged. None appear anywhere in the file — not in a row, not in a
WAL page, not in a freelist remnant.

The probe reads the file with `strings`, deliberately bypassing SQL. A query
only shows what the schema exposes; this shows what is physically present.

## What This Establishes

- The Provider abstraction holds with two Providers and three authorities in one
  plan, without the plan naming either Provider.
- Observe fan-out reaches every distinct Provider; Mediate and Advise admit
  exactly one, and the difference is plan policy rather than special-casing.
- Both boundaries write into one hash chain that a reader can recompute from the
  stored bytes alone.
- Content-freedom survives a real multi-Provider run, verified against the
  database file rather than asserted from the record model.

## What It Does Not Establish

- Real `agent-sec-core` detection or real `tokenless` compression — the leaf
  executables were shims, per **Validation Scope** above.
- Concurrent-writer behavior. Both hook invocations were sequential. The
  interim writer's contention story is reasoned about in
  [AW Ledger](ledger.md#the-interim-hook-writer), not measured here.
- Provider sandboxing. `--allow-unenforced-provider` is required precisely
  because declared network and filesystem permissions are validated but not yet
  enforced by an OS sandbox.

See [AW Ledger](ledger.md) for the record model and hash chain, and
[Context Projection](context-projection.md) for the Advise data model.
