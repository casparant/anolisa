#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Core-owned execution context and context-preparation policy.
//!
//! Agent Environments establish a stable execution context here and submit
//! tool results without constructing Provider envelopes. Core selects an
//! admitted implementation, invokes it, and returns a candidate separately
//! from the content-free Provider receipt.

use std::num::TryFromIntError;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

use aw_contracts::common::{
    BoundedName, BoundedStringError, Digest, DigestError, IdempotencyKey, TargetRef,
};
use aw_contracts::context::{
    context_projection_prepare_capability, context_projection_prepare_input_contract,
    context_projection_prepare_output_contract, ContextArtifactOrigin, ContextContractBuildError,
    ContextProjectionCandidate, ToolResultSubmission,
};
use aw_contracts::ids::{
    ActorId, AgentSessionId, AgentWorkId, ArtifactId, AttemptId, EnvironmentId, ExecutionContextId,
    IdError, ProviderInvocationId, ToolUseId, TurnId,
};
use aw_contracts::provider::{
    CapabilityInvocation, ExecutionScope, ProviderAuthority, ProviderDisposition,
    ProviderHealthState, ProviderInvocationBudget, ProviderPayload, ProviderReceipt,
    ProviderScopeKind, ProviderSelection, SchemaReference, VersionedSchema,
};
use aw_provider_host::{
    canonical_json_v1_bytes, ProviderCatalog, ProviderGuaranteeState, ProviderHostError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use uuid::Uuid;

const MAX_TRANSFORM_CHAIN_ITEMS: usize = 64;

/// Caller-owned identities used to establish one governed Agent execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContextSpec {
    /// Host or remote environment in which Agent work is taking place.
    pub target: TargetRef,
    /// Agent Environment presenting work to Core.
    pub environment_id: EnvironmentId,
    /// Actor identity asserted at the caller's Core trust boundary.
    ///
    /// A service boundary must authenticate this assertion before using it for
    /// authorization. An in-process adapter may use it only for correlation.
    pub actor_id: ActorId,
    /// Logical Agent session when the Environment can identify one.
    pub agent_session_id: Option<AgentSessionId>,
    /// Durable Work identity when the execution belongs to managed Work.
    pub work_id: Option<AgentWorkId>,
    /// Attempt identity when the execution belongs to managed Work.
    pub attempt_id: Option<AttemptId>,
    /// Existing Core context propagated by an Agent Environment hook.
    ///
    /// Omit this only when beginning a new execution; Core then allocates the
    /// identity returned by [`AgentExecutionContext::execution_context_id`].
    pub execution_context_id: Option<ExecutionContextId>,
}

/// Stable Core context shared by all observed work in one Agent execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionContext {
    target: TargetRef,
    environment_id: EnvironmentId,
    execution_context_id: ExecutionContextId,
    actor_id: ActorId,
    agent_session_id: Option<AgentSessionId>,
    work_id: Option<AgentWorkId>,
    attempt_id: Option<AttemptId>,
}

impl AgentExecutionContext {
    /// Returns the governed target associated with this execution.
    #[must_use]
    pub fn target(&self) -> &TargetRef {
        &self.target
    }

    /// Returns the Agent Environment that established the context.
    #[must_use]
    pub fn environment_id(&self) -> &EnvironmentId {
        &self.environment_id
    }

    /// Returns the Core identity propagated across hooks for this execution.
    #[must_use]
    pub fn execution_context_id(&self) -> &ExecutionContextId {
        &self.execution_context_id
    }

    /// Returns the caller-asserted actor associated with this execution.
    #[must_use]
    pub fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// Returns the logical Agent session when one was supplied.
    #[must_use]
    pub fn agent_session_id(&self) -> Option<&AgentSessionId> {
        self.agent_session_id.as_ref()
    }

    /// Returns the durable Work identity when one was supplied.
    #[must_use]
    pub fn work_id(&self) -> Option<&AgentWorkId> {
        self.work_id.as_ref()
    }

