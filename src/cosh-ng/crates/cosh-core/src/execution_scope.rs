//! Correlates COSH sessions, turns, and native tool calls with AW identities.

use aw_contracts::ids::{
    ActorId, AgentSessionId, EnvironmentId, ExecutionContextId, ToolUseId, TurnId,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// AW correlation attached to a COSH post-tool hook invocation.
///
/// The Provider-native tool call ID remains a separate hook field because it
/// belongs to the Agent protocol. These typed values correlate the local PoC;
/// they are not credentials and must not authorize work by themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionScopeCorrelation {
    /// Agent Environment serving the session.
    pub environment_id: EnvironmentId,
    /// Governed execution context shared by the session.
    pub execution_context_id: ExecutionContextId,
    /// Opaque local actor correlation allocated by COSH for this process.
    pub actor_id: ActorId,
    /// Logical Agent session correlated with the COSH session.
    pub agent_session_id: AgentSessionId,
    /// Prompt turn that produced the tool call.
    pub turn_id: TurnId,
    /// System-owned identity for this observed tool call.
    pub tool_use_id: ToolUseId,
}

/// Stable base identities owned by one `CoshCore` process.
#[derive(Debug, Clone)]
pub(crate) struct ExecutionScopeContext {
    environment_id: EnvironmentId,
    execution_context_id: ExecutionContextId,
    actor_id: ActorId,
    agent_session_id: AgentSessionId,
}

impl ExecutionScopeContext {
    /// Establishes one correlation context for a COSH session.
    pub(crate) fn for_session(session_id: &str) -> Self {
        Self {
            environment_id: EnvironmentId::new(),
            execution_context_id: ExecutionContextId::new(),
            actor_id: ActorId::new(),
            agent_session_id: agent_session_id_from_cosh(session_id).unwrap_or_default(),
        }
    }

    /// Derives a stable scope for one Provider-native tool call.
    pub(crate) fn tool_call_scope(
        &self,
        cosh_turn_id: &str,
        native_tool_use_id: &str,
    ) -> ExecutionScopeCorrelation {
        let turn_id = turn_id_from_cosh(cosh_turn_id).unwrap_or_default();
        ExecutionScopeCorrelation {
            environment_id: self.environment_id.clone(),
            execution_context_id: self.execution_context_id.clone(),
            actor_id: self.actor_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            tool_use_id: tool_use_id_from_cosh(
                &self.agent_session_id,
                &turn_id,
                native_tool_use_id,
            ),
            turn_id,
        }
    }
}

fn canonical_uuid(value: &str) -> Option<String> {
    let uuid = Uuid::parse_str(value).ok()?;
    let canonical = uuid.hyphenated().to_string();
    (canonical == value).then_some(canonical)
}

fn agent_session_id_from_cosh(session_id: &str) -> Option<AgentSessionId> {
    let uuid = canonical_uuid(session_id)?;
    AgentSessionId::parse(format!("ags_{uuid}")).ok()
}

fn turn_id_from_cosh(turn_id: &str) -> Option<TurnId> {
    let uuid = canonical_uuid(turn_id)?;
    TurnId::parse(format!("trn_{uuid}")).ok()
}

fn tool_use_id_from_cosh(
    agent_session_id: &AgentSessionId,
    turn_id: &TurnId,
    native_tool_use_id: &str,
) -> ToolUseId {
    let mut hasher = Sha256::new();
    hasher.update(b"agent-workload/cosh-tool-use/v1");
    for value in [
        agent_session_id.as_str(),
        turn_id.as_str(),
        native_tool_use_id,
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
    ToolUseId::parse(format!("tol_{}", Uuid::from_bytes(bytes).hyphenated()))
        .expect("UUIDv8 has the canonical ToolUseId representation")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
    const TURN_ID: &str = "22222222-2222-4222-8222-222222222222";

    #[test]
    fn canonical_cosh_session_and_turn_ids_keep_their_uuid_bodies() {
        let context = ExecutionScopeContext::for_session(SESSION_ID);
        let scope = context.tool_call_scope(TURN_ID, "provider-call-1");

        assert_eq!(
            scope.agent_session_id.as_str(),
            "ags_11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(
            scope.turn_id.as_str(),
            "trn_22222222-2222-4222-8222-222222222222"
        );
    }

    #[test]
    fn one_native_tool_call_reuses_all_correlation_ids() {
        let context = ExecutionScopeContext::for_session(SESSION_ID);
        let first = context.tool_call_scope(TURN_ID, "provider-call-1");
        let second = context.tool_call_scope(TURN_ID, "provider-call-1");

        assert_eq!(first.environment_id, second.environment_id);
        assert_eq!(first.execution_context_id, second.execution_context_id);
        assert_eq!(first.actor_id, second.actor_id);
        assert_eq!(first.agent_session_id, second.agent_session_id);
        assert_eq!(first.turn_id, second.turn_id);
        assert_eq!(first.tool_use_id, second.tool_use_id);

        let other = context.tool_call_scope(TURN_ID, "provider-call-2");
        assert_ne!(first.tool_use_id, other.tool_use_id);
    }

    #[test]
    fn noncanonical_session_id_gets_one_process_stable_agent_session_id() {
        let context = ExecutionScopeContext::for_session("provider-session");
        let first = context.tool_call_scope(TURN_ID, "provider-call-1");
        let second = context.tool_call_scope(TURN_ID, "provider-call-1");

        assert_eq!(first.agent_session_id, second.agent_session_id);
        assert!(first.agent_session_id.as_str().starts_with("ags_"));
    }
}
