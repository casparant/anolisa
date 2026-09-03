//! Provider-independent contracts for security inspection and Tool Call gates.
//!
//! These Capabilities report facts about content that already exists, or return
//! a verdict for a Tool Call that has not run yet. None of them carries matched
//! content: every textual field is a closed enum or a [`SecurityRuleId`], so a
//! finding cannot become a channel for the secret it found.

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::{
    common::{BoundedName, BoundedStringError, Digest, DigestError},
    ids::ArtifactId,
    provider::{SchemaReference, VersionedSchema},
};

/// Stable identity of the content-inspection Capability.
pub const SECURITY_CONTENT_INSPECT_CAPABILITY_ID: &str = "security.content.inspect";
/// Current revision of the content-inspection Capability.
pub const SECURITY_CONTENT_INSPECT_CAPABILITY_VERSION: u16 = 1;
/// Stable identity of the canonical content-inspection input schema.
pub const SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_ID: &str = "security.content.inspect.input";
/// Current revision of the canonical content-inspection input schema.
pub const SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical content-inspection input schema resource.
pub const SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_SHA256: &str =
    "4d1c7d2b3c58d29af35c6dce10d36a2774d12ec7d2d7928e262cf978c2babeb3";
/// Stable identity of the canonical content-inspection output schema.
pub const SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_ID: &str = "security.content.inspect.output";
/// Current revision of the canonical content-inspection output schema.
pub const SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical content-inspection output schema resource.
pub const SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_SHA256: &str =
    "15d47c6c1c9b43928d9613485bf8dee398622e2b1b7f0ab96a5ad7d5342d55ef";

/// Stable identity of the code-inspection Capability.
pub const SECURITY_CODE_INSPECT_CAPABILITY_ID: &str = "security.code.inspect";
/// Current revision of the code-inspection Capability.
pub const SECURITY_CODE_INSPECT_CAPABILITY_VERSION: u16 = 1;
/// Stable identity of the canonical code-inspection input schema.
pub const SECURITY_CODE_INSPECT_INPUT_SCHEMA_ID: &str = "security.code.inspect.input";
/// Current revision of the canonical code-inspection input schema.
pub const SECURITY_CODE_INSPECT_INPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical code-inspection input schema resource.
pub const SECURITY_CODE_INSPECT_INPUT_SCHEMA_SHA256: &str =
    "856b0626c2f2523cc78db468daae2dae5df685707950e6dd4f5123d4e616236f";
/// Stable identity of the canonical code-inspection output schema.
pub const SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_ID: &str = "security.code.inspect.output";
/// Current revision of the canonical code-inspection output schema.
pub const SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical code-inspection output schema resource.
pub const SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_SHA256: &str =
    "735eae6cb55642b2f956214edccf7a09d0931ab93c6be1bcaaaaac0ceed276b0";

/// Stable identity of the command-inspection Capability.
pub const SECURITY_COMMAND_INSPECT_CAPABILITY_ID: &str = "security.command.inspect";
/// Current revision of the command-inspection Capability.
pub const SECURITY_COMMAND_INSPECT_CAPABILITY_VERSION: u16 = 1;
/// Stable identity of the canonical command-inspection input schema.
pub const SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_ID: &str = "security.command.inspect.input";
/// Current revision of the canonical command-inspection input schema.
pub const SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical command-inspection input schema resource.
pub const SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_SHA256: &str =
    "77777d6a3168724747c8070735690d391c7492ad0255f4d4520188763b298219";
/// Stable identity of the canonical command-inspection output schema.
pub const SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_ID: &str = "security.command.inspect.output";
/// Current revision of the canonical command-inspection output schema.
pub const SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_VERSION: u16 = 1;
/// SHA-256 of the current canonical command-inspection output schema resource.
pub const SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_SHA256: &str =
    "2b6564e583294b3777ae1ec48baa6744a4f169ae010757bb9568674dfabc3d11";

