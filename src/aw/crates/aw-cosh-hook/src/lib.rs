#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! COSH PostToolUse adapter for the AW Core context pipeline.
//!
//! The adapter consumes COSH's hook envelope, submits only the model-visible
//! tool result to Core, and emits a replacement only after an admitted
//! Provider returns a valid context-projection candidate.

use std::io::{self, Read, Write};

use aw_contracts::common::{BoundedName, BoundedOpaque, BoundedStringError, TargetRef};
use aw_contracts::context::{ContextArtifactOrigin, ContextReversibility, ToolResultSubmission};
use aw_contracts::ids::{
    ActorId, AgentSessionId, EnvironmentId, ExecutionContextId, ToolUseId, TurnId,
};
use aw_contracts::provider::{
    ProviderDisposition, ProviderMeasurementKind, ProviderMeter, ProviderReceipt,
};
use aw_core::{
    Core, CoreConfig, CoreError, PrepareToolResultOptions, PreparedToolResult, SessionContextSpec,
};
use aw_provider_host::{
    ProviderAdmissionOptions, ProviderCatalog, ProviderHostError, ProviderManifestSource,
    MAX_PROVIDER_INVOCATION_BYTES,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const POST_TOOL_USE: &str = "PostToolUse";

/// Explicit operator inputs used by the COSH adapter.
#[derive(Debug, Clone)]
pub struct CoshHookConfig {
    /// Manifest file or package directory admitted for this hook call.
    pub provider_source: ProviderManifestSource,
    /// Executable roots used by Provider admission.
    pub provider_admission: ProviderAdmissionOptions,
    /// Host or remote target asserted for this local adapter invocation.
    pub target: TargetRef,
    /// Provider selected by policy when several implementations qualify.
    pub preferred_provider_id: Option<BoundedName>,
    /// Explicitly trust a Provider before OS controls enforce its declarations.
    pub allow_unenforced_provider: bool,
}

/// Content-free summary of one COSH hook invocation.
#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoshHookRun {
    /// Whether the adapter asked COSH to replace the model-visible result.
    ///
    /// COSH may still apply later Hook aggregation or redaction, so this is
    /// not proof of the final bytes delivered to a model.
    pub replacement_requested: bool,
    /// Provider facts when Core reached an accepted invocation.
    pub receipt: Option<ProviderReceipt>,
}

