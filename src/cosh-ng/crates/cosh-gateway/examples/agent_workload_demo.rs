//! Deterministic Agent Workload scenarios reduced from durable Task events.

use std::error::Error;
use std::io;

use cosh_gateway::agent_workload::AgentWorkloadView;
use cosh_gateway_contracts::common::{
    BoundedName, BoundedOpaque, ContractHeader, ContractSchema, Correlation, Digest,
    RuntimeBindingRef, RuntimeSelector, TargetRef,
};
use cosh_gateway_contracts::error::{ContractError, ErrorCategory};
use cosh_gateway_contracts::external::{ExternalRef, ExternalRefKind};
use cosh_gateway_contracts::ids::{
    ActorId, AgentSessionId, ExecutionId, InstallationId, MessageId, PermitId, RunId,
    RuntimeBindingId, RuntimeInstanceId, TaskId,
};
use cosh_gateway_contracts::task::{TaskEvent, TaskEventEnvelope, UncertaintyCode};

struct ScenarioIds {
    task_id: TaskId,
    actor_id: ActorId,
    installation_id: InstallationId,
    first_run_id: RunId,
    second_run_id: RunId,
    agent_session_id: AgentSessionId,
    runtime_binding_id: RuntimeBindingId,
    runtime_instance_id: RuntimeInstanceId,
}

impl ScenarioIds {
    fn fixed() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            task_id: TaskId::parse("tsk_11111111-1111-4111-8111-111111111111")?,
            actor_id: ActorId::parse("act_22222222-2222-4222-8222-222222222222")?,
            installation_id: InstallationId::parse("ins_55555555-5555-4555-8555-555555555555")?,
            first_run_id: RunId::parse("run_33333333-3333-4333-8333-333333333333")?,
            second_run_id: RunId::parse("run_44444444-4444-4444-8444-444444444444")?,
            agent_session_id: AgentSessionId::parse("ags_88888888-8888-4888-8888-888888888888")?,
            runtime_binding_id: RuntimeBindingId::parse(
                "rtb_99999999-9999-4999-8999-999999999999",
            )?,
            runtime_instance_id: RuntimeInstanceId::parse(
                "rti_aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            )?,
        })
    }
}

fn runtime() -> Result<RuntimeSelector, Box<dyn Error>> {
    Ok(RuntimeSelector {
        runtime: BoundedName::new("core")?,
        profile: Some(BoundedName::new("gateway-brokered-v1")?),
    })
}

fn target() -> Result<TargetRef, Box<dyn Error>> {
    Ok(TargetRef {
        kind: BoundedName::new("workspace")?,
        authority: BoundedName::new("cosh")?,
        identifier: BoundedOpaque::new("demo-project")?,
    })
}

fn envelope(ids: &ScenarioIds, revision: u64, event: TaskEvent) -> TaskEventEnvelope {
    let mut correlation = Correlation::new(ids.installation_id.clone());
    correlation.actor_id = Some(ids.actor_id.clone());
    correlation.task_id = Some(ids.task_id.clone());
    TaskEventEnvelope {
        header: ContractHeader::new(
            ContractSchema::TaskEvent,
            MessageId::new(),
            revision,
            correlation,
        ),
        task_id: ids.task_id.clone(),
        revision,
        event,
    }
}

fn submitted(ids: &ScenarioIds) -> Result<TaskEventEnvelope, Box<dyn Error>> {
    Ok(envelope(
        ids,
        1,
        TaskEvent::TaskSubmitted {
            intent_digest: Digest::parse("a".repeat(64))?,
            target: target()?,
        },
    ))
}

