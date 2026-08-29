//! Gateway headers, actors, and runtime context built on shared AW values.

use serde::{de, Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub use aw_contracts::common::{
    BoundedName, BoundedOpaque, BoundedStringError, BoundedText, Digest, DigestError,
    IdempotencyKey, TargetRef, MAX_IDEMPOTENCY_KEY_BYTES, MAX_NAME_BYTES, MAX_OPAQUE_BYTES,
    MAX_TEXT_BYTES,
};

use crate::{
    external::ExternalRef,
    ids::{
        ActorId, AgentSessionId, ApprovalId, ExecutionId, InstallationId, MessageId, PermitId,
        RunId, RuntimeBindingId, RuntimeInstanceId, TaskId,
    },
};

/// Current Gateway command schema version.
pub const CONTRACT_SCHEMA_VERSION: u16 = 1;
/// Durable Task event payload schema version.
///
/// Keep this independent from ingress and Runtime wire versions. A bump
/// requires a SQLite schema migration that rewrites every persisted Task event
/// and projection before current readers can open the database.
pub const TASK_EVENT_SCHEMA_VERSION: u16 = 1;
/// Current Runtime command and event schema version.
pub const RUNTIME_CONTRACT_SCHEMA_VERSION: u16 = 4;

/// Stable schema discriminator for a contract envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractSchema {
    /// Gateway ingress command schema.
    #[serde(rename = "cosh.gateway.command")]
    GatewayCommand,
    /// Durable Task lifecycle event schema.
    #[serde(rename = "cosh.task.event")]
    TaskEvent,
    /// Neutral Agent Runtime command schema.
    #[serde(rename = "cosh.runtime.command")]
    RuntimeCommand,
    /// Neutral Agent Runtime event schema.
    #[serde(rename = "cosh.runtime.event")]
    RuntimeEvent,
}

/// Failure returned when an envelope declares an unsupported schema version.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unsupported contract schema version {actual}; expected {expected}")]
pub struct SchemaVersionError {
    /// Version accepted by this crate.
    pub expected: u16,
    /// Version declared by the envelope.
    pub actual: u16,
}

/// Failure returned when an envelope carries another contract schema.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("envelope schema {actual:?} does not match expected schema {expected:?}")]
pub struct EnvelopeSchemaError {
    /// Schema required by the envelope type.
    pub expected: ContractSchema,
    /// Schema declared in the decoded header.
    pub actual: ContractSchema,
}

/// Metadata common to every Gateway domain envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContractHeader {
    /// Domain schema carried by the envelope.
    pub schema: ContractSchema,
    /// Version of the domain schema, independent from ACP and Core versions.
    pub schema_version: u16,
    /// Unique identity of this command or event.
    pub message_id: MessageId,
    /// Milliseconds since the Unix epoch recorded by the producer.
    pub occurred_at_ms: u64,
    /// Lifecycle identities propagated with the message.
    pub correlation: Correlation,
}

impl ContractHeader {
    /// Creates a header at the current supported domain schema version.
    #[must_use]
    pub fn new(
        schema: ContractSchema,
        message_id: MessageId,
        occurred_at_ms: u64,
        correlation: Correlation,
    ) -> Self {
        Self {
            schema,
            schema_version: expected_schema_version(schema),
            message_id,
            occurred_at_ms,
            correlation,
        }
    }

    /// Rejects versions that this crate cannot interpret safely.
    pub fn validate_version(&self) -> Result<(), SchemaVersionError> {
        let expected = expected_schema_version(self.schema);
        if self.schema_version == expected {
            Ok(())
        } else {
            Err(SchemaVersionError {
                expected,
                actual: self.schema_version,
            })
        }
    }

    /// Rejects a header used with a different envelope type.
    pub fn validate_schema(&self, expected: ContractSchema) -> Result<(), EnvelopeSchemaError> {
        if self.schema == expected {
            Ok(())
        } else {
            Err(EnvelopeSchemaError {
                expected,
                actual: self.schema,
            })
        }
    }
}

const fn expected_schema_version(schema: ContractSchema) -> u16 {
    match schema {
        ContractSchema::RuntimeCommand | ContractSchema::RuntimeEvent => {
            RUNTIME_CONTRACT_SCHEMA_VERSION
        }
        ContractSchema::GatewayCommand => CONTRACT_SCHEMA_VERSION,
        ContractSchema::TaskEvent => TASK_EVENT_SCHEMA_VERSION,
    }
}