/// Failure returned before the adapter can emit a trustworthy hook response.
#[derive(Debug, Error)]
pub enum CoshHookError {
    /// Hook input could not be read or output could not be written.
    #[error("hook I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Hook input exceeded the shared AW invocation boundary.
    #[error("hook input exceeds the {MAX_PROVIDER_INVOCATION_BYTES}-byte limit")]
    InputTooLarge,
    /// COSH supplied malformed or structurally incomplete JSON.
    #[error("invalid COSH hook input: {0}")]
    InvalidInput(#[source] serde_json::Error),
    /// A typed COSH response could not be encoded as JSON.
    #[error("COSH hook output could not be encoded: {0}")]
    InvalidOutput(#[source] serde_json::Error),
    /// The adapter was called at a boundary it cannot transform.
    #[error("expected COSH {POST_TOOL_USE} input, received `{0}`")]
    WrongHookEvent(String),
    /// A pre-correlation COSH runtime cannot provide an enforceable Tool scope.
    #[error("COSH hook input does not contain `execution_scope`")]
    MissingExecutionScopeCorrelation,
    /// COSH supplied no model-visible content to prepare.
    #[error("COSH hook input contains no model-visible tool response")]
    MissingToolResponse,
    /// A target, tool name, or Provider preference violated a bounded Contract.
    #[error(transparent)]
    BoundedValue(#[from] BoundedStringError),
    /// Provider discovery or admission failed.
    #[error(transparent)]
    ProviderHost(#[from] ProviderHostError),
    /// Core could not route or prepare the tool result.
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Processes one COSH PostToolUse envelope and writes one COSH hook response.
///
/// The response contains `updatedToolResponse` only when a reversible
/// projection was produced. A bypass or settled Provider failure keeps the
/// original tool response and never copies its content into the receipt.
/// Correlation fields in the hook input are not authorization credentials.
///
/// # Errors
///
/// Returns an error for malformed hook input, missing AW correlation,
/// Provider admission failure, Core routing failure, or output I/O failure.
pub fn run_cosh_post_tool_use(
    mut reader: impl Read,
    mut writer: impl Write,
    config: &CoshHookConfig,
) -> Result<CoshHookRun, CoshHookError> {
    let mut input = Vec::new();
    reader
        .by_ref()
        .take(MAX_PROVIDER_INVOCATION_BYTES as u64 + 1)
        .read_to_end(&mut input)?;
    if input.len() > MAX_PROVIDER_INVOCATION_BYTES {
        return Err(CoshHookError::InputTooLarge);
    }

    let input: CoshPostToolUseInput =
        serde_json::from_slice(&input).map_err(CoshHookError::InvalidInput)?;
    if input.hook_event_name != POST_TOOL_USE {
        return Err(CoshHookError::WrongHookEvent(input.hook_event_name));
    }
    let scope = input
        .execution_scope
        .ok_or(CoshHookError::MissingExecutionScopeCorrelation)?;
    if input.tool_response_is_error {
        write_response(&mut writer, &CoshHookOutput::default())?;
        return Ok(CoshHookRun {
            replacement_requested: false,
            receipt: None,
        });
    }
    let content =
        model_visible_content(&input.tool_response).ok_or(CoshHookError::MissingToolResponse)?;
    if content.is_empty() {
        write_response(&mut writer, &CoshHookOutput::default())?;
        return Ok(CoshHookRun {
            replacement_requested: false,
            receipt: None,
        });
    }

    let catalog =
        ProviderCatalog::discover(config.provider_source.clone(), &config.provider_admission)?;
    let core = Core::with_config(
        catalog,
        CoreConfig {
            allow_unenforced_providers: config.allow_unenforced_provider,
            ..CoreConfig::default()
        },
    )?;
    let context = core.establish_execution_context(SessionContextSpec {
        target: config.target.clone(),
        environment_id: scope.environment_id,
        actor_id: scope.actor_id,
        agent_session_id: Some(scope.agent_session_id),
        work_id: None,
        attempt_id: None,
        execution_context_id: Some(scope.execution_context_id),
    })?;
    let submission = ToolResultSubmission {
        media_type: BoundedName::new(media_type(&content))?,
        origin: origin_for_tool(&input.tool_name),
        tool_name: Some(BoundedName::new(input.tool_name)?),
        content,
        allow_text_reencoding: true,
    };
    let prepared = core.prepare_tool_result(
        &context,
        scope.turn_id,
        scope.tool_use_id,
        submission,
        PrepareToolResultOptions {
            preferred_provider_id: config.preferred_provider_id.clone(),
        },
    )?;
    let output = hook_output(&prepared);
    let replacement_requested = output
        .hook_specific_output
        .as_ref()
        .and_then(|specific| specific.updated_tool_response.as_ref())
        .is_some();
    write_response(&mut writer, &output)?;

    Ok(CoshHookRun {
        replacement_requested,
        receipt: Some(prepared.receipt),
    })
}

/// Builds the common local-host target used by the standalone adapter.
///
/// # Errors
///
/// Returns an error when `identifier` violates the bounded target Contract.
pub fn local_host_target(identifier: impl Into<String>) -> Result<TargetRef, BoundedStringError> {
    Ok(TargetRef {
        kind: BoundedName::new("host")?,
        authority: BoundedName::new("local")?,
        identifier: BoundedOpaque::new(identifier)?,
    })
}

#[derive(Debug, Deserialize)]
struct CoshPostToolUseInput {
    hook_event_name: String,
    tool_name: String,
    tool_response: Value,
    #[serde(default)]
    tool_response_is_error: bool,
    execution_scope: Option<CoshExecutionScope>,
}

#[derive(Debug, Deserialize)]
struct CoshExecutionScope {
    environment_id: EnvironmentId,
    execution_context_id: ExecutionContextId,
    actor_id: ActorId,
    agent_session_id: AgentSessionId,
    turn_id: TurnId,
    tool_use_id: ToolUseId,
}

#[derive(Debug, Default, Serialize)]
struct CoshHookOutput {
    #[serde(rename = "suppressOutput", skip_serializing_if = "Option::is_none")]
    suppress_output: Option<bool>,
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<CoshHookSpecificOutput>,
}

#[derive(Debug, Serialize)]
struct CoshHookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "updatedToolResponse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_tool_response: Option<String>,
}

fn model_visible_content(response: &Value) -> Option<String> {
    match response {
        Value::String(content) => Some(content.clone()),
        Value::Object(object) => object
            .get("llmContent")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Value::Array(_) | Value::Bool(_) | Value::Number(_) => serde_json::to_string(response).ok(),
        Value::Null => None,
    }
}

fn media_type(content: &str) -> &'static str {
    if serde_json::from_str::<Value>(content).is_ok() {
        "application/json"
    } else {
        "text/plain"
    }
}

fn origin_for_tool(tool_name: &str) -> ContextArtifactOrigin {
    match tool_name {
        "shell" | "run_shell_command" | "Bash" | "terminal" | "exec" | "process" => {
            ContextArtifactOrigin::CommandOutput
        }
        "read_file" | "Read" | "grep" | "grep_search" | "list_directory" => {
            ContextArtifactOrigin::FileContent
        }
        _ => ContextArtifactOrigin::ApiResponse,
    }
}

fn hook_output(prepared: &PreparedToolResult) -> CoshHookOutput {
    let Some(candidate) = prepared.candidate.as_ref().filter(|candidate| {
        candidate.reversibility == ContextReversibility::Lossless && !candidate.content.is_empty()
    }) else {
        let system_message = matches!(
            prepared.receipt.disposition,
            ProviderDisposition::Denied
                | ProviderDisposition::Failed
                | ProviderDisposition::Uncertain
        )
        .then(|| {
            format!(
                "AW · {} · original tool result kept ({})",
                prepared.receipt.provider_id.as_str(),
                disposition_label(prepared.receipt.disposition)
            )
        });
        return CoshHookOutput {
            suppress_output: system_message.as_ref().map(|_| true),
            system_message,
            hook_specific_output: None,
        };
    };

    CoshHookOutput {
        suppress_output: Some(true),
        system_message: Some(savings_message(&prepared.receipt)),
        hook_specific_output: Some(CoshHookSpecificOutput {
            hook_event_name: POST_TOOL_USE,
            updated_tool_response: Some(candidate.content.clone()),
        }),
    }
}

fn disposition_label(disposition: ProviderDisposition) -> &'static str {
    match disposition {
        ProviderDisposition::Produced => "produced",
        ProviderDisposition::EffectApplied => "effect_applied",
        ProviderDisposition::Bypassed => "bypassed",
        ProviderDisposition::Denied => "denied",
        ProviderDisposition::Failed => "failed",
        ProviderDisposition::Uncertain => "uncertain",
    }
}

fn savings_message(receipt: &ProviderReceipt) -> String {
    let source = meter(receipt, "context.source_tokens");
    let prepared = meter(receipt, "context.prepared_tokens");
    match (source, prepared) {
        (Some(source), Some(prepared)) if source.value > 0 && prepared.value <= source.value => {
            let saved_percent =
                ((source.value - prepared.value) as f64 / source.value as f64) * 100.0;
            let qualifier = if source.measurement_kind == ProviderMeasurementKind::Estimate
                || prepared.measurement_kind == ProviderMeasurementKind::Estimate
            {
                "estimated context "
            } else {
                "context "
            };
            format!(
                "AW · {} · {}{}→{} tokens · saved {:.0}%",
                receipt.provider_id.as_str(),
                qualifier,
                source.value,
                prepared.value,
                saved_percent
            )
        }
        _ => format!(
            "AW · {} · context projection applied",
            receipt.provider_id.as_str()
        ),
    }
}

fn meter<'a>(receipt: &'a ProviderReceipt, meter_id: &str) -> Option<&'a ProviderMeter> {
    receipt
        .meters
        .iter()
        .find(|meter| meter.meter_id.as_str() == meter_id)
}

