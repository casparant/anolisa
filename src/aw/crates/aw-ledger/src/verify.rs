//! Hash-chain verification.
//!
//! [`verify_chain`] walks every record in sequence order and recomputes
//! two digests per row: the body digest over the stored canonical body
//! bytes, and the record digest over the full canonical record bytes.
//! It also verifies that each record's parent link matches the previous
//! row's identity and digest. A passing verification proves the stored
//! bytes are the bytes the writer committed to and no record has been
//! inserted, deleted, or tampered with since admission.

use aw_contracts::canonical::canonical_json_v1_bytes;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::store::{LedgerStore, StoreError};

/// Failure returned when the hash chain is broken.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// Two consecutive records are not adjacent in sequence.
    #[error("sequence gap at {at}: expected {expected}, found {found}")]
    SequenceGap {
        /// Position (0-based row index) where the gap was detected.
        at: usize,
        /// Sequence the chain expected.
        expected: u64,
        /// Sequence the row declared.
        found: u64,
    },
    /// A record's parent link does not match the preceding row.
    #[error("parent link broken at sequence {sequence}")]
    ParentLinkBroken {
        /// Sequence of the record whose parent link is wrong.
        sequence: u64,
    },
    /// A record's stored body digest does not match the digest of its
    /// stored canonical body bytes.
    #[error("body digest mismatch at sequence {sequence}")]
    BodyDigestMismatch {
        /// Sequence of the record whose body digest is wrong.
        sequence: u64,
    },
    /// A record's stored record digest does not match the digest
    /// recomputed from its stored canonical record bytes.
    #[error("record digest mismatch at sequence {sequence}")]
    RecordDigestMismatch {
        /// Sequence of the record whose digest is wrong.
        sequence: u64,
    },
    /// The stored canonical record bytes do not decode to a valid
    /// record envelope.
    #[error("canonical record bytes corrupt at sequence {sequence}")]
    IntegrityBroken {
        /// Sequence of the record whose bytes are corrupt.
        sequence: u64,
    },
    /// A database error prevented verification.
    #[error("ledger database error: {0}")]
    Database(#[from] crate::StoreError),
}

/// Walks every record in sequence order and verifies the hash chain.
///
/// Returns the number of records verified on success.
///
/// # Errors
///
/// Returns [`VerifyError`] at the first invariant violation. A full
/// verification of *n* records performs *n* digest recomputations and
/// *n* − 1 parent link checks.
pub fn verify_chain(store: &LedgerStore) -> Result<usize, VerifyError> {
    let mut stmt = store
        .conn()
        .prepare(
            "SELECT id, sequence, parent_id, parent_digest,
                    body_digest, body_canonical, record_canonical, record_digest
             FROM ledger_records
             ORDER BY sequence ASC",
        )
        .map_err(StoreError::from)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RawRow {
                id: row.get(0)?,
                sequence: row.get::<_, i64>(1)? as u64,
                parent_id: row.get(2)?,
                parent_digest: row.get(3)?,
                body_digest: row.get(4)?,
                body_canonical: row.get(5)?,
                record_canonical: row.get(6)?,
                record_digest: row.get(7)?,
            })
        })
        .map_err(StoreError::from)?;

    let mut prev_id: Option<String> = None;
    let mut prev_digest: Option<String> = None;
    let mut expected_sequence: u64 = 0;
    let mut count = 0;

    for row in rows {
        let row = row.map_err(StoreError::from)?;

        // 1. Sequence continuity.
        if row.sequence != expected_sequence {
            return Err(VerifyError::SequenceGap {
                at: count,
                expected: expected_sequence,
                found: row.sequence,
            });
        }

        // 2. Parent link (skipped for genesis).
        if expected_sequence > 0 {
            let parent_ok = match (&row.parent_id, &row.parent_digest, &prev_id, &prev_digest) {
                (Some(pid), Some(pd), Some(prev_id), Some(prev_d)) => {
                    pid == prev_id && pd == prev_d
                }
                _ => false,
            };
            if !parent_ok {
                return Err(VerifyError::ParentLinkBroken {
                    sequence: row.sequence,
                });
            }
        } else if row.parent_id.is_some() {
            // Genesis must not have a parent.
            return Err(VerifyError::ParentLinkBroken { sequence: 0 });
        }

        // 3. Body digest over canonical body bytes.
        let body_digest_computed = digest_hex(&row.body_canonical);
        if body_digest_computed != row.body_digest {
            return Err(VerifyError::BodyDigestMismatch {
                sequence: row.sequence,
            });
        }

        // 4. Record digest over canonical record bytes.
        let record_digest_computed = digest_hex(&row.record_canonical);
        if record_digest_computed != row.record_digest {
            return Err(VerifyError::RecordDigestMismatch {
                sequence: row.sequence,
            });
        }

        // 5. Canonical record bytes decode to a valid envelope whose
        //    body digest matches the stored body digest column.
        verify_envelope_body_digest(&row.record_canonical, &row.body_digest, row.sequence)?;

        prev_id = Some(row.id);
        prev_digest = Some(row.record_digest);
        expected_sequence = row.sequence + 1;
        count += 1;
    }

    Ok(count)
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Decodes the stored canonical record bytes and verifies that the
/// body embedded in the envelope digests to the stored body digest.
fn verify_envelope_body_digest(
    record_canonical: &[u8],
    expected_body_digest: &str,
    sequence: u64,
) -> Result<(), VerifyError> {
    let envelope: Envelope = serde_json::from_slice(record_canonical)
        .map_err(|_| VerifyError::IntegrityBroken { sequence })?;
    let body_canonical = canonical_json_v1_bytes(&envelope.body)
        .map_err(|_| VerifyError::IntegrityBroken { sequence })?;
    let body_digest_computed = digest_hex(&body_canonical);
    if body_digest_computed != expected_body_digest {
        return Err(VerifyError::BodyDigestMismatch { sequence });
    }
    Ok(())
}

