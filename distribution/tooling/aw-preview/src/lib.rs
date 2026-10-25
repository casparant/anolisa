//! Distribution-owned builder for the historical AW Preview delivery set.

pub mod package;
pub use local_package::{digest, inspect, Component, Error, Manifest, Payload, Result};

fn require(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
