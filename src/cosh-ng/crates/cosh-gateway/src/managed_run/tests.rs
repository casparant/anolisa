use cosh_gateway_contracts::common::{
    BoundedName, BoundedOpaque, ContractHeader, ContractSchema, Correlation, Digest,
    RuntimeSelector, TargetRef,
};
use cosh_gateway_contracts::error::{ContractError, ErrorCategory};
use cosh_gateway_contracts::ids::{
    ActorId, ExecutionId, InstallationId, MessageId, PermitId, RunId, TaskId,
};
use cosh_gateway_contracts::task::{
    CancellationStage, ExecutionOutcome, SuspensionCode, TaskEvent, TaskEventEnvelope,
    UncertaintyCode,
};

use super::*;

fn target() -> TargetRef {
    TargetRef {
        kind: BoundedName::new("workspace").unwrap(),
        authority: BoundedName::new("cosh").unwrap(),
        identifier: BoundedOpaque::new("managed-run-demo").unwrap(),
    }
}

fn runtime() -> RuntimeSelector {
    RuntimeSelector {
        runtime: BoundedName::new("core").unwrap(),
        profile: Some(BoundedName::new("gateway-brokered-v1").unwrap()),
    }
}

fn event(
    task_id: &TaskId,
    actor_id: &ActorId,
    revision: u64,
    event: TaskEvent,
) -> TaskEventEnvelope {
    let mut correlation = Correlation::new(InstallationId::new());
    correlation.actor_id = Some(actor_id.clone());
    correlation.task_id = Some(task_id.clone());
    TaskEventEnvelope {
        header: ContractHeader::new(
            ContractSchema::TaskEvent,
            MessageId::new(),
            revision,
            correlation,
        ),
        task_id: task_id.clone(),
        revision,
        event,
    }
}

fn submitted(task_id: &TaskId, actor_id: &ActorId) -> TaskEventEnvelope {
    event(
        task_id,
        actor_id,
        1,
        TaskEvent::TaskSubmitted {
            intent_digest: Digest::parse("a".repeat(64)).unwrap(),
            target: target(),
        },
    )
}

fn failure(code: &str) -> ContractError {
    ContractError::new(
        code,
        ErrorCategory::RuntimeUnavailable,
        true,
        "runtime stopped",
    )
    .unwrap()
}