/// Maximum UTF-8 byte length of a security rule identity.
pub const MAX_SECURITY_RULE_ID_BYTES: usize = 64;
/// Maximum number of findings Core accepts from one inspection.
pub const MAX_OBSERVATION_FINDINGS: usize = 64;
/// Maximum number of rationale codes Core accepts from one gate verdict.
pub const MAX_GATE_REASONS: usize = 32;

/// Failure returned when a security rule identity is not a stable label.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecurityRuleIdError {
    /// A rule identity must name a concrete rule.
    #[error("security rule id must not be empty")]
    Empty,
    /// Rule identities are capped to keep Ledger records predictable.
    #[error("security rule id exceeds the {MAX_SECURITY_RULE_ID_BYTES}-byte limit")]
    TooLong,
    /// The character set is deliberately narrow; see [`SecurityRuleId`].
    #[error("security rule id must use lowercase ASCII letters, digits, '.', '_', and '-'")]
    InvalidCharacter,
}

/// Stable identity of one security rule that produced a finding.
///
/// The accepted character set is narrower than [`BoundedName`] on purpose. A
/// rule label is the only free-form field an inspection result carries, so
/// restricting it to `[a-z0-9._-]` keeps a Provider from smuggling matched
/// content — an API key, a password, a personal identifier — out through it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecurityRuleId(String);

impl SecurityRuleId {
    /// Parses a stable lowercase rule identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is empty, oversized, or contains any
    /// character outside `[a-z0-9._-]`.
    pub fn parse(value: impl Into<String>) -> Result<Self, SecurityRuleIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SecurityRuleIdError::Empty);
        }
        if value.len() > MAX_SECURITY_RULE_ID_BYTES {
            return Err(SecurityRuleIdError::TooLong);
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        }) {
            return Err(SecurityRuleIdError::InvalidCharacter);
        }
        Ok(Self(value))
    }

    /// Returns the stable rule identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for SecurityRuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecurityRuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Environment boundary at which an inspection Capability is invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityBoundary {
    /// Before a Tool Call executes, while a gate can still change the outcome.
    PreTool,
    /// After a Tool Call produced a result.
    PostTool,
}

/// Source language a code inspection should assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCodeLanguage {
    /// Let the implementation choose from the content.
    Auto,
    /// POSIX or Bash shell.
    Bash,
    /// Python.
    Python,
}

/// Language a code inspection reported having analysed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityDetectedLanguage {
    /// POSIX or Bash shell.
    Bash,
    /// Python.
    Python,
    /// The implementation could not classify the content.
    Unknown,
}

/// Broad class of a security finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingCategory {
    /// Long-lived secret material such as an API key or private key.
    Secret,
    /// Personal data such as an identity number or contact detail.
    PersonalData,
    /// Interactive credential such as a password or token.
    Credential,
    /// Construct whose execution is intrinsically risky.
    DangerousPattern,
    /// Construct that appears intended to hide its behaviour.
    Obfuscation,
    /// A class the Capability does not model more precisely.
    Other,
}

/// Severity an implementation attached to a finding class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingSeverity {
    /// Recorded for completeness; no action implied.
    Info,
    /// Minor concern.
    Low,
    /// Concern that deserves review.
    Medium,
    /// Serious concern.
    High,
    /// Severe concern.
    Critical,
}

/// How confident an implementation is that a finding is real.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingConfidence {
    /// Likely to include false positives.
    Low,
    /// Balanced precision and recall.
    Medium,
    /// Precise match.
    High,
}

/// Overall conclusion of one inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityInspectionVerdict {
    /// Nothing was found.
    Clean,
    /// Something was found that warrants attention but is not conclusive.
    Suspicious,
    /// Content that must be treated as sensitive was found.
    Sensitive,
}

/// Gate verdict returned for a pending Tool Call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandVerdict {
    /// The Capability found no reason to stop the Tool Call.
    Allow,
    /// The Capability found a concern that does not justify refusing.
    Warn,
    /// The Capability judges the Tool Call unsafe to run.
    Deny,
}

