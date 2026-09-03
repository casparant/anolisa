#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Side-effect-free contracts shared by AW Core, Agent Environments, and Providers.
//!
//! Concrete transports, persistence engines, Agent protocols, and Provider
//! implementations depend on these types without becoming part of the public
//! system contract.

pub mod canonical;
pub mod common;
pub mod context;
pub mod error;
pub mod ids;
pub mod ledger;
pub mod provider;
pub mod security;
