//! Core-owned Capability Plans for one Agent Environment event.
//!
//! A Plan is pure: it is built from compiled-in Contract constants and never
//! consults the Runtime Capability Graph. Plan ownership stays in Core so an
//! Agent Environment submits one event and cannot decide which Capabilities
//! apply, in what order, or under which failure policy.

use aw_contracts::context::{
    context_projection_prepare_capability, context_projection_prepare_input_contract,
    context_projection_prepare_output_contract,
};
use aw_contracts::provider::{
    ProviderAuthority, ProviderScopeKind, SchemaReference, VersionedSchema,
};
use aw_contracts::security::{
    security_code_inspect_capability, security_code_inspect_input_contract,
    security_code_inspect_output_contract, security_command_inspect_capability,
    security_command_inspect_input_contract, security_command_inspect_output_contract,
    security_content_inspect_capability, security_content_inspect_input_contract,
    security_content_inspect_output_contract,
};

use crate::CoreError;

/// Agent Environment boundary a Capability Plan serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanBoundary {
    /// A Tool Call that has not run yet and can still be stopped.
    PreToolUse,
    /// A tool result that already exists.
    PostToolUse,
}

impl PlanBoundary {
    /// Returns the idempotency-key prefix that identifies this boundary.
    pub(crate) fn key_prefix(self) -> &'static str {
        match self {
            Self::PreToolUse => "tool-call",
            Self::PostToolUse => "tool-result",
        }
    }
}

/// Number of admitted implementations Core may invoke for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilitySelection {
    /// Exactly one implementation must survive routing, or the step fails.
    ExactlyOne,
    /// Every distinct Provider that qualifies is invoked once.
    ///
    /// Observing facts is not a competition: two installed scanners both have
    /// something to say, and silently using only one of them would hide the
    /// other. Routing still admits at most one implementation per Provider.
    AllDistinctProviders,
}

/// Effect of a step that yields no usable result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepFailurePolicy {
    /// Reject the whole plan; the Environment keeps its original result.
    RejectPlan,
    /// Record why the fact is missing and continue with the remaining steps.
    RecordGapAndContinue,
    /// Resolve the Tool Call gate by the configured mediation default.
    ApplyMediationDefault,
}

/// Typed input and output family Core builds and decodes for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// `context.projection.prepare/v1`.
    ContextProjection,
    /// `security.content.inspect/v1`.
    ContentInspection,
    /// `security.code.inspect/v1`.
    CodeInspection,
    /// `security.command.inspect/v1`.
    CommandInspection,
}

/// One Capability Core decided to invoke for a single Environment event.
#[derive(Debug, Clone)]
pub(crate) struct CapabilityPlanStep {
    /// Provider-independent Capability identity.
    pub(crate) capability: VersionedSchema,
    /// Authority an implementation must declare to serve this step.
    pub(crate) authority: ProviderAuthority,
    /// Exact canonical input Contract.
    pub(crate) input_contract: SchemaReference,
    /// Exact canonical output Contract.
    pub(crate) output_contract: SchemaReference,
    /// Most specific scope the invocation belongs to.
    pub(crate) scope: ProviderScopeKind,
    /// How many implementations Core may invoke.
    pub(crate) selection: CapabilitySelection,
    /// What Core does when the step yields nothing usable.
    pub(crate) failure: StepFailurePolicy,
    /// Typed payload family for this step.
    pub(crate) kind: StepKind,
}

/// Builds the Core Plan for one observed tool result.
///
/// Observe steps precede the Advise step. Facts about the original artifact are
/// therefore recorded before any derived representation exists, and the data
/// flow already runs in the direction a future "do not compress content holding
/// a secret" policy would need.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid, which
/// indicates a build-time defect rather than a caller mistake.
pub(crate) fn post_tool_use_plan() -> Result<Vec<CapabilityPlanStep>, CoreError> {
    Ok(vec![
        CapabilityPlanStep {
            capability: security_content_inspect_capability()?,
            authority: ProviderAuthority::Observe,
            input_contract: security_content_inspect_input_contract()?,
            output_contract: security_content_inspect_output_contract()?,
            scope: ProviderScopeKind::ToolCall,
            selection: CapabilitySelection::AllDistinctProviders,
            failure: StepFailurePolicy::RecordGapAndContinue,
            kind: StepKind::ContentInspection,
        },
        CapabilityPlanStep {
            capability: security_code_inspect_capability()?,
            authority: ProviderAuthority::Observe,
            input_contract: security_code_inspect_input_contract()?,
            output_contract: security_code_inspect_output_contract()?,
            scope: ProviderScopeKind::ToolCall,
            selection: CapabilitySelection::AllDistinctProviders,
            failure: StepFailurePolicy::RecordGapAndContinue,
            kind: StepKind::CodeInspection,
        },
        CapabilityPlanStep {
            capability: context_projection_prepare_capability()?,
            authority: ProviderAuthority::Advise,
            input_contract: context_projection_prepare_input_contract()?,
            output_contract: context_projection_prepare_output_contract()?,
            scope: ProviderScopeKind::ToolCall,
            selection: CapabilitySelection::ExactlyOne,
            failure: StepFailurePolicy::RejectPlan,
            kind: StepKind::ContextProjection,
        },
    ])
}

/// Builds the Core Plan for one pending Tool Call.
///
/// Mediation admits exactly one implementation. Composing several verdicts needs
/// a stable precedence and conflict rule that does not exist yet, so a second
/// eligible implementation degrades the gate rather than being merged silently.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid, which
/// indicates a build-time defect rather than a caller mistake.
pub(crate) fn pre_tool_use_plan() -> Result<Vec<CapabilityPlanStep>, CoreError> {
    Ok(vec![CapabilityPlanStep {
        capability: security_command_inspect_capability()?,
        authority: ProviderAuthority::Mediate,
        input_contract: security_command_inspect_input_contract()?,
        output_contract: security_command_inspect_output_contract()?,
        scope: ProviderScopeKind::ToolCall,
        selection: CapabilitySelection::ExactlyOne,
        failure: StepFailurePolicy::ApplyMediationDefault,
        kind: StepKind::CommandInspection,
    }])
}