/// Content-free count of one finding class.
///
/// A finding never carries the matched value, its offset, or its surrounding
/// text. It reports which rule fired, how it is classified, and how often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityFinding {
    /// Rule that produced the matches.
    pub rule_id: SecurityRuleId,
    /// Broad class of the finding.
    pub category: SecurityFindingCategory,
    /// Severity the implementation attached.
    pub severity: SecurityFindingSeverity,
    /// Confidence the implementation attached.
    pub confidence: SecurityFindingConfidence,
    /// Number of matches attributed to this rule.
    pub count: u32,
}

/// Content-free result of `security.content.inspect/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentInspection {
    /// Overall conclusion.
    pub verdict: SecurityInspectionVerdict,
    /// Per-rule counts.
    pub findings: Vec<SecurityFinding>,
    /// Bytes the implementation reported inspecting.
    pub scanned_bytes: u64,
    /// Whether the implementation stopped before the whole artifact.
    pub truncated: bool,
}

/// Content-free result of `security.code.inspect/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeInspection {
    /// Overall conclusion.
    pub verdict: SecurityInspectionVerdict,
    /// Per-rule counts.
    pub findings: Vec<SecurityFinding>,
    /// Bytes the implementation reported inspecting.
    pub scanned_bytes: u64,
    /// Whether the implementation stopped before the whole artifact.
    pub truncated: bool,
    /// Language the implementation reported analysing, when it classified one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_detected: Option<SecurityDetectedLanguage>,
}

/// Gate verdict and rationale returned by `security.command.inspect/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandInspection {
    /// Verdict Core turns into a Tool Call gate.
    pub verdict: CommandVerdict,
    /// Rationale codes safe for operator presentation.
    pub reasons: Vec<SecurityRuleId>,
    /// Per-rule counts.
    pub findings: Vec<SecurityFinding>,
    /// Bytes the implementation reported inspecting.
    pub scanned_bytes: u64,
}

/// Pending Tool Call an Agent Environment offers to a Mediate Capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingToolCallSubmission {
    /// Command text the Agent proposes to execute.
    pub command: String,
    /// Language the Environment believes the command is written in.
    pub language: SecurityCodeLanguage,
    /// Tool name when the Environment can provide one safely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<BoundedName>,
}

/// Why an Observe Capability produced no usable fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationGapReason {
    /// No admitted implementation satisfies the Capability Contract.
    NoImplementation,
    /// A matching implementation only declares its isolation controls.
    ControlsNotEnforced,
    /// The invocation settled without producing a result.
    NotProduced,
    /// The implementation returned a result Core could not accept.
    InvalidOutput,
    /// The Provider Host could not complete the invocation.
    HostFailure,
    /// The fact could not be recorded durably, so it is not claimed.
    LedgerUnavailable,
}

/// Why a Tool Call gate resolved without an implementation verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDegradation {
    /// No admitted implementation satisfies the Capability Contract.
    NoImplementation,
    /// Several implementations qualify and routing policy named none.
    AmbiguousRoute,
    /// A matching implementation only declares its isolation controls.
    ControlsNotEnforced,
    /// The invocation settled without producing a verdict.
    NotProduced,
    /// The implementation returned a verdict Core could not accept.
    InvalidOutput,
    /// The Provider Host could not complete the invocation.
    HostFailure,
    /// The decision could not be recorded durably.
    LedgerUnavailable,
}

/// Gate outcome Core requires an Agent Environment to honour.
///
/// [`ToolCallGate::NotMediated`] is not an approval. It states that no verdict
/// exists, so the Environment must apply its own default rather than read the
/// absence of an opinion as permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallGate {
    /// No governed verdict was produced for this Tool Call.
    NotMediated,
    /// The Tool Call may proceed.
    Allow,
    /// The Tool Call may proceed and the operator should be told why not to.
    Warn,
    /// A human must decide before the Tool Call proceeds.
    Ask,
    /// The Tool Call must not proceed.
    Block,
}

