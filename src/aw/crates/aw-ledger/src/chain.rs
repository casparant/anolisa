//! Append-only hash-chain state for the Ledger.
//!
//! The chain is an in-memory view of the Ledger tip. Every admitted record
//! advances it; the tip carries everything the next candidate needs to
//! prove continuity — the sequence number, the tip identity, and the
//! digest of the tip's canonical bytes.

use aw_contracts::common::Digest;
use aw_contracts::ids::LedgerEventId;

use crate::AdmittedRecord;

/// In-memory view of the Ledger tip.
///
/// An empty chain represents the pre-genesis state. After the first record
/// is appended, `sequence`, `tip_id`, and `tip_digest` track the most
/// recent record so subsequent candidates can prove continuity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    sequence: u64,
    tip_id: Option<LedgerEventId>,
    tip_digest: Option<Digest>,
}

/// Read-only snapshot of the chain tip handed to admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainTip<'a> {
    /// Sequence of the last admitted record, or zero when the chain is empty.
    pub sequence: u64,
    /// Identity of the last admitted record, or `None` before genesis.
    pub id: Option<&'a LedgerEventId>,
    /// Canonical-bytes digest of the last admitted record, or `None` before genesis.
    pub digest: Option<&'a Digest>,
}

impl Chain {
    /// Returns an empty chain with no records.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sequence: 0,
            tip_id: None,
            tip_digest: None,
        }
    }

    /// Snapshot of the current tip, used to validate the next candidate.
    #[must_use]
    pub fn tip(&self) -> ChainTip<'_> {
        ChainTip {
            sequence: self.sequence,
            id: self.tip_id.as_ref(),
            digest: self.tip_digest.as_ref(),
        }
    }

    /// Advances the chain with one admitted record.
    ///
    /// The caller must have validated the record against the current tip
    /// before calling this — see [`crate::admit`]. `extend` is infallible
    /// because it trusts admitted records to already be well-formed.
    pub fn extend(&mut self, record: &AdmittedRecord) {
        self.sequence = record.header.sequence;
        self.tip_id = Some(record.header.id.clone());
        self.tip_digest = Some(record.record_digest.clone());
    }
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}