/// Minimal envelope used only to decode stored canonical record bytes
/// during verification. Only `body` is declared; serde skips the header
/// the writer also encoded.
#[derive(serde::Deserialize)]
struct Envelope {
    body: serde_json::Value,
}

struct RawRow {
    id: String,
    sequence: u64,
    parent_id: Option<String>,
    parent_digest: Option<String>,
    body_digest: String,
    body_canonical: Vec<u8>,
    record_canonical: Vec<u8>,
    record_digest: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{admit, Chain, LedgerStore};
    use serde_json::json;
    use tempfile::tempdir;

    fn append_n(store: &mut LedgerStore, chain: &mut Chain, n: usize) {
        for _ in 0..n {
            let body = json!({
                "projection": {
                    "id": "prj_00000000-0000-0000-0000-000000000000",
                    "digest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                }
            });
            let tip = chain.tip();
            let candidate = crate::tests::candidate(&tip, body);
            let admitted = admit(&tip, candidate).expect("admit");
            store.append(&admitted, None).expect("append");
            chain.extend(&admitted);
        }
    }

    #[test]
    fn empty_chain_verifies_as_zero_records() {
        let dir = tempdir().expect("temp dir");
        let store = LedgerStore::open(dir.path()).expect("store opens");
        assert_eq!(verify_chain(&store).expect("verify"), 0);
    }

    #[test]
    fn single_record_verifies() {
        let dir = tempdir().expect("temp dir");
        let mut store = LedgerStore::open(dir.path()).expect("store opens");
        let mut chain = Chain::new();
        append_n(&mut store, &mut chain, 1);
        assert_eq!(verify_chain(&store).expect("verify"), 1);
    }

    #[test]
    fn multi_record_chain_verifies() {
        let dir = tempdir().expect("temp dir");
        let mut store = LedgerStore::open(dir.path()).expect("store opens");
        let mut chain = Chain::new();
        append_n(&mut store, &mut chain, 5);
        assert_eq!(verify_chain(&store).expect("verify"), 5);
    }

    #[test]
    fn tampered_body_digest_is_detected() {
        let dir = tempdir().expect("temp dir");
        let mut store = LedgerStore::open(dir.path()).expect("store opens");
        let mut chain = Chain::new();
        append_n(&mut store, &mut chain, 1);

        // Corrupt the body_digest column.
        store
            .conn()
            .execute(
                "UPDATE ledger_records SET body_digest = ?1 WHERE sequence = 0",
                ["0000000000000000000000000000000000000000000000000000000000000000"],
            )
            .expect("corrupt");

        let result = verify_chain(&store);
        assert!(
            matches!(result, Err(VerifyError::BodyDigestMismatch { sequence: 0 })),
            "expected BodyDigestMismatch, got {result:?}"
        );
    }

    #[test]
    fn tampered_record_digest_is_detected() {
        let dir = tempdir().expect("temp dir");
        let mut store = LedgerStore::open(dir.path()).expect("store opens");
        let mut chain = Chain::new();
        append_n(&mut store, &mut chain, 1);

        store
            .conn()
            .execute(
                "UPDATE ledger_records SET record_digest = ?1 WHERE sequence = 0",
                ["0000000000000000000000000000000000000000000000000000000000000000"],
            )
            .expect("corrupt");

        let result = verify_chain(&store);
        assert!(
            matches!(
                result,
                Err(VerifyError::RecordDigestMismatch { sequence: 0 })
            ),
            "expected RecordDigestMismatch, got {result:?}"
        );
    }

    #[test]
    fn broken_parent_link_is_detected() {
        let dir = tempdir().expect("temp dir");
        let mut store = LedgerStore::open(dir.path()).expect("store opens");
        let mut chain = Chain::new();
        append_n(&mut store, &mut chain, 2);

        // Corrupt the parent_id of the second record.
        store
            .conn()
            .execute(
                "UPDATE ledger_records SET parent_id = ?1 WHERE sequence = 1",
                ["evt_00000000-0000-0000-0000-000000000000"],
            )
            .expect("corrupt");

        let result = verify_chain(&store);
        assert!(
            matches!(result, Err(VerifyError::ParentLinkBroken { sequence: 1 })),
            "expected ParentLinkBroken, got {result:?}"
        );
    }
}