    /// Returns the managed Work attempt when one was supplied.
    #[must_use]
    pub fn attempt_id(&self) -> Option<&AttemptId> {
        self.attempt_id.as_ref()
    }

    fn tool_scope(&self, turn_id: TurnId, tool_use_id: ToolUseId) -> ExecutionScope {
        ExecutionScope {
            target: self.target.clone(),
            environment_id: self.environment_id.clone(),
            execution_context_id: self.execution_context_id.clone(),
            actor_id: self.actor_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            work_id: self.work_id.clone(),
            attempt_id: self.attempt_id.clone(),
            turn_id: Some(turn_id),
            tool_use_id: Some(tool_use_id),
        }
    }
}

/// Core policy defaults applied to Provider invocations created for tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreConfig {
    /// Policy revision attributed to invocations created by this Core instance.
    pub policy_revision: u64,
    /// Maximum time Core grants one context-preparation Provider invocation.
    pub provider_wall_time_ms: u64,
    /// Maximum canonical output bytes Core accepts from one Provider invocation.
    pub provider_output_bytes: u64,
    /// Allow a Provider to read submitted content before OS controls enforce
    /// its declared network and filesystem permissions.
    ///
    /// Keep this disabled outside an explicit trusted-Provider PoC.
    pub allow_unenforced_providers: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            policy_revision: 1,
            provider_wall_time_ms: 2_000,
            provider_output_bytes: 64 * 1024 * 1024,
            allow_unenforced_providers: false,
        }
    }
}

/// Optional routing preference for one tool-result preparation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepareToolResultOptions {
    /// Exact Provider identity to use when multiple implementations qualify.
    pub preferred_provider_id: Option<BoundedName>,
}

/// Core result for one tool output offered to context preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolResult {
    /// Core identity allocated to the immutable source artifact.
    pub source_artifact_id: ArtifactId,
    /// SHA-256 of the original tool-result content.
    pub source_digest: Digest,
    /// Provider proposal available for a later Core adoption decision.
    ///
    /// A bypassed, denied, failed, uncertain, or effect result carries no
    /// projection candidate even if a Provider returned transient output.
    pub candidate: Option<ContextProjectionCandidate>,
    /// Content-free terminal Provider facts safe for persistence and display.
    pub receipt: ProviderReceipt,
}

/// Core policy owner over one admitted Provider catalog.
#[derive(Debug)]
pub struct Core {
    providers: ProviderCatalog,
    config: CoreConfig,
}

impl Core {
    /// Creates Core with production-safe default Provider invocation ceilings.
    #[must_use]
    pub fn new(providers: ProviderCatalog) -> Self {
        Self {
            providers,
            config: CoreConfig::default(),
        }
    }

    /// Creates Core with explicit invocation policy.
    ///
    /// # Errors
    ///
    /// Returns an error when either Provider resource ceiling is zero.
    pub fn with_config(providers: ProviderCatalog, config: CoreConfig) -> Result<Self, CoreError> {
        if config.provider_wall_time_ms == 0 || config.provider_output_bytes == 0 {
            return Err(CoreError::InvalidConfig);
        }
        Ok(Self { providers, config })
    }

    /// Establishes or resumes one stable Agent execution context.
    ///
    /// When `spec.execution_context_id` is absent, Core allocates a new
    /// identity. A propagated identity is retained exactly so several COSH or
    /// third-party hook calls remain attached to the same execution.
    ///
    /// # Errors
    ///
    /// Returns an error when an Attempt is supplied without its Work identity.
    pub fn establish_execution_context(
        &self,
        spec: SessionContextSpec,
    ) -> Result<AgentExecutionContext, CoreError> {
        if spec.attempt_id.is_some() && spec.work_id.is_none() {
            return Err(CoreError::AttemptWithoutWork);
        }
        Ok(AgentExecutionContext {
            target: spec.target,
            environment_id: spec.environment_id,
            execution_context_id: spec.execution_context_id.unwrap_or_default(),
            actor_id: spec.actor_id,
            agent_session_id: spec.agent_session_id,
            work_id: spec.work_id,
            attempt_id: spec.attempt_id,
        })
    }

