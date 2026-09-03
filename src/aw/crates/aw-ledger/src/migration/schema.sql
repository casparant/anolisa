-- AW Ledger Schema v1
--
-- Content-freedom: only digests, IDs, and bounded metadata are stored.
-- Raw tool content (command text, tool input/output) never enters these
-- columns; the canonical body bytes are the exact bytes the hash chain
-- commits to.

CREATE TABLE IF NOT EXISTS ledger_records (
    id              TEXT PRIMARY KEY,
    sequence        INTEGER NOT NULL UNIQUE,
    timestamp_ms    INTEGER NOT NULL,
    kind            TEXT NOT NULL,
    schema          TEXT NOT NULL,
    parent_id       TEXT,
    parent_digest   TEXT,
    body_digest     TEXT NOT NULL,
    body_canonical  BLOB NOT NULL,
    record_canonical BLOB NOT NULL,
    record_digest   TEXT NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS idx_ledger_records_sequence
    ON ledger_records(sequence);

CREATE INDEX IF NOT EXISTS idx_ledger_records_kind
    ON ledger_records(kind);

CREATE TABLE IF NOT EXISTS ledger_scope (
    record_id       TEXT NOT NULL REFERENCES ledger_records(id) ON DELETE CASCADE,
    attempt_id      TEXT,
    tool_use_id     TEXT,
    invocation_id   TEXT,
    PRIMARY KEY (record_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_ledger_scope_attempt
    ON ledger_scope(attempt_id) WHERE attempt_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ledger_scope_tool_use
    ON ledger_scope(tool_use_id) WHERE tool_use_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_ledger_scope_invocation
    ON ledger_scope(invocation_id) WHERE invocation_id IS NOT NULL;