/// Failure returned while constructing a built-in security Contract reference.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecurityContractBuildError {
    /// A built-in schema name violates the bounded-name invariant.
    #[error(transparent)]
    Name(#[from] BoundedStringError),
    /// A built-in schema digest is not canonical SHA-256 text.
    #[error(transparent)]
    Digest(#[from] DigestError),
}

/// Immutable content offered to an inspection Capability.
///
/// This mirrors the context-projection artifact so one Environment event can
/// submit the same bytes to several Capabilities under one identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionArtifact {
    /// Core identity of the immutable source artifact.
    pub id: ArtifactId,
    /// SHA-256 of the artifact content.
    pub digest: Digest,
    /// Media type of the artifact content.
    pub media_type: BoundedName,
}

/// Returns the current content-inspection Capability identity.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant violates its bounded
/// representation. Such a failure indicates a build-time defect.
pub fn security_content_inspect_capability() -> Result<VersionedSchema, SecurityContractBuildError>
{
    versioned_schema(
        SECURITY_CONTENT_INSPECT_CAPABILITY_ID,
        SECURITY_CONTENT_INSPECT_CAPABILITY_VERSION,
    )
}

/// Returns the exact current canonical content-inspection input Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_content_inspect_input_contract(
) -> Result<SchemaReference, SecurityContractBuildError> {
    schema_reference(
        SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_ID,
        SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_VERSION,
        SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_SHA256,
    )
}

/// Returns the exact current canonical content-inspection output Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_content_inspect_output_contract(
) -> Result<SchemaReference, SecurityContractBuildError> {
    schema_reference(
        SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_ID,
        SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_VERSION,
        SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_SHA256,
    )
}

/// Returns the current code-inspection Capability identity.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_code_inspect_capability() -> Result<VersionedSchema, SecurityContractBuildError> {
    versioned_schema(
        SECURITY_CODE_INSPECT_CAPABILITY_ID,
        SECURITY_CODE_INSPECT_CAPABILITY_VERSION,
    )
}

/// Returns the exact current canonical code-inspection input Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_code_inspect_input_contract() -> Result<SchemaReference, SecurityContractBuildError>
{
    schema_reference(
        SECURITY_CODE_INSPECT_INPUT_SCHEMA_ID,
        SECURITY_CODE_INSPECT_INPUT_SCHEMA_VERSION,
        SECURITY_CODE_INSPECT_INPUT_SCHEMA_SHA256,
    )
}

/// Returns the exact current canonical code-inspection output Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_code_inspect_output_contract() -> Result<SchemaReference, SecurityContractBuildError>
{
    schema_reference(
        SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_ID,
        SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_VERSION,
        SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_SHA256,
    )
}

/// Returns the current command-inspection Capability identity.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_command_inspect_capability() -> Result<VersionedSchema, SecurityContractBuildError>
{
    versioned_schema(
        SECURITY_COMMAND_INSPECT_CAPABILITY_ID,
        SECURITY_COMMAND_INSPECT_CAPABILITY_VERSION,
    )
}

/// Returns the exact current canonical command-inspection input Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_command_inspect_input_contract(
) -> Result<SchemaReference, SecurityContractBuildError> {
    schema_reference(
        SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_ID,
        SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_VERSION,
        SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_SHA256,
    )
}

/// Returns the exact current canonical command-inspection output Contract.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid.
pub fn security_command_inspect_output_contract(
) -> Result<SchemaReference, SecurityContractBuildError> {
    schema_reference(
        SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_ID,
        SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_VERSION,
        SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_SHA256,
    )
}

fn schema_reference(
    id: &str,
    version: u16,
    digest: &str,
) -> Result<SchemaReference, SecurityContractBuildError> {
    Ok(SchemaReference {
        schema: versioned_schema(id, version)?,
        digest: Digest::parse(digest)?,
    })
}

