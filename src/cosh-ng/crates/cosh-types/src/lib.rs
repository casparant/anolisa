#![forbid(unsafe_code)]
//! Shared, side-effect-free COSH compatibility and audit types.
//!
//! The checkpoint module preserves the historical ws-ckpt binary wire contract;
//! COSH no longer exposes a checkpoint command.

pub mod audit;
pub mod checkpoint;
pub mod error;