    /// Offers one observed tool result to the context-projection Capability.
    ///
    /// Core allocates the artifact and invocation identities, computes source
    /// and canonical input digests, binds the Tool Call scope, chooses an exact
    /// admitted Provider release, and applies its deadline and output budget.
    /// The returned candidate remains advice; this method does not replace the
    /// Agent's original tool result.
    ///
    /// # Errors
    ///
    /// Returns an error for an incomplete tool scope, no unique eligible
    /// Provider route, malformed Provider output, clock failure, or Host error.
    pub fn prepare_tool_result(
        &self,
        context: &AgentExecutionContext,
        turn_id: TurnId,
        tool_use_id: ToolUseId,
        submission: ToolResultSubmission,
        options: PrepareToolResultOptions,
    ) -> Result<PreparedToolResult, CoreError> {
        if context.agent_session_id.is_none() {
            return Err(CoreError::ToolResultWithoutAgentSession);
        }

        let capability = context_projection_prepare_capability()?;
        let input_contract = context_projection_prepare_input_contract()?;
        let output_contract = context_projection_prepare_output_contract()?;
        let provider = self.select_context_provider(
            &capability,
            &input_contract,
            &output_contract,
            options.preferred_provider_id.as_ref(),
        )?;

        let source_digest = sha256_digest(submission.content.as_bytes())?;
        let artifact_id = context_artifact_id(
            context.execution_context_id(),
            &turn_id,
            &tool_use_id,
            &source_digest,
        )?;
        let input_body = context_projection_input(&artifact_id, &source_digest, &submission)?;
        let canonical_input = canonical_json_v1_bytes(&input_body)?;
        let input_digest = sha256_digest(&canonical_input)?;
        let invocation_id = ProviderInvocationId::new();
        let idempotency_key = tool_result_idempotency_key(&tool_use_id, &input_digest)?;
        let deadline_at_ms = unix_time_ms()?
            .checked_add(self.config.provider_wall_time_ms)
            .ok_or(CoreError::DeadlineOverflow)?;
        let invocation = CapabilityInvocation {
            invocation_id,
            provider,
            capability,
            scope: context.tool_scope(turn_id, tool_use_id),
            binding_id: None,
            idempotency_key,
            policy_revision: self.config.policy_revision,
            deadline_at_ms,
            budget: ProviderInvocationBudget {
                wall_time_ms: self.config.provider_wall_time_ms,
                output_bytes: self.config.provider_output_bytes,
            },
            input: ProviderPayload {
                schema: input_contract.schema,
                digest: input_digest,
                body: input_body,
            },
        };
        let result = self.providers.invoke(&invocation, None)?;
        let candidate = if result.receipt.disposition == ProviderDisposition::Produced {
            let output = result
                .outcome
                .output
                .ok_or(CoreError::ProducedWithoutOutput)?;
            if output.schema != output_contract.schema {
                return Err(CoreError::UnexpectedOutputSchema {
                    actual: schema_label(&output.schema),
                });
            }
            let envelope: ContextProjectionOutput = serde_json::from_value(output.body)?;
            validate_candidate(&envelope.candidate, &artifact_id, &source_digest)?;
            Some(envelope.candidate)
        } else {
            None
        };

        Ok(PreparedToolResult {
            source_artifact_id: artifact_id,
            source_digest,
            candidate,
            receipt: result.receipt,
        })
    }

