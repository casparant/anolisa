// SPDX-License-Identifier: Apache-2.0
//! Append-only JSONL trajectory recorder (design §6.2.8).
//!
//! v0.1 ships the recorder interface and a working JSONL writer; replay
//! is parked for v0.2 (see roadmap Phase 4). Every event line is one
//! JSON object terminated by `\n`, so logs are tail-friendly.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{AnvilError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrajectoryEventKind {
    Create,
    Start,
    Pause,
    Checkpoint,
    Reset,
    Destroy,
    #[serde(rename = "tool-call")]
    ToolCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryEvent {
    pub instance_id: Uuid,
    pub sequence: u64,
    pub event_kind: TrajectoryEventKind,
    pub timestamp: DateTime<Utc>,
    /// SHA-256 hex digest of the request payload. Plaintext args are
    /// only allowed when the policy explicitly opts in (see
    /// [`crate::policy::PolicyTrajectory::record_args_plaintext`]); the
    /// recorder itself never stores plaintext.
    #[serde(default)]
    pub args_hash: Option<String>,
    #[serde(default)]
    pub result_hash: Option<String>,
}

impl TrajectoryEvent {
    pub fn new(instance_id: Uuid, sequence: u64, event_kind: TrajectoryEventKind) -> Self {
        Self {
            instance_id,
            sequence,
            event_kind,
            timestamp: Utc::now(),
            args_hash: None,
            result_hash: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrajectoryRecorder {
    base_dir: PathBuf,
}

impl TrajectoryRecorder {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Append `event` to `{base_dir}/{instance_id}.jsonl`. Creates the
    /// directory + log file on first call. Errors are wrapped in
    /// [`AnvilError::TrajectoryWriteError`] so callers can apply a
    /// per-instance circuit breaker without losing context.
    pub fn record(&self, event: &TrajectoryEvent) -> Result<()> {
        fs::create_dir_all(&self.base_dir).map_err(|source| AnvilError::TrajectoryWriteError {
            instance_id: event.instance_id.to_string(),
            source,
        })?;
        let path = self.log_path(event.instance_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| AnvilError::TrajectoryWriteError {
                instance_id: event.instance_id.to_string(),
                source,
            })?;
        let line = serde_json::to_vec(event)?;
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|source| AnvilError::TrajectoryWriteError {
                instance_id: event.instance_id.to_string(),
                source,
            })?;
        tracing::debug!(
            instance = %event.instance_id,
            seq = event.sequence,
            kind = ?event.event_kind,
            "trajectory event recorded"
        );
        Ok(())
    }

    /// Read events filtered by `[from_seq, to_seq]` (both inclusive,
    /// either bound optional). Lines that fail to parse are dropped
    /// with a warning so a single corrupt line never blocks tail/replay.
    pub fn read_log(
        &self,
        instance_id: Uuid,
        from_seq: Option<u64>,
        to_seq: Option<u64>,
    ) -> Result<Vec<TrajectoryEvent>> {
        let path = self.log_path(instance_id);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(AnvilError::TrajectoryWriteError {
                    instance_id: instance_id.to_string(),
                    source: e,
                });
            }
        };
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| AnvilError::TrajectoryWriteError {
                instance_id: instance_id.to_string(),
                source,
            })?;
            if line.is_empty() {
                continue;
            }
            let event: TrajectoryEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed trajectory line");
                    continue;
                }
            };
            if from_seq.map(|f| event.sequence < f).unwrap_or(false) {
                continue;
            }
            if to_seq.map(|t| event.sequence > t).unwrap_or(false) {
                continue;
            }
            out.push(event);
        }
        Ok(out)
    }

    /// SHA-256 hex digest helper for caller-side hashing of args /
    /// result payloads before they are stored in
    /// [`TrajectoryEvent::args_hash`] / [`TrajectoryEvent::result_hash`].
    pub fn compute_hash(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let digest = hasher.finalize();
        let mut s = String::with_capacity(digest.len() * 2);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    fn log_path(&self, instance_id: Uuid) -> PathBuf {
        self.base_dir.join(format!("{instance_id}.jsonl"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_round_trip() {
        let tmp = tempfile::tempdir().expect("tmp");
        let rec = TrajectoryRecorder::new(tmp.path().to_path_buf());
        let id = Uuid::new_v4();
        for seq in 0u64..5 {
            let mut e = TrajectoryEvent::new(id, seq, TrajectoryEventKind::ToolCall);
            e.args_hash = Some(TrajectoryRecorder::compute_hash(b"hello"));
            rec.record(&e).expect("record");
        }
        let all = rec.read_log(id, None, None).expect("read");
        assert_eq!(all.len(), 5);

        let win = rec.read_log(id, Some(2), Some(3)).expect("read");
        assert_eq!(win.len(), 2);
        assert_eq!(win[0].sequence, 2);
        assert_eq!(win[1].sequence, 3);
    }

    #[test]
    fn missing_log_returns_empty() {
        let tmp = tempfile::tempdir().expect("tmp");
        let rec = TrajectoryRecorder::new(tmp.path().to_path_buf());
        let empty = rec.read_log(Uuid::new_v4(), None, None).expect("ok");
        assert!(empty.is_empty());
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc") known vector
        let h = TrajectoryRecorder::compute_hash(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