impl<'de> Deserialize<'de> for ContractHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireHeader {
            schema: ContractSchema,
            schema_version: u16,
            message_id: MessageId,
            occurred_at_ms: u64,
            correlation: Correlation,
        }

        let wire = WireHeader::deserialize(deserializer)?;
        let header = Self {
            schema: wire.schema,
            schema_version: wire.schema_version,
            message_id: wire.message_id,
            occurred_at_ms: wire.occurred_at_ms,
            correlation: wire.correlation,
        };
        header.validate_version().map_err(de::Error::custom)?;
        Ok(header)
    }
}

/// Internal identities propagated across ingress, Task, Runtime, and execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correlation {
    /// Gateway installation that allocated the identities.
    pub installation_id: InstallationId,
    /// Authenticated actor, when resolution has completed.
    pub actor_id: Option<ActorId>,
    /// Durable Task owning the lifecycle.
    pub task_id: Option<TaskId>,
    /// Current Task execution attempt.
    pub run_id: Option<RunId>,
    /// COSH-owned logical Agent session.
    pub agent_session_id: Option<AgentSessionId>,
    /// Fenced Runtime binding producing the message.
    pub runtime_binding_id: Option<RuntimeBindingId>,
    /// Approval relevant to this message.
    pub approval_id: Option<ApprovalId>,
    /// Permit relevant to this message.
    pub permit_id: Option<PermitId>,
    /// Governed execution relevant to this message.
    pub execution_id: Option<ExecutionId>,
    /// Direct accepted message that caused this message.
    pub causation_message_id: Option<MessageId>,
}

impl Correlation {
    /// Starts an empty lifecycle correlation for one installation.
    #[must_use]
    pub fn new(installation_id: InstallationId) -> Self {
        Self {
            installation_id,
            actor_id: None,
            task_id: None,
            run_id: None,
            agent_session_id: None,
            runtime_binding_id: None,
            approval_id: None,
            permit_id: None,
            execution_id: None,
            causation_message_id: None,
        }
    }
}

/// Source category of an authenticated actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// Interactive human principal.
    Human,
    /// Locally configured automation principal.
    Automation,
    /// Operating-system service principal.
    Service,
}

/// Authentication strength established by an ingress adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthAssurance {
    /// Local operating-system identity was verified.
    LocalOs,
    /// A channel or web identity assertion was verified.
    RemoteVerified,
    /// A configured automation credential was verified.
    AutomationCredential,
}

/// Authenticated actor identity supplied by an ingress identity resolver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorRef {
    /// COSH-owned actor identity.
    pub actor_id: ActorId,
    /// Actor source category.
    pub actor_kind: ActorKind,
    /// Bounded identity issuer name.
    pub issuer: BoundedName,
    /// Assurance established by the adapter.
    pub assurance: AuthAssurance,
}

/// Workspace supplied to a newly opened Agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRef {
    /// Digest of the canonical workspace scope.
    pub scope_digest: Digest,
    /// Optional safe display label.
    pub display_name: Option<BoundedText>,
}

/// Runtime choice requested by a Task command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSelector {
    /// Runtime adapter kind, such as an ACP or Core bridge.
    pub runtime: BoundedName,
    /// Optional configured runtime profile.
    pub profile: Option<BoundedName>,
}

/// Fenced binding between a Task Run and an external Agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBindingRef {
    /// COSH binding identity.
    pub binding_id: RuntimeBindingId,
    /// Task owning the binding.
    pub task_id: TaskId,
    /// Run owning the binding.
    pub run_id: RunId,
    /// COSH logical Agent session.
    pub agent_session_id: AgentSessionId,
    /// Supervised child process identity.
    pub runtime_instance_id: RuntimeInstanceId,
    /// Process generation used to reject stale output.
    pub runtime_generation: u64,
    /// Scoped provider or ACP session reference.
    pub external_session: ExternalRef,
}

/// Content exchanged with an Agent Runtime without transport-specific types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentPart {
    /// Bounded UTF-8 text.
    Text {
        /// Text content.
        text: BoundedText,
    },
    /// Link to a resource resolved outside the contract layer.
    ResourceLink {
        /// Opaque bounded resource locator.
        uri: BoundedOpaque,
        /// Optional safe display label.
        label: Option<BoundedText>,
    },
}
