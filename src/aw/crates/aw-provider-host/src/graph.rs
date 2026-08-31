//! Runtime Capability Graph projection over admitted Provider manifests.

use aw_contracts::common::{BoundedName, BoundedText, Digest};
use aw_contracts::provider::{
    ProviderAuthority, ProviderHealthState, ProviderScopeKind, SchemaReference, VersionedSchema,
};
use serde::{Deserialize, Serialize};

use super::AdmittedProvider;

/// Version of the headless Runtime Capability Graph projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeCapabilityGraphVersion {
    /// First graph shape over admitted Provider manifests.
    #[serde(rename = "runtime.agentic-os.sh/v1")]
    V1,
}

/// Deterministic view of the Capabilities available to this Provider Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilityGraph {
    /// Graph schema version.
    pub api_version: RuntimeCapabilityGraphVersion,
    /// One entry per admitted Provider and Capability revision.
    pub capabilities: Vec<RuntimeCapabilityEntry>,
}

/// Network access declared by a Provider package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNetworkAccess {
    /// Provider declares that it does not require network access.
    None,
}

/// Package-declared process and filesystem requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPermissionDeclaration {
    /// Network access required by the Provider.
    pub network: ProviderNetworkAccess,
    /// Whether the Provider asks to inherit the Host process environment.
    pub inherit_environment: bool,
    /// Symbolic filesystem locations the Provider declares it may read.
    pub filesystem_read: Vec<String>,
    /// Symbolic filesystem locations the Provider declares it may write.
    pub filesystem_write: Vec<String>,
}

/// Package-declared data handling behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDataDeclaration {
    /// Data categories read by the Provider.
    pub reads: Vec<BoundedName>,
    /// Data categories written by the Provider.
    pub writes: Vec<BoundedName>,
    /// Declared sensitivity propagation rule.
    pub sensitivity: BoundedName,
    /// Declared retention owner.
    pub retention: BoundedName,
    /// Declared telemetry behavior.
    pub telemetry: BoundedName,
}

/// Strength with which the Host currently enforces package declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderGuaranteeState {
    /// Declarations are visible but not fully enforced by an OS sandbox.
    DeclaredNotEnforced,
}

/// One Provider-backed Capability projected for status and conformance tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilityEntry {
    /// Stable Provider identity.
    pub provider_id: BoundedName,
    /// Exact Provider release admitted by the Host.
    pub provider_version: BoundedName,
    /// Exact admitted manifest revision.
    pub manifest_digest: Digest,
    /// Provider-independent Capability identity.
    pub capability: VersionedSchema,
    /// Authority exercised by this implementation.
    pub authority: ProviderAuthority,
    /// Canonical input Contract admitted for this Capability.
    pub input_contract: SchemaReference,
    /// Canonical output Contract admitted for this Capability.
    pub output_contract: SchemaReference,
    /// Supported invocation or binding scopes.
    pub scopes: Vec<ProviderScopeKind>,
    /// Process and filesystem requirements declared by the package.
    pub permissions: ProviderPermissionDeclaration,
    /// Data handling behavior declared by the package.
    pub data: ProviderDataDeclaration,
    /// Enforcement strength for the declarations above.
    pub guarantee: ProviderGuaranteeState,
    /// Current static admission and executable-readiness projection.
    pub health: ProviderHealthState,
    /// Safe reason when the projected health is not ready.
    pub reason: Option<BoundedText>,
}

pub(super) fn project(providers: &[AdmittedProvider]) -> RuntimeCapabilityGraph {
    let mut capabilities = providers
        .iter()
        .flat_map(|provider| {
            provider
                .capabilities
                .iter()
                .map(|capability| RuntimeCapabilityEntry {
                    provider_id: provider.descriptor.provider_id.clone(),
                    provider_version: provider.descriptor.provider_version.clone(),
                    manifest_digest: provider.descriptor.manifest_digest.clone(),
                    capability: capability.descriptor.capability.clone(),
                    authority: capability.descriptor.authority,
                    input_contract: capability.descriptor.input_contract.clone(),
                    output_contract: capability.descriptor.output_contract.clone(),
                    scopes: capability.descriptor.scopes.clone(),
                    permissions: provider.permissions.clone(),
                    data: provider.data.clone(),
                    guarantee: ProviderGuaranteeState::DeclaredNotEnforced,
                    health: ProviderHealthState::Ready,
                    reason: None,
                })
        })
        .collect::<Vec<_>>();
    capabilities.sort_by(|left, right| {
        left.provider_id
            .cmp(&right.provider_id)
            .then_with(|| left.capability.id.cmp(&right.capability.id))
            .then_with(|| left.capability.version.cmp(&right.capability.version))
    });
    RuntimeCapabilityGraph {
        api_version: RuntimeCapabilityGraphVersion::V1,
        capabilities,
    }
}
