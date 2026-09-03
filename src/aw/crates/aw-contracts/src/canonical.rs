#![forbid(unsafe_code)]
//! Canonical JSON encoding shared by every AW boundary that digests payloads.
//!
//! The encoding is a contract, not an implementation detail of one driver:
//! equivalent `serde_json::Value` trees — regardless of object insertion order
//! — produce byte-identical compact UTF-8 JSON, so two parties computing a
//! SHA-256 over these bytes agree on one digest.

use serde_json::{Map, Value};

/// Encodes one value as Agent Workload canonical JSON v1.
///
/// Objects are recursively sorted by key, arrays retain order, and the result
/// is compact UTF-8 JSON. Provider payload digests, `exec-json/v1` stdin, and
/// Ledger record digests use these exact bytes so equivalent object insertion
/// orders produce one digest.
///
/// # Errors
///
/// Returns an error if `serde_json` cannot encode the normalized value.
pub fn canonical_json_v1_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonical_json_v1_value(value))
}

fn canonical_json_v1_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonical_json_v1_value(value));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_v1_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_insertion_orders_yield_identical_bytes() {
        let left: Value =
            serde_json::from_str(r#"{"z":1,"nested":{"b":2,"a":1},"items":[{"d":4,"c":3}]}"#)
                .unwrap();
        let right: Value =
            serde_json::from_str(r#"{"items":[{"c":3,"d":4}],"nested":{"a":1,"b":2},"z":1}"#)
                .unwrap();
        assert_eq!(
            canonical_json_v1_bytes(&left).unwrap(),
            canonical_json_v1_bytes(&right).unwrap()
        );
    }

    #[test]
    fn arrays_retain_insertion_order() {
        let value: Value = serde_json::from_str(r#"{"xs":[3,1,2]}"#).unwrap();
        let encoded = String::from_utf8(canonical_json_v1_bytes(&value).unwrap()).unwrap();
        assert_eq!(encoded, r#"{"xs":[3,1,2]}"#);
    }

    #[test]
    fn nested_object_keys_are_sorted_at_every_depth() {
        let value: Value = serde_json::from_str(
            r#"{"outer":{"z":9,"inner":{"beta":2,"alpha":1}},"alpha":"first"}"#,
        )
        .unwrap();
        let encoded = String::from_utf8(canonical_json_v1_bytes(&value).unwrap()).unwrap();
        assert_eq!(
            encoded,
            r#"{"alpha":"first","outer":{"inner":{"alpha":1,"beta":2},"z":9}}"#
        );
    }
}
