//! Plan resolution and bounded invocation for one Agent Environment event.
//!
//! Execution runs in three phases. Planning is pure. Resolution matches every
//! step against the Runtime Capability Graph *before* any Provider runs, so a
//! routing failure on one step can never discard results already collected from
//! another. Only then does Core invoke, in deterministic plan order.

use std::collections::BTreeMap;

use aw_contracts::common::{BoundedName, BoundedStringError, Digest, IdempotencyKey};
use aw_contracts::ids::ProviderInvocationId;
use aw_contracts::provider::{
    CapabilityInvocation, ExecutionScope, ProviderHealthState, ProviderInvocationBudget,
    ProviderInvocationResult, ProviderPayload, ProviderSelection, VersionedSchema,
};
use aw_contracts::security::{GateDegradation, ObservationGapReason};
use aw_provider_host::{ProviderGuaranteeState, RuntimeCapabilityEntry};
use serde_json::Value;

use crate::plan::{CapabilityPlanStep, CapabilitySelection, PlanBoundary, StepFailurePolicy};
use crate::{canonical_input_digest, unix_time_ms, Core, CoreError};

/// Routing preferences applied to Capability steps that admit exactly one route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityPreferences {
    /// Exact Provider chosen per Capability name, for single-route steps only.
    ///
    /// A preference naming a Capability that fans out across implementations, or
    /// a Capability absent from the plan, is rejected rather than silently
    /// narrowing the set of implementations Core consults.
    pub preferred_providers: BTreeMap<BoundedName, BoundedName>,
}

impl CapabilityPreferences {
    /// Builds preferences that pin one Provider for one Capability.
    ///
    /// # Errors
    ///
    /// Returns an error when either name violates its bounded contract.
    pub fn for_capability(
        capability_id: &str,
        provider_id: BoundedName,
    ) -> Result<Self, BoundedStringError> {
        let mut preferred_providers = BTreeMap::new();
        preferred_providers.insert(BoundedName::new(capability_id)?, provider_id);
        Ok(Self {
            preferred_providers,
        })
    }
}

/// Why routing produced no target for a step allowed to degrade.
///
/// Resolution reports its own small vocabulary, and each boundary maps it to the
/// vocabulary that boundary publishes. An observation gap and a gate degradation
/// are different public facts even when the routing cause is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteGap {
    /// No admitted implementation satisfies the Capability Contract.
    NoImplementation,
    /// A matching implementation only declares its isolation controls.
    ControlsNotEnforced,
    /// Several implementations qualify and routing policy named none.
    AmbiguousRoute,
}

impl RouteGap {
    /// Maps a routing cause to the Observe vocabulary.
    pub(crate) fn as_observation_reason(self) -> ObservationGapReason {
        match self {
            // A fan-out step invokes every qualifying implementation, so
            // ambiguity cannot arise there; report it as absent rather than
            // inventing a reason the Observe vocabulary does not carry.
            Self::NoImplementation | Self::AmbiguousRoute => ObservationGapReason::NoImplementation,
            Self::ControlsNotEnforced => ObservationGapReason::ControlsNotEnforced,
        }
    }

    /// Maps a routing cause to the Tool Call gate vocabulary.
    pub(crate) fn as_gate_degradation(self) -> GateDegradation {
        match self {
            Self::NoImplementation => GateDegradation::NoImplementation,
            Self::ControlsNotEnforced => GateDegradation::ControlsNotEnforced,
            Self::AmbiguousRoute => GateDegradation::AmbiguousRoute,
        }
    }
}

/// One plan step with its routing already decided.
pub(crate) struct ResolvedStep {
    pub(crate) step: CapabilityPlanStep,
    pub(crate) targets: Vec<ProviderSelection>,
    /// Why this step has no route, when its policy allows degrading.
    pub(crate) gap: Option<RouteGap>,
}