#[test]
fn runtime_success_does_not_claim_verification_or_disposition() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let run_id = RunId::new();
    let view = ManagedRunView::replay(&[
        submitted(&task_id, &actor_id),
        event(
            &task_id,
            &actor_id,
            2,
            TaskEvent::TaskQueued {
                run_id: run_id.clone(),
                runtime: runtime(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::RunStarted {
                run_id: run_id.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            4,
            TaskEvent::RunSucceeded {
                run_id: run_id.clone(),
            },
        ),
        event(&task_id, &actor_id, 5, TaskEvent::TaskSucceeded),
    ])
    .unwrap();

    assert_eq!(view.task_id, task_id);
    assert_eq!(view.state, ManagedRunState::ExecutionCompleted);
    assert_eq!(view.completion.execution, ManagedExecutionState::Succeeded);
    assert_eq!(view.completion.verification, VerificationState::NotRecorded);
    assert_eq!(
        view.completion.workspace_disposition,
        WorkspaceDisposition::NotRecorded
    );
    assert_eq!(view.attempts.len(), 1);
    assert_eq!(view.attempts[0].run_id, run_id);
    assert_eq!(view.attempts[0].state, AttemptState::Succeeded);
}

#[test]
fn retry_preserves_task_identity_and_appends_an_attempt() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let first_run = RunId::new();
    let second_run = RunId::new();
    let run_error = failure("runtime_lost");
    let view = ManagedRunView::replay(&[
        submitted(&task_id, &actor_id),
        event(
            &task_id,
            &actor_id,
            2,
            TaskEvent::TaskQueued {
                run_id: first_run.clone(),
                runtime: runtime(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::RunStarted {
                run_id: first_run.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            4,
            TaskEvent::RunFailed {
                run_id: first_run.clone(),
                error: run_error.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            5,
            TaskEvent::RunRetryQueued {
                previous_run_id: first_run.clone(),
                next_run_id: second_run.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            6,
            TaskEvent::RunStarted {
                run_id: second_run.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            7,
            TaskEvent::RunSucceeded {
                run_id: second_run.clone(),
            },
        ),
        event(&task_id, &actor_id, 8, TaskEvent::TaskSucceeded),
    ])
    .unwrap();

    assert_eq!(view.task_id, task_id);
    assert_eq!(view.active_run_id.as_ref(), Some(&second_run));
    assert_eq!(view.attempts.len(), 2);
    assert_eq!(view.attempts[0].attempt, 1);
    assert_eq!(view.attempts[0].run_id, first_run);
    assert_eq!(view.attempts[0].state, AttemptState::Failed);
    assert_eq!(view.attempts[0].failure.as_ref(), Some(&run_error));
    assert_eq!(view.attempts[0].last_revision, 5);
    assert_eq!(view.attempts[0].retry_run_id.as_ref(), Some(&second_run));
    assert_eq!(view.attempts[1].attempt, 2);
    assert_eq!(view.attempts[1].run_id, second_run);
    assert_eq!(view.attempts[1].state, AttemptState::Succeeded);
    assert!(view.attempts[1].retry_run_id.is_none());
    assert_eq!(view.attempts[0].runtime, view.attempts[1].runtime);
}

#[test]
fn uncertain_side_effect_suspends_without_inventing_a_result() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let run_id = RunId::new();
    let execution_id = ExecutionId::new();
    let view = ManagedRunView::replay(&[
        submitted(&task_id, &actor_id),
        event(
            &task_id,
            &actor_id,
            2,
            TaskEvent::TaskQueued {
                run_id: run_id.clone(),
                runtime: runtime(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::RunStarted {
                run_id: run_id.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            4,
            TaskEvent::ExecutionPlanned {
                execution_id: execution_id.clone(),
                permit_id: PermitId::new(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            5,
            TaskEvent::ExecutionUncertain {
                execution_id,
                reason: UncertaintyCode::TransportLost,
            },
        ),
    ])
    .unwrap();

    assert_eq!(view.state, ManagedRunState::Suspended);
    assert_eq!(view.completion.execution, ManagedExecutionState::Uncertain);
    assert_eq!(view.attempts[0].state, AttemptState::Uncertain);
    assert_eq!(view.attempts[0].executions.planned, 1);
    assert_eq!(view.attempts[0].executions.uncertain, 1);
    assert_eq!(
        view.attempts[0].uncertainty_reason,
        Some(UncertaintyCode::TransportLost)
    );
}

#[test]
fn incremental_projection_is_unchanged_after_invalid_event() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let mut projector = ManagedRunProjector::new();
    projector.apply(&submitted(&task_id, &actor_id)).unwrap();
    let before = projector.view().cloned();

    let error = projector
        .apply(&event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::TaskQueued {
                run_id: RunId::new(),
                runtime: runtime(),
            },
        ))
        .unwrap_err();

    assert!(matches!(
        error,
        ManagedRunProjectionError::Aggregate(AggregateError::RevisionGap {
            expected: 2,
            actual: 3
        })
    ));
    assert_eq!(projector.view(), before.as_ref());
}

#[test]
fn cancellation_before_runtime_keeps_the_attempt_and_stage() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let run_id = RunId::new();
    let view = ManagedRunView::replay(&[
        submitted(&task_id, &actor_id),
        event(
            &task_id,
            &actor_id,
            2,
            TaskEvent::TaskQueued {
                run_id: run_id.clone(),
                runtime: runtime(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::CancellationRequested {
                run_id: run_id.clone(),
                cause: cosh_gateway_contracts::task::CancelReason::UserRequested,
            },
        ),
        event(
            &task_id,
            &actor_id,
            4,
            TaskEvent::RunCancelled {
                run_id: run_id.clone(),
                stage: CancellationStage::BeforeRuntime,
            },
        ),
        event(&task_id, &actor_id, 5, TaskEvent::TaskCancelled),
    ])
    .unwrap();

    assert_eq!(view.state, ManagedRunState::Cancelled);
    assert_eq!(view.completion.execution, ManagedExecutionState::Cancelled);
    assert_eq!(view.attempts[0].state, AttemptState::Cancelled);
    assert!(view.attempts[0].cancellation_requested);
    assert_eq!(
        view.attempts[0].cancellation_stage,
        Some(CancellationStage::BeforeRuntime)
    );
}

#[test]
fn suspension_and_known_execution_results_remain_distinct() {
    let task_id = TaskId::new();
    let actor_id = ActorId::new();
    let run_id = RunId::new();
    let succeeded_execution = ExecutionId::new();
    let failed_execution = ExecutionId::new();
    let view = ManagedRunView::replay(&[
        submitted(&task_id, &actor_id),
        event(
            &task_id,
            &actor_id,
            2,
            TaskEvent::TaskQueued {
                run_id: run_id.clone(),
                runtime: runtime(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            3,
            TaskEvent::RunStarted {
                run_id: run_id.clone(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            4,
            TaskEvent::ExecutionPlanned {
                execution_id: succeeded_execution.clone(),
                permit_id: PermitId::new(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            5,
            TaskEvent::ExecutionResultRecorded {
                execution_id: succeeded_execution,
                outcome: ExecutionOutcome::Succeeded { evidence_ref: None },
            },
        ),
        event(
            &task_id,
            &actor_id,
            6,
            TaskEvent::ExecutionPlanned {
                execution_id: failed_execution.clone(),
                permit_id: PermitId::new(),
            },
        ),
        event(
            &task_id,
            &actor_id,
            7,
            TaskEvent::ExecutionResultRecorded {
                execution_id: failed_execution,
                outcome: ExecutionOutcome::Failed {
                    error: failure("operation_failed"),
                },
            },
        ),
        event(
            &task_id,
            &actor_id,
            8,
            TaskEvent::RunSuspended {
                run_id,
                reason: SuspensionCode::OperatorRequired,
            },
        ),
    ])
    .unwrap();

    assert_eq!(view.state, ManagedRunState::Suspended);
    assert_eq!(view.completion.execution, ManagedExecutionState::Suspended);
    assert_eq!(
        view.attempts[0].suspension_reason,
        Some(SuspensionCode::OperatorRequired)
    );
    assert_eq!(view.attempts[0].executions.planned, 2);
    assert_eq!(view.attempts[0].executions.succeeded, 1);
    assert_eq!(view.attempts[0].executions.failed, 1);
    assert_eq!(view.attempts[0].executions.uncertain, 0);
}
