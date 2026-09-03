//! Content-freedom invariant tests.
//!
//! Each forbidden key must be rejected at the root, inside nested objects,
//! and inside arrays. Case-insensitive matching must catch mixed-case
//! variants. Legitimate digest-only bodies must pass.

use serde_json::json;

use crate::{admit, AdmissionError, Chain};

use super::{candidate, clean_body};

#[test]
fn clean_body_is_admitted() {
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, clean_body());
    admit(&tip, candidate).expect("clean body admitted");
}

#[test]
fn each_forbidden_key_is_rejected_at_root() {
    for key in [
        "command",
        "tool_input",
        "tool_response",
        "matched",
        "content",
        "payload",
    ] {
        let body = json!({ key: "raw tool text" });
        let chain = Chain::new();
        let tip = chain.tip();
        let candidate = candidate(&tip, body);
        let error = admit(&tip, candidate).unwrap_err();
        match error {
            AdmissionError::ContentForbidden {
                path,
                key: rejected,
            } => {
                assert_eq!(path, "/", "root path must be /: key={key}");
                assert_eq!(rejected, key);
            }
            other => panic!("expected ContentForbidden for `{key}`, got {other:?}"),
        }
    }
}

#[test]
fn forbidden_key_nested_in_object_is_rejected() {
    let body = json!({
        "evidence": {
            "findings": [{ "matched": "rm -rf /" }]
        }
    });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    let error = admit(&tip, candidate).unwrap_err();
    match error {
        AdmissionError::ContentForbidden { key, .. } => assert_eq!(key, "matched"),
        other => panic!("expected ContentForbidden, got {other:?}"),
    }
}

#[test]
fn forbidden_key_inside_array_element_is_rejected() {
    let body = json!({
        "items": [
            { "digest": "abc" },
            { "tool_input": "ls -la" }
        ]
    });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    let error = admit(&tip, candidate).unwrap_err();
    match error {
        AdmissionError::ContentForbidden { key, .. } => assert_eq!(key, "tool_input"),
        other => panic!("expected ContentForbidden, got {other:?}"),
    }
}

#[test]
fn case_insensitive_match_rejects_uppercase() {
    let body = json!({ "Command": "echo hello" });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    let error = admit(&tip, candidate).unwrap_err();
    match error {
        AdmissionError::ContentForbidden { key, .. } => assert_eq!(key, "Command"),
        other => panic!("expected ContentForbidden, got {other:?}"),
    }
}

#[test]
fn case_insensitive_match_rejects_mixed_case() {
    let body = json!({ "TOOL_INPUT": "data" });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    let error = admit(&tip, candidate).unwrap_err();
    match error {
        AdmissionError::ContentForbidden { key, .. } => assert_eq!(key, "TOOL_INPUT"),
        other => panic!("expected ContentForbidden, got {other:?}"),
    }
}

#[test]
fn deeply_nested_forbidden_key_is_rejected() {
    let body = json!({
        "level1": {
            "level2": {
                "level3": {
                    "content": "raw artifact text"
                }
            }
        }
    });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    let error = admit(&tip, candidate).unwrap_err();
    match error {
        AdmissionError::ContentForbidden { key, path } => {
            assert_eq!(key, "content");
            assert!(
                path.contains("level3"),
                "path should name the containing object: {path}"
            );
        }
        other => panic!("expected ContentForbidden, got {other:?}"),
    }
}

#[test]
fn digest_only_body_is_admitted() {
    let body = json!({
        "projection": {
            "id": "prj_00000000-0000-0000-0000-000000000000",
            "digest": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        },
        "scope": {
            "attempt_id": "atm_00000000-0000-0000-0000-000000000000"
        }
    });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    admit(&tip, candidate).expect("digest-only body admitted");
}

#[test]
fn empty_object_body_is_admitted() {
    let body = json!({});
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    admit(&tip, candidate).expect("empty object admitted");
}

#[test]
fn similar_but_allowed_keys_pass() {
    // Keys that share substrings with forbidden keys but are not forbidden.
    let body = json!({
        "body_digest": "abc",
        "input_ref": "pvi_00000000-0000-0000-0000-000000000000",
        "response_code": 200,
        "match_count": 3,
        "content_ref": "art_00000000-0000-0000-0000-000000000000"
    });
    let chain = Chain::new();
    let tip = chain.tip();
    let candidate = candidate(&tip, body);
    admit(&tip, candidate).expect("similar-but-allowed keys admitted");
}
