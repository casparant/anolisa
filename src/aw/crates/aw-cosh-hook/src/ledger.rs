//! Interim hook-side Ledger writer.
//!
//! The Ledger writer belongs in a daemon that owns the database for the whole
//! machine. Until that exists, the hook process opens the store itself, which
//! is why this module is named for what it can actually promise.
//!
//! Two concurrent hook processes contend on the same SQLite file. WAL mode and
//! the `IMMEDIATE` transaction mean the loser of that race fails its append
//! rather than corrupting the chain — the sequence UNIQUE constraint is the
//! backstop. That is safe but lossy, and [`LedgerAssurance`] is how a caller
//! says whether losing a record is acceptable.

use std::path::PathBuf;

use aw_contracts::common::Digest;
use aw_contracts::ids::{AttemptId, LedgerEventId, ToolUseId};
use aw_contracts::ledger::{
    LedgerEventKind, LedgerTraceScope, LEDGER_POST_TOOL_USE_PLAN_SCHEMA,
    LEDGER_PRE_TOOL_USE_GATE_SCHEMA,
};
use aw_ledger::{LedgerSink, LedgerStore, SinkError};
use serde::Serialize;

/// What the caller requires of a durable Ledger append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerAssurance {
    /// Record the fact, but let the boundary proceed if the append fails.
    ///
    /// The hook response still reflects what Core decided. The Ledger simply
    /// does not claim the fact, which is the distinction
    /// `ObservationGapReason::LedgerUnavailable` exists to express.
    Correlated,
    /// Fail the boundary when the append fails.
    ///
    /// Use this when an unrecorded decision is worse than a blocked Tool Call.
    /// On PreToolUse the resulting non-zero exit is what makes COSH fail closed.
    Required,
}

/// Where the interim writer stores records and how strict it is.
#[derive(Debug, Clone)]
pub struct LedgerSpec {
    /// Directory that holds `ledger.db`. Created on first use.
    pub root: PathBuf,
    /// Whether a failed append fails the boundary.
    pub assurance: LedgerAssurance,
}

/// Content-free summary of one record this hook appended.
///
/// Enough for an operator to find the row and re-verify the chain from it,
/// and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoshLedgerRecord {
    /// Identity of the appended record.
    pub event_id: LedgerEventId,
    /// Sequence the record occupies in the chain.
    pub sequence: u64,
    /// Digest of the canonical record bytes.
    pub record_digest: Digest,
}

/// Failure returned when a required Ledger append did not settle.
#[derive(Debug, thiserror::Error)]
#[error("ledger append failed: {0}")]
pub struct LedgerWriteError(#[from] SinkError);

/// Appends one boundary record and returns its content-free summary.
///
/// `Ok(None)` means the append failed under [`LedgerAssurance::Correlated`]:
/// the caller should treat the fact as unclaimed and continue.
///
/// # Errors
///
/// Returns [`LedgerWriteError`] only under [`LedgerAssurance::Required`].
pub(crate) fn append_record<T: Serialize>(
    spec: &LedgerSpec,
    kind: LedgerEventKind,
    body: &T,
    scope: &LedgerTraceScope,
) -> Result<Option<CoshLedgerRecord>, LedgerWriteError> {
    let schema = match kind {
        LedgerEventKind::PostToolUsePlan => LEDGER_POST_TOOL_USE_PLAN_SCHEMA,
        LedgerEventKind::PreToolUseGate => LEDGER_PRE_TOOL_USE_GATE_SCHEMA,
        // The hook writes only the two boundary records it owns. A future
        // recorder for the remaining kinds names its own schema.
        other => {
            debug_assert!(false, "hook does not write {other:?} records");
            return Ok(None);
        }
    };

    match try_append(spec, kind, schema, body, scope) {
        Ok(record) => Ok(Some(record)),
        Err(error) => match spec.assurance {
            LedgerAssurance::Required => Err(LedgerWriteError(error)),
            LedgerAssurance::Correlated => Ok(None),
        },
    }
}

fn try_append<T: Serialize>(
    spec: &LedgerSpec,
    kind: LedgerEventKind,
    schema: &str,
    body: &T,
    scope: &LedgerTraceScope,
) -> Result<CoshLedgerRecord, SinkError> {
    let store = LedgerStore::open(&spec.root)?;
    let mut sink = LedgerSink::new(store);
    let value = serde_json::to_value(body).expect("ledger body types always serialize");
    let admitted = sink.record(kind, schema, value, Some(scope))?;
    Ok(CoshLedgerRecord {
        event_id: admitted.header.id.clone(),
        sequence: admitted.header.sequence,
        record_digest: admitted.record_digest.clone(),
    })
}

/// Builds the trace scope every hook-written record carries.
pub(crate) fn trace_scope(
    tool_use_id: &ToolUseId,
    attempt_id: Option<&AttemptId>,
) -> LedgerTraceScope {
    LedgerTraceScope {
        attempt_id: attempt_id.cloned(),
        tool_use_id: Some(tool_use_id.clone()),
        invocation_id: None,
    }
}
