#![forbid(unsafe_code)]
#![allow(
    clippy::result_large_err,
    reason = "crate APIs share public CoshError; per-function allows obscure one API trade-off"
)]
//! Runtime support shared by COSH processes.
//!
//! The crate owns audit persistence and process-group primitives. Retired
//! package, service, and checkpoint command implementations do not live here.

pub mod audit;
pub mod process;