fn versioned_schema(id: &str, version: u16) -> Result<VersionedSchema, SecurityContractBuildError> {
    Ok(VersionedSchema {
        id: BoundedName::new(id)?,
        version,
    })
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    use super::*;

    #[test]
    fn built_in_security_contracts_are_canonical() {
        let content = security_content_inspect_capability().expect("Capability is canonical");
        let code = security_code_inspect_capability().expect("Capability is canonical");
        let command = security_command_inspect_capability().expect("Capability is canonical");

        assert_eq!(content.id.as_str(), SECURITY_CONTENT_INSPECT_CAPABILITY_ID);
        assert_eq!(code.id.as_str(), SECURITY_CODE_INSPECT_CAPABILITY_ID);
        assert_eq!(command.id.as_str(), SECURITY_COMMAND_INSPECT_CAPABILITY_ID);

        for contract in [
            security_content_inspect_input_contract(),
            security_content_inspect_output_contract(),
            security_code_inspect_input_contract(),
            security_code_inspect_output_contract(),
            security_command_inspect_input_contract(),
            security_command_inspect_output_contract(),
        ] {
            contract.expect("compiled-in Contract is canonical");
        }
    }

    #[test]
    fn canonical_security_schema_resources_match_their_contract_digests() {
        for (bytes, expected) in [
            (
                &include_bytes!("../schemas/security-content-inspect-input-v1.schema.json")[..],
                SECURITY_CONTENT_INSPECT_INPUT_SCHEMA_SHA256,
            ),
            (
                &include_bytes!("../schemas/security-content-inspect-output-v1.schema.json")[..],
                SECURITY_CONTENT_INSPECT_OUTPUT_SCHEMA_SHA256,
            ),
            (
                &include_bytes!("../schemas/security-code-inspect-input-v1.schema.json")[..],
                SECURITY_CODE_INSPECT_INPUT_SCHEMA_SHA256,
            ),
            (
                &include_bytes!("../schemas/security-code-inspect-output-v1.schema.json")[..],
                SECURITY_CODE_INSPECT_OUTPUT_SCHEMA_SHA256,
            ),
            (
                &include_bytes!("../schemas/security-command-inspect-input-v1.schema.json")[..],
                SECURITY_COMMAND_INSPECT_INPUT_SCHEMA_SHA256,
            ),
            (
                &include_bytes!("../schemas/security-command-inspect-output-v1.schema.json")[..],
                SECURITY_COMMAND_INSPECT_OUTPUT_SCHEMA_SHA256,
            ),
        ] {
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected);
        }
    }

    #[test]
    fn rule_ids_reject_characters_that_could_carry_matched_content() {
        for rejected in [
            "AKIA1234",
            "aws key",
            "rule/slash",
            "rule:colon",
            "rule=value",
            "パス",
        ] {
            assert_eq!(
                SecurityRuleId::parse(rejected),
                Err(SecurityRuleIdError::InvalidCharacter),
                "{rejected} must be rejected"
            );
        }
        assert_eq!(SecurityRuleId::parse(""), Err(SecurityRuleIdError::Empty));
        assert_eq!(
            SecurityRuleId::parse("a".repeat(MAX_SECURITY_RULE_ID_BYTES + 1)),
            Err(SecurityRuleIdError::TooLong)
        );

        for accepted in ["pii.aws_access_key", "shell-rm-rf", "rule.v2", "a"] {
            SecurityRuleId::parse(accepted).expect("stable label is accepted");
        }
    }

    #[test]
    fn inspection_results_reject_unknown_fields() {
        let smuggled = serde_json::json!({
            "verdict": "sensitive",
            "findings": [{
                "rule_id": "pii.aws_access_key",
                "category": "secret",
                "severity": "critical",
                "confidence": "high",
                "count": 1,
                "match": "AKIAIOSFODNN7EXAMPLE"
            }],
            "scanned_bytes": 42,
            "truncated": false
        });

        assert!(serde_json::from_value::<ContentInspection>(smuggled).is_err());
    }
}