impl Core {
    /// Resolves every plan step against the current Runtime Capability Graph.
    ///
    /// Resolution completes for all steps before any Provider is invoked.
    pub(crate) fn resolve_plan(
        &self,
        steps: Vec<CapabilityPlanStep>,
        preferences: &CapabilityPreferences,
    ) -> Result<Vec<ResolvedStep>, CoreError> {
        let planned = steps
            .iter()
            .map(|step| step.capability.id.clone())
            .collect::<Vec<_>>();
        for capability in preferences.preferred_providers.keys() {
            let applicable = steps.iter().any(|step| {
                step.capability.id == *capability
                    && step.selection == CapabilitySelection::ExactlyOne
            });
            if !applicable {
                return Err(CoreError::PreferenceNotApplicable {
                    capability: capability.as_str().to_owned(),
                    planned: planned
                        .iter()
                        .map(BoundedName::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }

        steps
            .into_iter()
            .map(|step| self.resolve_step(step, preferences))
            .collect()
    }

    fn resolve_step(
        &self,
        step: CapabilityPlanStep,
        preferences: &CapabilityPreferences,
    ) -> Result<ResolvedStep, CoreError> {
        let graph = self.providers.capability_graph();
        let mut eligible = graph
            .capabilities
            .iter()
            .filter(|entry| {
                entry.capability == step.capability
                    && entry.authority == step.authority
                    && entry.scopes.contains(&step.scope)
                    && entry.health == ProviderHealthState::Ready
                    && entry.input_contract == step.input_contract
                    && entry.output_contract == step.output_contract
            })
            .collect::<Vec<_>>();

        let degrades = !matches!(step.failure, StepFailurePolicy::RejectPlan);
        if !self.config.allow_unenforced_providers && !eligible.is_empty() {
            eligible.retain(|entry| entry.guarantee != ProviderGuaranteeState::DeclaredNotEnforced);
            if eligible.is_empty() {
                if degrades {
                    return Ok(ResolvedStep {
                        step,
                        targets: Vec::new(),
                        gap: Some(RouteGap::ControlsNotEnforced),
                    });
                }
                return Err(CoreError::ProviderControlsNotEnforced);
            }
        }

        eligible.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
        for window in eligible.windows(2) {
            if window[0].provider_id == window[1].provider_id {
                return Err(CoreError::DuplicateCapabilityRoute {
                    provider_id: window[0].provider_id.as_str().to_owned(),
                    capability: schema_label(&step.capability),
                });
            }
        }

        fn select(entry: &RuntimeCapabilityEntry) -> ProviderSelection {
            ProviderSelection {
                provider_id: entry.provider_id.clone(),
                provider_version: entry.provider_version.clone(),
                manifest_digest: entry.manifest_digest.clone(),
            }
        }

        let preference = preferences.preferred_providers.get(&step.capability.id);
        match step.selection {
            CapabilitySelection::AllDistinctProviders => {
                if eligible.is_empty() {
                    return Ok(ResolvedStep {
                        step,
                        targets: Vec::new(),
                        gap: Some(RouteGap::NoImplementation),
                    });
                }
                let targets = eligible.iter().map(|entry| select(entry)).collect();
                Ok(ResolvedStep {
                    step,
                    targets,
                    gap: None,
                })
            }
            CapabilitySelection::ExactlyOne => {
                let selected = match preference {
                    Some(preferred) => eligible
                        .into_iter()
                        .find(|entry| entry.provider_id == *preferred)
                        .ok_or_else(|| CoreError::PreferredProviderUnavailable {
                            provider_id: preferred.as_str().to_owned(),
                        })?,
                    None => match eligible.as_slice() {
                        [] if degrades => {
                            return Ok(ResolvedStep {
                                step,
                                targets: Vec::new(),
                                gap: Some(RouteGap::NoImplementation),
                            })
                        }
                        [] => {
                            return Err(CoreError::CapabilityUnavailable {
                                capability: schema_label(&step.capability),
                            })
                        }
                        [only] => *only,
                        many if degrades => {
                            let _ = many;
                            return Ok(ResolvedStep {
                                step,
                                targets: Vec::new(),
                                gap: Some(RouteGap::AmbiguousRoute),
                            });
                        }
                        many => {
                            return Err(CoreError::AmbiguousCapabilityRoute {
                                capability: schema_label(&step.capability),
                                provider_ids: many
                                    .iter()
                                    .map(|entry| entry.provider_id.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            })
                        }
                    },
                };
                Ok(ResolvedStep {
                    step,
                    targets: vec![select(selected)],
                    gap: None,
                })
            }
        }
    }

    /// Invokes one resolved target under Core policy, deadline, and budget.
    pub(crate) fn invoke_step(
        &self,
        step: &CapabilityPlanStep,
        boundary: PlanBoundary,
        provider: ProviderSelection,
        scope: ExecutionScope,
        tool_use_id_text: &str,
        input_body: Value,
    ) -> Result<ProviderInvocationResult, CoreError> {
        let input_digest = canonical_input_digest(&input_body)?;
        let deadline_at_ms = unix_time_ms()?
            .checked_add(self.config.provider_wall_time_ms)
            .ok_or(CoreError::DeadlineOverflow)?;
        let invocation = CapabilityInvocation {
            invocation_id: ProviderInvocationId::new(),
            provider,
            capability: step.capability.clone(),
            scope,
            binding_id: None,
            idempotency_key: capability_idempotency_key(
                boundary,
                &step.capability,
                tool_use_id_text,
                &input_digest,
            )?,
            policy_revision: self.config.policy_revision,
            deadline_at_ms,
            budget: ProviderInvocationBudget {
                wall_time_ms: self.config.provider_wall_time_ms,
                output_bytes: self.config.provider_output_bytes,
            },
            input: ProviderPayload {
                schema: step.input_contract.schema.clone(),
                digest: input_digest,
                body: input_body,
            },
        };
        Ok(self.providers.invoke(&invocation, None)?)
    }
}

/// Derives the caller-scoped replay key for one Capability on one Tool Call.
///
/// The Capability identity is part of the key so two Capabilities observing the
/// same tool result do not share a replay identity. Two implementations of the
/// *same* Capability do share one key, which is correct: the key is scoped to
/// the caller, and each Provider only ever sees its own invocations.
pub(crate) fn capability_idempotency_key(
    boundary: PlanBoundary,
    capability: &VersionedSchema,
    tool_use_id_text: &str,
    input_digest: &Digest,
) -> Result<IdempotencyKey, BoundedStringError> {
    IdempotencyKey::new(format!(
        "{}:{}:{}:{}",
        boundary.key_prefix(),
        tool_use_id_text,
        schema_label(capability),
        input_digest.as_str()
    ))
}

pub(crate) fn schema_label(schema: &VersionedSchema) -> String {
    format!("{}/v{}", schema.id.as_str(), schema.version)
}
