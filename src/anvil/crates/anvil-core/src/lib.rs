// SPDX-License-Identifier: Apache-2.0
//! anvil-core: shared types and v0.1 in-memory implementations for the
//! anvil sandbox-orchestration daemon.
//!
//! This crate intentionally has no I/O surface beyond JSON/TOML on local
//! filesystems. Network/UDS surfaces are implemented in the `anvil` daemon
//! crate (see Task #3). Modules map 1:1 to the design doc §6 module
//! breakdown:
//!
//! - [`config`]: daemon TOML configuration (§7.5)
//! - [`policy`]: workload class + policy file schema (§7.2)
//! - [`backend`]: backend kinds + selection / fallback (§6.2.3)
//! - [`lifecycle`]: sandbox state machine + JSON persistence (§6.2.5)
//! - [`pool`]: warm-pool key/stat/manager (§6.2.6)
//! - [`template`]: template registry + refcnt + GC (§6.2.7)
//! - [`trajectory`]: JSONL trajectory recorder (§6.2.8)
//! - [`kernel`]: kernel hook registry, per-hook mutex (§6.2.4)
//! - [`error`]: unified [`AnvilError`] error enum

pub mod backend;
pub mod config;
pub mod error;
pub mod kernel;
pub mod lifecycle;
pub mod policy;
pub mod pool;
pub mod template;
pub mod trajectory;

pub use error::{AnvilError, Result};