fn write_response(mut writer: impl Write, output: &CoshHookOutput) -> Result<(), CoshHookError> {
    serde_json::to_writer(&mut writer, output).map_err(CoshHookError::InvalidOutput)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aw_contracts::common::Digest;
    use aw_contracts::context::ContextProjectionCandidate;
    use aw_contracts::ids::{ArtifactId, ProviderInvocationId};
    use aw_contracts::provider::{ProviderMeasurementKind, ProviderMeter, VersionedSchema};

    #[test]
    fn extracts_only_the_model_visible_cosh_slot() {
        let response = serde_json::json!({
            "llmContent": "model text",
            "returnDisplay": "operator text"
        });

        assert_eq!(
            model_visible_content(&response).as_deref(),
            Some("model text")
        );
    }

    #[test]
    fn operator_display_is_not_treated_as_model_context() {
        let response = serde_json::json!({
            "returnDisplay": "operator-only text"
        });

        assert_eq!(model_visible_content(&response), None);
    }

    #[test]
    fn error_tool_result_bypasses_provider_discovery() {
        let input = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "shell",
            "tool_response": {"llmContent": "sandbox denied"},
            "tool_response_is_error": true,
            "execution_scope": {
                "environment_id": EnvironmentId::new(),
                "execution_context_id": ExecutionContextId::new(),
                "actor_id": ActorId::new(),
                "agent_session_id": AgentSessionId::new(),
                "turn_id": TurnId::new(),
                "tool_use_id": ToolUseId::new()
            }
        });
        let mut output = Vec::new();
        let config = CoshHookConfig {
            provider_source: ProviderManifestSource::File("/provider-does-not-exist".into()),
            provider_admission: ProviderAdmissionOptions::default(),
            target: local_host_target("test-host").expect("target is valid"),
            preferred_provider_id: None,
            allow_unenforced_provider: false,
        };

        let run = run_cosh_post_tool_use(
            serde_json::to_vec(&input)
                .expect("hook input serializes")
                .as_slice(),
            &mut output,
            &config,
        )
        .expect("error results bypass before Provider discovery");

        assert!(!run.replacement_requested);
        assert!(run.receipt.is_none());
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("hook output is JSON"),
            serde_json::json!({})
        );
    }

    #[test]
    fn reversible_candidate_emits_replacement_and_savings() {
        let prepared = prepared_result(Some(candidate()), ProviderDisposition::Produced);
        let output = hook_output(&prepared);
        let encoded = serde_json::to_value(output).expect("hook output serializes");

        assert_eq!(
            encoded.pointer("/hookSpecificOutput/updatedToolResponse"),
            Some(&Value::String("small context".to_owned()))
        );
        assert_eq!(
            encoded.get("systemMessage").and_then(Value::as_str),
            Some("AW · tokenless · estimated context 359→110 tokens · saved 69%")
        );
    }

    #[test]
    fn retrievable_candidate_waits_for_a_retrieval_contract() {
        let mut retrievable = candidate();
        retrievable.reversibility = ContextReversibility::Retrievable;
        let prepared = prepared_result(Some(retrievable), ProviderDisposition::Produced);

        let output = hook_output(&prepared);

        assert!(output.hook_specific_output.is_none());
        assert!(output.system_message.is_none());
    }

    #[test]
    fn empty_candidate_is_not_reported_as_adopted() {
        let mut empty = candidate();
        empty.content.clear();
        let prepared = prepared_result(Some(empty), ProviderDisposition::Produced);

        let output = hook_output(&prepared);

        assert!(output.hook_specific_output.is_none());
        assert!(output.system_message.is_none());
    }

    #[test]
    fn failed_provider_keeps_original_without_content_in_message() {
        let prepared = prepared_result(None, ProviderDisposition::Failed);
        let output = hook_output(&prepared);
        let encoded = serde_json::to_value(output).expect("hook output serializes");

        assert!(encoded.get("hookSpecificOutput").is_none());
        assert_eq!(
            encoded.get("systemMessage").and_then(Value::as_str),
            Some("AW · tokenless · original tool result kept (failed)")
        );
    }

    fn candidate() -> ContextProjectionCandidate {
        ContextProjectionCandidate {
            source_artifact_id: ArtifactId::new(),
            source_digest: digest('a'),
            content: "small context".to_owned(),
            media_type: name("text/plain"),
            content_type: None,
            transform_chain: vec![name("toon")],
            reversibility: ContextReversibility::Lossless,
        }
    }

    fn prepared_result(
        candidate: Option<ContextProjectionCandidate>,
        disposition: ProviderDisposition,
    ) -> PreparedToolResult {
        let source_artifact_id = candidate
            .as_ref()
            .map(|candidate| candidate.source_artifact_id.clone())
            .unwrap_or_default();
        let source_digest = candidate
            .as_ref()
            .map(|candidate| candidate.source_digest.clone())
            .unwrap_or_else(|| digest('a'));
        PreparedToolResult {
            source_artifact_id,
            source_digest,
            candidate,
            receipt: receipt(disposition),
        }
    }

    fn receipt(disposition: ProviderDisposition) -> ProviderReceipt {
        ProviderReceipt {
            invocation_id: ProviderInvocationId::new(),
            provider_id: name("tokenless"),
            provider_version: name("0.7.14"),
            manifest_digest: digest('b'),
            binding_id: None,
            provider_generation: None,
            capability: schema("context.projection.prepare"),
            scope: aw_contracts::provider::ExecutionScope {
                target: local_host_target("test-host").expect("target is valid"),
                environment_id: EnvironmentId::new(),
                execution_context_id: ExecutionContextId::new(),
                actor_id: ActorId::new(),
                agent_session_id: Some(AgentSessionId::new()),
                work_id: None,
                attempt_id: None,
                turn_id: Some(TurnId::new()),
                tool_use_id: Some(ToolUseId::new()),
            },
            disposition,
            output_schema: None,
            output_digest: None,
            output_bytes: None,
            error: None,
            meters: vec![
                ProviderMeter {
                    meter_id: name("context.source_tokens"),
                    unit: name("tokens"),
                    measurement_kind: ProviderMeasurementKind::Estimate,
                    method: Some(name("heuristic-v1")),
                    value: 359,
                },
                ProviderMeter {
                    meter_id: name("context.prepared_tokens"),
                    unit: name("tokens"),
                    measurement_kind: ProviderMeasurementKind::Estimate,
                    method: Some(name("heuristic-v1")),
                    value: 110,
                },
            ],
            evidence: Vec::new(),
            started_at_ms: 1,
            completed_at_ms: 2,
        }
    }

    fn schema(id: &str) -> VersionedSchema {
        VersionedSchema {
            id: name(id),
            version: 1,
        }
    }

    fn name(value: &str) -> BoundedName {
        BoundedName::new(value).expect("test name is bounded")
    }

    fn digest(value: char) -> Digest {
        Digest::parse(value.to_string().repeat(64)).expect("test digest is canonical")
    }
}