    fn select_context_provider(
        &self,
        capability: &VersionedSchema,
        input_contract: &SchemaReference,
        output_contract: &SchemaReference,
        preferred_provider_id: Option<&BoundedName>,
    ) -> Result<ProviderSelection, CoreError> {
        let graph = self.providers.capability_graph();
        let mut candidates = graph
            .capabilities
            .iter()
            .filter(|entry| {
                entry.capability == *capability
                    && entry.authority == ProviderAuthority::Advise
                    && entry.scopes.contains(&ProviderScopeKind::ToolCall)
                    && entry.health == ProviderHealthState::Ready
                    && entry.input_contract == *input_contract
                    && entry.output_contract == *output_contract
            })
            .collect::<Vec<_>>();

        if !self.config.allow_unenforced_providers && !candidates.is_empty() {
            candidates
                .retain(|entry| entry.guarantee != ProviderGuaranteeState::DeclaredNotEnforced);
            if candidates.is_empty() {
                return Err(CoreError::ProviderControlsNotEnforced);
            }
        }

        candidates.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        let selected = match preferred_provider_id {
            Some(preferred) => candidates
                .into_iter()
                .find(|entry| entry.provider_id == *preferred)
                .ok_or_else(|| CoreError::PreferredProviderUnavailable {
                    provider_id: preferred.as_str().to_owned(),
                })?,
            None => match candidates.as_slice() {
                [] => return Err(CoreError::ContextProjectionUnavailable),
                [only] => *only,
                many => {
                    return Err(CoreError::AmbiguousContextProviders {
                        provider_ids: many
                            .iter()
                            .map(|entry| entry.provider_id.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    })
                }
            },
        };
        Ok(ProviderSelection {
            provider_id: selected.provider_id.clone(),
            provider_version: selected.provider_version.clone(),
            manifest_digest: selected.manifest_digest.clone(),
        })
    }
}

/// Failure returned by execution-context or context-preparation policy.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An Attempt cannot exist outside durable Work.
    #[error("an Attempt identity requires an Agent Work identity")]
    AttemptWithoutWork,
    /// Tool-call scope requires a logical Agent session.
    #[error("tool-result preparation requires an Agent session identity")]
    ToolResultWithoutAgentSession,
    /// Core Provider ceilings must be enforceable and non-zero.
    #[error("Provider wall-time and output-byte limits must be non-zero")]
    InvalidConfig,
    /// No admitted Provider satisfies the exact context Contract and policy.
    #[error("no ready Advise Provider implements the exact Tool Call context Contract")]
    ContextProjectionUnavailable,
    /// A matching Provider would receive content without enforced isolation.
    #[error(
        "matching context Providers only declare network and filesystem controls; explicit trusted-Provider opt-in is required"
    )]
    ProviderControlsNotEnforced,
    /// More than one implementation qualifies and policy supplied no preference.
    #[error("multiple context Providers qualify; select one of: {provider_ids}")]
    AmbiguousContextProviders {
        /// Deterministically sorted eligible Provider identities.
        provider_ids: String,
    },
    /// The requested implementation does not satisfy current routing policy.
    #[error("preferred context Provider `{provider_id}` is not eligible")]
    PreferredProviderUnavailable {
        /// Requested Provider identity.
        provider_id: String,
    },
    /// Produced disposition requires a transient typed output.
    #[error("Provider reported `produced` without a transient output")]
    ProducedWithoutOutput,
    /// Provider output used a schema other than the selected canonical Contract.
    #[error("Provider returned unexpected output schema `{actual}`")]
    UnexpectedOutputSchema {
        /// Provider-returned schema label.
        actual: String,
    },
    /// Candidate does not identify the source artifact submitted by Core.
    #[error("Provider candidate does not refer to the submitted source artifact")]
    CandidateSourceMismatch,
    /// Candidate exceeds the canonical transformation-chain bound.
    #[error("Provider candidate transform chain exceeds {MAX_TRANSFORM_CHAIN_ITEMS} items")]
    TransformChainTooLong,
    /// System time precedes the Unix epoch.
    #[error("system clock precedes the Unix epoch")]
    ClockBeforeEpoch(#[source] SystemTimeError),
    /// System time cannot be represented by the public millisecond Contract.
    #[error("system time cannot be represented as u64 milliseconds")]
    ClockOutOfRange(#[source] TryFromIntError),
    /// Deadline arithmetic exceeded the public timestamp range.
    #[error("Provider invocation deadline overflowed")]
    DeadlineOverflow,
    /// A built-in context Contract constant is invalid.
    #[error(transparent)]
    ContextContract(#[from] ContextContractBuildError),
    /// A bounded Core value could not be constructed.
    #[error(transparent)]
    BoundedValue(#[from] BoundedStringError),
    /// A computed SHA-256 value violated its canonical representation.
    #[error(transparent)]
    Digest(#[from] DigestError),
    /// A deterministic Core identity could not be represented canonically.
    #[error(transparent)]
    Identity(#[from] IdError),
    /// Canonical input or Provider output JSON could not be encoded or decoded.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Provider discovery or invocation failed.
    #[error(transparent)]
    ProviderHost(#[from] ProviderHostError),
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ContextProjectionInput<'a> {
    artifact: ContextArtifactInput<'a>,
    boundary: ContextBoundary,
    constraints: ContextProjectionConstraints,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ContextArtifactInput<'a> {
    id: &'a ArtifactId,
    digest: &'a Digest,
    content: &'a str,
    media_type: &'a BoundedName,
    origin: ContextArtifactOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<&'a BoundedName>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ContextBoundary {
    PostTool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextProjectionConstraints {
    allow_text_reencoding: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextProjectionOutput {
    candidate: ContextProjectionCandidate,
}

fn context_projection_input(
    artifact_id: &ArtifactId,
    source_digest: &Digest,
    submission: &ToolResultSubmission,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(ContextProjectionInput {
        artifact: ContextArtifactInput {
            id: artifact_id,
            digest: source_digest,
            content: &submission.content,
            media_type: &submission.media_type,
            origin: submission.origin,
            tool_name: submission.tool_name.as_ref(),
        },
        boundary: ContextBoundary::PostTool,
        constraints: ContextProjectionConstraints {
            allow_text_reencoding: submission.allow_text_reencoding,
        },
    })
}

fn validate_candidate(
    candidate: &ContextProjectionCandidate,
    artifact_id: &ArtifactId,
    source_digest: &Digest,
) -> Result<(), CoreError> {
    if candidate.source_artifact_id != *artifact_id || candidate.source_digest != *source_digest {
        return Err(CoreError::CandidateSourceMismatch);
    }
    if candidate.transform_chain.len() > MAX_TRANSFORM_CHAIN_ITEMS {
        return Err(CoreError::TransformChainTooLong);
    }
    Ok(())
}

fn sha256_digest(bytes: &[u8]) -> Result<Digest, DigestError> {
    Digest::parse(format!("{:x}", Sha256::digest(bytes)))
}

fn context_artifact_id(
    execution_context_id: &ExecutionContextId,
    turn_id: &TurnId,
    tool_use_id: &ToolUseId,
    source_digest: &Digest,
) -> Result<ArtifactId, IdError> {
    let mut hasher = Sha256::new();
    hasher.update(b"agent-workload/context-artifact/v1");
    for value in [
        execution_context_id.as_str(),
        turn_id.as_str(),
        tool_use_id.as_str(),
        source_digest.as_str(),
    ] {
        hasher.update([0]);
        hasher.update(value.as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUIDv8 marks this as an application-defined, SHA-256-derived identity.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ArtifactId::parse(format!("art_{}", Uuid::from_bytes(bytes).hyphenated()))
}

fn unix_time_ms() -> Result<u64, CoreError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(CoreError::ClockBeforeEpoch)?;
    u64::try_from(elapsed.as_millis()).map_err(CoreError::ClockOutOfRange)
}

fn schema_label(schema: &VersionedSchema) -> String {
    format!("{}/v{}", schema.id.as_str(), schema.version)
}

fn tool_result_idempotency_key(
    tool_use_id: &ToolUseId,
    input_digest: &Digest,
) -> Result<IdempotencyKey, BoundedStringError> {
    IdempotencyKey::new(format!(
        "tool-result:{}:{}",
        tool_use_id.as_str(),
        input_digest.as_str()
    ))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
