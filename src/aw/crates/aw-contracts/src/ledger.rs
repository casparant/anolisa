#![forbid(unsafe_code)]
//! Versioned Ledger Contracts shared by every writer and reader.
//!
//! The Ledger is the durable append-only record of every AW boundary event
//! worth auditing: plan snapshots, Observe evidence, Mediate credentials,
//! Provider receipts, and their hash chain. This module owns only the
//! schema-shaped types and the event taxonomy. Storage, admission, hash-chain
//! verification, and queries live in `aw-ledger`.
//!
//! Content-freedom rule: Ledger records carry bounded metadata, digests, and
//! IDs only. Raw tool input, tool output, and command text are never stored
//! — readers reconstruct facts from the referenced Artifact and Provider
//! receipts.

use serde::{Deserialize, Serialize};

use crate::common::Digest;
use crate::ids::{
    ArtifactId, AttemptId, LedgerCredentialId, LedgerEventId, LedgerEvidenceId, LedgerProjectionId,
    ProviderInvocationId, ToolUseId,
};

/// Taxonomy of events the Ledger records.
///
/// Variants are additive: a later release can append a variant without
/// invalidating older records, because every stored event already names its
/// schema revision through `LedgerRecordHeader::schema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEventKind {
    /// The plan resolved for one PostToolUse boundary, naming which
    /// Capabilities run and in which order.
    PostToolUsePlan,
    /// The PreToolUse Mediate gate produced a credential (block, ask, allow,
    /// or warn).
    PreToolUseGate,
    /// A Provider invocation completed and its receipt was admitted.
    ProviderInvoked,
    /// An Observe evidence bundle was attached to an existing plan event.
    EvidenceStored,
    /// A Provider receipt was attached to an existing plan event.
    ReceiptStored,
}

/// Header fields shared by every Ledger record regardless of its payload.
///
/// The header commits to the payload digest; the hash chain commits to the
/// previous record's digest. Together they form a tamper-evident sequence that
/// a reader can recompute without the body bytes in memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRecordHeader {
    /// Stable identity of this record.
    pub id: LedgerEventId,
    /// Monotonic, gap-free sequence number within the Ledger.
    pub sequence: u64,
    /// Wall-clock timestamp, milliseconds since the Unix epoch, observed by
    /// the writer at append time.
    pub timestamp_ms: u64,
    /// Which event taxonomy entry this record records.
    pub kind: LedgerEventKind,
    /// Schema revision governing `body`. The hash chain treats this as opaque
    /// text; a reader uses it to pick a deserializer.
    pub schema: String,
    /// Parent link committing to the immediately preceding record. Absent
    /// only on the genesis record at sequence zero. Bundling ID and digest
    /// keeps the header from referencing one without the other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<LedgerParent>,
    /// Canonical JSON v1 digest of this record's body.
    pub body_digest: Digest,
}

/// A link to the immediately preceding Ledger record.
///
/// Both the identity and the digest of that record travel together so a
/// reader can recompute the hash chain by fetching one parent at a time and
/// verifying the bytes it actually stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerParent {
    /// Identity of the preceding record.
    pub id: LedgerEventId,
    /// Digest of the preceding record's canonical bytes.
    pub digest: Digest,
}

/// Identity-bearing references for the payload bodies stored alongside the
/// Ledger. Each variant pins one body by its typed identity plus the digest
/// the writer committed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LedgerBodyRef {
    /// A Capability plan projection snapshot.
    Projection {
        /// Projection identity.
        id: LedgerProjectionId,
        /// Canonical JSON v1 digest of the projection body.
        digest: Digest,
    },
    /// An Observe evidence bundle.
    Evidence {
        /// Evidence identity.
        id: LedgerEvidenceId,
        /// Canonical JSON v1 digest of the evidence body.
        digest: Digest,
    },
    /// A Mediate gate credential.
    Credential {
        /// Credential identity.
        id: LedgerCredentialId,
        /// Canonical JSON v1 digest of the credential body.
        digest: Digest,
    },
    /// A Provider invocation receipt already recorded by the Host.
    Receipt {
        /// Provider invocation identity.
        invocation: ProviderInvocationId,
        /// Canonical JSON v1 digest of the receipt body.
        digest: Digest,
    },
    /// A source artifact referenced by an Observe or Mediate finding.
    Artifact {
        /// Artifact identity.
        id: ArtifactId,
        /// Canonical JSON v1 digest of the artifact metadata envelope.
        digest: Digest,
    },
}

/// Stable scope keys recorded with a Ledger event so a reader can filter the
/// trace by execution, attempt, or tool call without touching the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerTraceScope {
    /// Attempt this event contributes to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<AttemptId>,
    /// Tool use this event is about, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<ToolUseId>,
    /// Provider invocation this event records or references, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<ProviderInvocationId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_json_v1_bytes;

    fn empty_digest() -> Digest {
        Digest::parse("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
            .expect("empty SHA-256 parses")
    }

    #[test]
    fn event_kind_round_trips_through_snake_case_json() {
        let cases = [
            (LedgerEventKind::PostToolUsePlan, "\"post_tool_use_plan\""),
            (LedgerEventKind::PreToolUseGate, "\"pre_tool_use_gate\""),
            (LedgerEventKind::ProviderInvoked, "\"provider_invoked\""),
            (LedgerEventKind::EvidenceStored, "\"evidence_stored\""),
            (LedgerEventKind::ReceiptStored, "\"receipt_stored\""),
        ];
        for (kind, expected) in cases {
            let encoded = serde_json::to_string(&kind).expect("kind serializes");
            assert_eq!(encoded, expected);
            let decoded: LedgerEventKind =
                serde_json::from_str(&encoded).expect("kind deserializes");
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn unknown_event_kind_is_rejected() {
        let result = serde_json::from_str::<LedgerEventKind>("\"future_variant\"");
        assert!(result.is_err(), "unknown variants must fail closed");
    }

    #[test]
    fn body_ref_tag_is_stable_and_digests_match() {
        let projection = LedgerBodyRef::Projection {
            id: LedgerProjectionId::new(),
            digest: empty_digest(),
        };
        let encoded = serde_json::to_string(&projection).expect("body ref serializes");
        assert!(
            encoded.contains("\"kind\":\"projection\""),
            "tag must be stable for schema readers: {encoded}"
        );
        let decoded: LedgerBodyRef = serde_json::from_str(&encoded).expect("body ref deserializes");
        assert_eq!(decoded, projection);
    }

    #[test]
    fn record_header_digest_is_over_canonical_bytes() {
        // Construct one header, encode it canonically, and re-decode. The
        // bytes we commit to must be the same bytes a reader re-digests.
        let header = LedgerRecordHeader {
            id: LedgerEventId::new(),
            sequence: 7,
            timestamp_ms: 1_725_300_000_000,
            kind: LedgerEventKind::PreToolUseGate,
            schema: "aw.ledger.pre_tool_use_gate/v1".to_owned(),
            parent: Some(LedgerParent {
                id: LedgerEventId::new(),
                digest: Digest::parse(
                    "0000000000000000000000000000000000000000000000000000000000000000",
                )
                .expect("zero digest parses"),
            }),
            body_digest: empty_digest(),
        };
        let value = serde_json::to_value(&header).expect("header becomes a JSON value");
        let canonical = canonical_json_v1_bytes(&value).expect("canonical encoding succeeds");
        let decoded: LedgerRecordHeader =
            serde_json::from_slice(&canonical).expect("canonical header round-trips");
        assert_eq!(decoded, header);
    }
}
