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

use crate::CoreError;

/// Agent Environment boundary a Capability Plan serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanBoundary {
    /// A tool result that already exists.
    PostToolUse,
}

impl PlanBoundary {
    /// Returns the idempotency-key prefix that identifies this boundary.
    pub(crate) fn key_prefix(self) -> &'static str {
        match self {
            Self::PostToolUse => "tool-result",
        }
    }
}

/// Number of admitted implementations Core may invoke for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilitySelection {
    /// Exactly one implementation must survive routing, or the step fails.
    ExactlyOne,
}

/// Typed input and output family Core builds and decodes for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// `context.projection.prepare/v1`.
    ContextProjection,
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
    /// Typed payload family for this step.
    pub(crate) kind: StepKind,
}

/// Builds the Core Plan for one observed tool result.
///
/// # Errors
///
/// Returns an error if a compiled-in Contract constant is invalid, which
/// indicates a build-time defect rather than a caller mistake.
pub(crate) fn post_tool_use_plan() -> Result<Vec<CapabilityPlanStep>, CoreError> {
    Ok(vec![CapabilityPlanStep {
        capability: context_projection_prepare_capability()?,
        authority: ProviderAuthority::Advise,
        input_contract: context_projection_prepare_input_contract()?,
        output_contract: context_projection_prepare_output_contract()?,
        scope: ProviderScopeKind::ToolCall,
        selection: CapabilitySelection::ExactlyOne,
        kind: StepKind::ContextProjection,
    }])
}