fn success(ids: &ScenarioIds) -> Result<Vec<TaskEventEnvelope>, Box<dyn Error>> {
    Ok(vec![
        submitted(ids)?,
        envelope(
            ids,
            2,
            TaskEvent::TaskQueued {
                run_id: ids.first_run_id.clone(),
                runtime: runtime()?,
            },
        ),
        envelope(
            ids,
            3,
            TaskEvent::RunStarted {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(
            ids,
            4,
            TaskEvent::RunSucceeded {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(ids, 5, TaskEvent::TaskSucceeded),
    ])
}

fn retry(ids: &ScenarioIds) -> Result<Vec<TaskEventEnvelope>, Box<dyn Error>> {
    let error = ContractError::new(
        "runtime_lost",
        ErrorCategory::RuntimeUnavailable,
        true,
        "runtime stopped before completion",
    )?;
    Ok(vec![
        submitted(ids)?,
        envelope(
            ids,
            2,
            TaskEvent::TaskQueued {
                run_id: ids.first_run_id.clone(),
                runtime: runtime()?,
            },
        ),
        envelope(
            ids,
            3,
            TaskEvent::RunStarted {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(
            ids,
            4,
            TaskEvent::RunFailed {
                run_id: ids.first_run_id.clone(),
                error,
            },
        ),
        envelope(
            ids,
            5,
            TaskEvent::RunRetryQueued {
                previous_run_id: ids.first_run_id.clone(),
                next_run_id: ids.second_run_id.clone(),
            },
        ),
        envelope(
            ids,
            6,
            TaskEvent::RunStarted {
                run_id: ids.second_run_id.clone(),
            },
        ),
        envelope(
            ids,
            7,
            TaskEvent::RunSucceeded {
                run_id: ids.second_run_id.clone(),
            },
        ),
        envelope(ids, 8, TaskEvent::TaskSucceeded),
    ])
}

fn uncertain(ids: &ScenarioIds) -> Result<Vec<TaskEventEnvelope>, Box<dyn Error>> {
    let execution_id = ExecutionId::parse("exe_66666666-6666-4666-8666-666666666666")?;
    Ok(vec![
        submitted(ids)?,
        envelope(
            ids,
            2,
            TaskEvent::TaskQueued {
                run_id: ids.first_run_id.clone(),
                runtime: runtime()?,
            },
        ),
        envelope(
            ids,
            3,
            TaskEvent::RunStarted {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(
            ids,
            4,
            TaskEvent::ExecutionPlanned {
                execution_id: execution_id.clone(),
                permit_id: PermitId::parse("prm_77777777-7777-4777-8777-777777777777")?,
            },
        ),
        envelope(
            ids,
            5,
            TaskEvent::ExecutionUncertain {
                execution_id,
                reason: UncertaintyCode::TransportLost,
            },
        ),
    ])
}

fn provider_session(ids: &ScenarioIds) -> Result<Vec<TaskEventEnvelope>, Box<dyn Error>> {
    let binding = RuntimeBindingRef {
        binding_id: ids.runtime_binding_id.clone(),
        task_id: ids.task_id.clone(),
        run_id: ids.first_run_id.clone(),
        agent_session_id: ids.agent_session_id.clone(),
        runtime_instance_id: ids.runtime_instance_id.clone(),
        runtime_generation: 1,
        external_session: ExternalRef {
            kind: ExternalRefKind::ProviderSession,
            authority: BoundedName::new("anthropic-managed-agents")?,
            scope_digest: Digest::parse("b".repeat(64))?,
            value: BoundedOpaque::new("provider-session-42")?,
        },
    };
    Ok(vec![
        submitted(ids)?,
        envelope(
            ids,
            2,
            TaskEvent::TaskQueued {
                run_id: ids.first_run_id.clone(),
                runtime: runtime()?,
            },
        ),
        envelope(
            ids,
            3,
            TaskEvent::RunStarted {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(
            ids,
            4,
            TaskEvent::RuntimeBound {
                run_id: ids.first_run_id.clone(),
                binding,
            },
        ),
        envelope(
            ids,
            5,
            TaskEvent::RunSucceeded {
                run_id: ids.first_run_id.clone(),
            },
        ),
        envelope(ids, 6, TaskEvent::TaskSucceeded),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let scenario = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "success".to_owned());
    let ids = ScenarioIds::fixed()?;
    let events = match scenario.as_str() {
        "success" => success(&ids)?,
        "retry" => retry(&ids)?,
        "uncertain" => uncertain(&ids)?,
        "provider-session" => provider_session(&ids)?,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scenario must be success, retry, uncertain, or provider-session",
            )
            .into())
        }
    };
    let view = AgentWorkloadView::replay(&events)?;
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}
