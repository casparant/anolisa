//! Ledger record admission.
//!
//! Admission is the single validation boundary between an incoming record
//! and the chain state. It enforces:
//!
//! 1. the schema is non-empty,
//! 2. the body respects content-freedom (no raw tool content),
//! 3. the sequence is monotonic and gap-free relative to the tip,
//! 4. the parent link matches the current tip identity and digest,
//! 5. the header's committed body digest matches the canonical encoding,
//! 6. the record digest commits to the full canonical record bytes.

use aw_contracts::canonical::canonical_json_v1_bytes;
use aw_contracts::common::Digest;
use aw_contracts::ledger::LedgerRecordHeader;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::chain::ChainTip;

/// Failure returned when a candidate record is rejected at admission.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdmissionError {
    /// Candidate declared an empty schema revision.
    #[error("ledger record schema must not be empty")]
    EmptySchema,
    /// Candidate's sequence does not equal `tip.sequence + 1` (or `0` at genesis).
    #[error("ledger record sequence must be {expected}, got {actual}")]
    SequenceMismatch {
        /// Sequence the chain requires next.
        expected: u64,
        /// Sequence the candidate declared.
        actual: u64,
    },
    /// Candidate's parent link does not match the current chain tip.
    #[error("ledger record parent link does not match chain tip")]
    ParentMismatch,
    /// Candidate's header commits to a body digest that does not match the
    /// canonical encoding of the supplied body.
    #[error("ledger record body digest does not match canonical body bytes")]
    BodyDigestMismatch,
    /// Candidate's body contains a forbidden content-bearing key.
    #[error("ledger record body violates content-freedom at {path}: forbidden key `{key}`")]
    ContentForbidden {
        /// JSON-pointer-style path to the offending object.
        path: String,
        /// Forbidden key encountered.
        key: String,
    },
}

/// One candidate record offered to the Ledger.
///
/// Carries the caller-supplied header and body. The caller is responsible
/// for filling `header.body_digest`; admission verifies it matches the
/// canonical body bytes.
#[derive(Debug, Clone)]
pub struct CandidateRecord {
    /// Record header with sequence, parent, and committed body digest.
    pub header: LedgerRecordHeader,
    /// Typed payload body governed by `header.schema`.
    pub body: Value,
}

/// One admitted record ready for storage.
///
/// Carries the canonical bytes the chain committed to plus both digests so
/// the storage layer does not have to recompute them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRecord {
    /// The candidate's header, unchanged by admission.
    pub header: LedgerRecordHeader,
    /// The candidate's body, unchanged by admission.
    pub body: Value,
    /// Canonical JSON v1 bytes of the body.
    pub body_canonical: Vec<u8>,
    /// Digest of the canonical body bytes.
    pub body_digest: Digest,
    /// Canonical JSON v1 bytes of the full record (header + body).
    pub record_canonical: Vec<u8>,
    /// Digest of the full canonical record — the value the next record
    /// commits to via its parent link.
    pub record_digest: Digest,
}

/// Validates one candidate against the current chain tip and returns the
/// admitted record carrying the canonical bytes the storage layer must
/// persist unchanged.
///
/// # Errors
///
/// Returns [`AdmissionError`] when the candidate fails any of the
/// admission invariants listed in the module docs.
pub fn admit(
    tip: &ChainTip<'_>,
    candidate: CandidateRecord,
) -> Result<AdmittedRecord, AdmissionError> {
    if candidate.header.schema.is_empty() {
        return Err(AdmissionError::EmptySchema);
    }
    check_content_freedom(&candidate.body)?;

    let expected_sequence = if tip.id.is_none() {
        0
    } else {
        tip.sequence + 1
    };
    if candidate.header.sequence != expected_sequence {
        return Err(AdmissionError::SequenceMismatch {
            expected: expected_sequence,
            actual: candidate.header.sequence,
        });
    }

    check_parent_link(tip, &candidate.header)?;

    let body_canonical = canonical_json_v1_bytes(&candidate.body)
        .expect("canonical encoding of admitted body cannot fail");
    let body_digest = digest_bytes(&body_canonical);
    if body_digest != candidate.header.body_digest {
        return Err(AdmissionError::BodyDigestMismatch);
    }

    let full_value = serde_json::to_value(AdmittedEnvelope {
        header: &candidate.header,
        body: &candidate.body,
    })
    .expect("admitted record must serialize");
    let record_canonical = canonical_json_v1_bytes(&full_value)
        .expect("canonical encoding of admitted record cannot fail");
    let record_digest = digest_bytes(&record_canonical);

    Ok(AdmittedRecord {
        header: candidate.header,
        body: candidate.body,
        body_canonical,
        body_digest,
        record_canonical,
        record_digest,
    })
}

fn check_parent_link(
    tip: &ChainTip<'_>,
    header: &LedgerRecordHeader,
) -> Result<(), AdmissionError> {
    match (tip.id, tip.digest, header.parent.as_ref()) {
        (None, None, None) => Ok(()),
        (Some(tip_id), Some(tip_digest), Some(parent))
            if parent.id == *tip_id && parent.digest == *tip_digest =>
        {
            Ok(())
        }
        _ => Err(AdmissionError::ParentMismatch),
    }
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    let hex = format!("{:x}", Sha256::digest(bytes));
    Digest::parse(hex).expect("sha2 output is always a valid lowercase hex digest")
}

/// Serialization envelope used to compute the record digest. The canonical
/// bytes cover the header and the body as one document; this struct exists
/// only so `serde_json::to_value` produces one combined object.
#[derive(serde::Serialize)]
struct AdmittedEnvelope<'a> {
    header: &'a LedgerRecordHeader,
    body: &'a Value,
}

pub(crate) mod content_freedom {
    //! Keys the Ledger refuses inside any body object.
    //!
    //! These identify raw tool content the Ledger must never store: the
    //! command a tool ran, the input it received, the raw response it
    //! produced, the matched string a rule flagged, the source artifact
    //! text, or an opaque payload dump.

    /// Forbidden keys checked at every depth of the body tree.
    pub(crate) const FORBIDDEN_KEYS: &[&str] = &[
        "command",
        "tool_input",
        "tool_response",
        "matched",
        "content",
        "payload",
    ];

    use serde_json::Value;

    use crate::AdmissionError;

    /// Walks the body and rejects any object that contains a forbidden key,
    /// at any depth.
    pub(crate) fn check(body: &Value) -> Result<(), AdmissionError> {
        visit(body, "")
    }

    fn visit(value: &Value, path: &str) -> Result<(), AdmissionError> {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    let lower = key.to_ascii_lowercase();
                    if FORBIDDEN_KEYS.iter().any(|&forbidden| forbidden == lower) {
                        return Err(AdmissionError::ContentForbidden {
                            path: if path.is_empty() {
                                "/".to_owned()
                            } else {
                                path.to_owned()
                            },
                            key: key.clone(),
                        });
                    }
                    let child_path = format!("{path}/{key}");
                    visit(nested, &child_path)?;
                }
                Ok(())
            }
            Value::Array(values) => {
                for (index, nested) in values.iter().enumerate() {
                    let child_path = format!("{path}[{index}]");
                    visit(nested, &child_path)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn check_content_freedom(body: &Value) -> Result<(), AdmissionError> {
    content_freedom::check(body)
}
