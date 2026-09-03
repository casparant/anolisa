//! Core results for one Agent Environment event.
//!
//! Every fact is paired with the receipt that produced it, so a later Ledger
//! writer never has to re-associate a Provider outcome with its invocation.

use aw_contracts::common::Digest;
use aw_contracts::context::ContextProjectionCandidate;
use aw_contracts::ids::ArtifactId;
use aw_contracts::provider::ProviderReceipt;
use serde::Serialize;

/// Advise candidate paired with the receipt for the invocation that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedProjection {
    /// Provider proposal available for a later Core adoption decision.
    ///
    /// A bypassed, denied, failed, or uncertain invocation carries no candidate
    /// even when the implementation returned transient output.
    pub candidate: Option<ContextProjectionCandidate>,
    /// Content-free terminal Provider facts safe for persistence and display.
    pub receipt: ProviderReceipt,
}

/// Core result for one observed tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolResultOutcome {
    /// Core identity allocated to the immutable source artifact.
    pub source_artifact_id: ArtifactId,
    /// SHA-256 of the original tool-result content.
    pub source_digest: Digest,
    /// Result of the single Advise context-projection step.
    pub projection: PreparedProjection,
}

impl ToolResultOutcome {
    /// Returns every accepted receipt in deterministic plan order.
    #[must_use]
    pub fn receipts(&self) -> Vec<&ProviderReceipt> {
        vec![&self.projection.receipt]
    }
}
