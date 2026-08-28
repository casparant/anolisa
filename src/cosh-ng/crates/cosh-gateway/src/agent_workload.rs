//! Agent Workload projections over the durable Task event ledger.
//!
//! A Task supplies the stable host identity and each Run is one execution
//! attempt. Provider-owned sessions are correlated through Runtime bindings;
//! they never replace the host identity or become a second lifecycle owner.

use cosh_gateway_contracts::common::{RuntimeBindingRef, RuntimeSelector, TargetRef};
use cosh_gateway_contracts::error::ContractError;
use cosh_gateway_contracts::ids::{RunId, TaskId};
use cosh_gateway_contracts::task::{
    CancellationStage, ExecutionOutcome, SuspensionCode, TaskEvent, TaskEventEnvelope, TaskState,
    UncertaintyCode,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::task::{AggregateError, TaskAggregate};

/// User-facing lifecycle of one durable Agent Workload.
///
/// `ExecutionCompleted` is intentionally not named `Succeeded`: the Runtime
/// finished, while independent verification and workspace disposition may
/// still be absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentWorkloadState {
    /// The intent is durable but no execution attempt exists.
    Accepted,
    /// An execution attempt is waiting for its Runtime.
    Queued,
    /// The current attempt is executing.
    Running,
    /// The current attempt is waiting for an approval decision.
    WaitingApproval,
    /// The current attempt is waiting for additional user input.
    WaitingInput,
    /// Progress stopped and requires reconciliation, retry, or operator action.
    Suspended,
    /// Runtime execution finished without establishing goal completion.
    ExecutionCompleted,
    /// The durable Task closed with a failure.
    Failed,
    /// The durable Task closed through cancellation.
    Cancelled,
}

/// Current execution state, independent of verification and disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkloadExecutionState {
    /// No attempt has been allocated.
    NotStarted,
    /// The current attempt is queued.
    Queued,
    /// The current attempt is active.
    Running,
    /// Execution is paused for approval.
    WaitingApproval,
    /// Execution is paused for user input.
    WaitingInput,
    /// Execution stopped with a recoverable or externally resolvable cause.
    Suspended,
    /// The Runtime reported successful execution.
    Succeeded,
    /// Execution ended with a known failure.
    Failed,
    /// Execution ended through cancellation.
    Cancelled,
    /// A side effect may have happened but its result is not proven.
    Uncertain,
}

/// Goal-verification evidence recorded after execution.
///
/// The current event contract has no verifier fact, so the projection reports
/// this state explicitly instead of treating Runtime success as verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VerificationState {
    /// No independent verification fact exists in the Task ledger.
    NotRecorded,
}

/// Durable disposition of changes produced in a governed workspace.
///
/// Checkpoint retention, commit, and rollback require explicit evidence. The
/// current Task event contract records none of those outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkspaceDisposition {
    /// No workspace disposition fact exists in the Task ledger.
    NotRecorded,
}

/// Three independent completion axes for one Agent Workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadCompletion {
    /// What happened while the Agent Runtime executed.
    pub execution: WorkloadExecutionState,
    /// Whether an independent verifier established the requested outcome.
    pub verification: VerificationState,
    /// What happened to mutations in the governed workspace.
    pub workspace_disposition: WorkspaceDisposition,
}

/// Current lifecycle state of one execution attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AttemptState {
    /// The attempt is queued.
    Queued,
    /// The attempt is executing.
    Running,
    /// The attempt is waiting for approval.
    WaitingApproval,
    /// The attempt is waiting for user input.
    WaitingInput,
    /// The attempt stopped without closing the durable Task.
    Suspended,
    /// The Runtime reported success for this attempt.
    Succeeded,
    /// The attempt ended with a known failure.
    Failed,
    /// The attempt ended through cancellation.
    Cancelled,
    /// A governed side effect has no proven result.
    Uncertain,
}

/// Counts of governed side-effect executions observed in one attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedExecutionSummary {
    /// Executions admitted with a durable plan.
    pub planned: u64,
    /// Planned executions with a known successful result.
    pub succeeded: u64,
    /// Planned executions with a known failed result.
    pub failed: u64,
    /// Planned executions whose result could not be proven.
    pub uncertain: u64,
}

/// Projection of one Run attempt under a stable Task identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadAttempt {
    /// One-based attempt number in durable event order.
    pub attempt: u64,
    /// Identity of this execution attempt.
    pub run_id: RunId,
    /// Runtime selector inherited by retries from the preceding attempt.
    pub runtime: RuntimeSelector,
    /// Latest fenced binding to a COSH or provider-owned Agent session.
    ///
    /// The external session is correlation metadata. Its provider retains
    /// ownership of the harness and session lifecycle.
    pub runtime_binding: Option<RuntimeBindingRef>,
    /// Current attempt state.
    pub state: AttemptState,
    /// Revision that allocated this attempt.
    pub queued_revision: u64,
    /// Revision that confirmed Runtime start, when it happened.
    pub started_revision: Option<u64>,
    /// Latest Task revision relevant to this attempt.
    pub last_revision: u64,
    /// Whether cancellation has durably won admission for this attempt.
    pub cancellation_requested: bool,
    /// Recoverable suspension cause, when recorded.
    pub suspension_reason: Option<SuspensionCode>,
    /// Cancellation stage, when cancellation completed.
    pub cancellation_stage: Option<CancellationStage>,
    /// Uncertainty cause, when a side effect could not be reconciled.
    pub uncertainty_reason: Option<UncertaintyCode>,
    /// Bounded Runtime failure, when this attempt failed.
    pub failure: Option<ContractError>,
    /// Replacement attempt allocated by an explicit retry, when present.
    pub retry_run_id: Option<RunId>,
    /// Governed side-effect accounting derived from execution events.
    pub executions: GovernedExecutionSummary,
}

/// Stable Agent Workload view reduced from immutable Task events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkloadView {
    /// Stable COSH host identity for the admitted Agent Workload.
    ///
    /// Provider sessions are recorded on attempts and do not replace this ID.
    pub task_id: TaskId,
    /// Latest durable Task event revision included in this projection.
    pub revision: u64,
    /// User-facing lifecycle that does not overclaim goal completion.
    pub state: AgentWorkloadState,
    /// Current attempt, when one has been allocated.
    pub active_run_id: Option<RunId>,
    /// Immutable environment governed by the Agent Workload.
    pub target: TargetRef,
    /// Independent execution, verification, and workspace outcomes.
    pub completion: WorkloadCompletion,
    /// Attempts in allocation order; retries append instead of changing identity.
    pub attempts: Vec<WorkloadAttempt>,
    /// Bounded Task-level failure, when the durable Task closed as failed.
    pub failure: Option<ContractError>,
}

impl AgentWorkloadView {
    /// Replays a complete ordered Task event history into an Agent Workload view.
    ///
    /// # Errors
    ///
    /// Returns the first Task invariant or Agent Workload projection violation.
    pub fn replay(events: &[TaskEventEnvelope]) -> Result<Self, AgentWorkloadProjectionError> {
        let mut projector = AgentWorkloadProjector::new();
        for event in events {
            projector.apply(event)?;
        }
        projector.finish()
    }

    fn from_aggregate(aggregate: &TaskAggregate) -> Self {
        Self {
            task_id: aggregate.task_id().clone(),
            revision: aggregate.revision(),
            state: AgentWorkloadState::Accepted,
            active_run_id: None,
            target: aggregate.target().clone(),
            completion: WorkloadCompletion {
                execution: WorkloadExecutionState::NotStarted,
                verification: VerificationState::NotRecorded,
                workspace_disposition: WorkspaceDisposition::NotRecorded,
            },
            attempts: Vec::new(),
            failure: None,
        }
    }

    fn reduce(
        &mut self,
        envelope: &TaskEventEnvelope,
        aggregate: &TaskAggregate,
    ) -> Result<(), AgentWorkloadProjectionError> {
        match &envelope.event {
            TaskEvent::TaskSubmitted { .. } => {
                return Err(AgentWorkloadProjectionError::DuplicateSubmission)
            }
            TaskEvent::TaskQueued { run_id, runtime } => {
                self.push_attempt(run_id.clone(), runtime.clone(), envelope.revision)?;
            }
            TaskEvent::RunStarted { run_id } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Running;
                attempt.started_revision = Some(envelope.revision);
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RuntimeBound { run_id, binding } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.runtime_binding = Some(binding.clone());
                attempt.state = AttemptState::Running;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RuntimeEventRecorded { run_id, .. }
            | TaskEvent::InputSubmitted { run_id, .. } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Running;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::InputRequested { request } => {
                let attempt = self.attempt_mut(request.run_id())?;
                attempt.state = AttemptState::WaitingInput;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::ApprovalRequested { approval } => {
                let attempt = self.attempt_mut(&approval.run_id)?;
                attempt.state = AttemptState::WaitingApproval;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::ApprovalResolved { .. } => {
                let attempt = self.active_attempt_mut()?;
                attempt.state = match aggregate.state() {
                    TaskState::WaitingApproval => AttemptState::WaitingApproval,
                    _ => AttemptState::Running,
                };
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::ExecutionPlanned { .. } => {
                let attempt = self.active_attempt_mut()?;
                increment_execution_count(&mut attempt.executions.planned)?;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::ExecutionResultRecorded { outcome, .. } => {
                let attempt = self.active_attempt_mut()?;
                match outcome {
                    ExecutionOutcome::Succeeded { .. } => {
                        increment_execution_count(&mut attempt.executions.succeeded)?;
                    }
                    ExecutionOutcome::Failed { .. } => {
                        increment_execution_count(&mut attempt.executions.failed)?;
                    }
                }
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::ExecutionUncertain { reason, .. } => {
                let attempt = self.active_attempt_mut()?;
                increment_execution_count(&mut attempt.executions.uncertain)?;
                attempt.state = AttemptState::Uncertain;
                attempt.uncertainty_reason = Some(*reason);
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::CancellationRequested { run_id, .. } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.cancellation_requested = true;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RunCancelled { run_id, stage } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Cancelled;
                attempt.cancellation_stage = Some(*stage);
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RunSuspended { run_id, reason } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Suspended;
                attempt.suspension_reason = Some(*reason);
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RunSucceeded { run_id } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Succeeded;
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RunFailed { run_id, error } => {
                let attempt = self.attempt_mut(run_id)?;
                attempt.state = AttemptState::Failed;
                attempt.failure = Some(error.clone());
                attempt.last_revision = envelope.revision;
            }
            TaskEvent::RunRetryQueued {
                previous_run_id,
                next_run_id,
            } => {
                let previous = self.attempt_mut(previous_run_id)?;
                previous.retry_run_id = Some(next_run_id.clone());
                previous.last_revision = envelope.revision;
                let runtime = previous.runtime.clone();
                self.push_attempt(next_run_id.clone(), runtime, envelope.revision)?;
            }
            TaskEvent::TaskSucceeded => {
                self.touch_active(envelope.revision)?;
            }
            TaskEvent::TaskFailed { error } => {
                self.failure = Some(error.clone());
                self.touch_active(envelope.revision)?;
            }
            TaskEvent::TaskCancelled => {
                if let Some(run_id) = self.active_run_id.clone() {
                    let attempt = self.attempt_mut(&run_id)?;
                    attempt.state = AttemptState::Cancelled;
                    attempt.last_revision = envelope.revision;
                }
            }
        }

        self.revision = aggregate.revision();
        self.active_run_id = aggregate.active_run_id().cloned();
        self.state = workload_state(aggregate.state(), self.active_attempt());
        self.completion.execution = execution_state(aggregate.state(), self.active_attempt());
        Ok(())
    }

    fn push_attempt(
        &mut self,
        run_id: RunId,
        runtime: RuntimeSelector,
        revision: u64,
    ) -> Result<(), AgentWorkloadProjectionError> {
        if self.attempts.iter().any(|attempt| attempt.run_id == run_id) {
            return Err(AgentWorkloadProjectionError::DuplicateAttempt { run_id });
        }
        let attempt = u64::try_from(self.attempts.len())
            .map_err(|_| AgentWorkloadProjectionError::AttemptNumberOverflow)?
            .checked_add(1)
            .ok_or(AgentWorkloadProjectionError::AttemptNumberOverflow)?;
        self.attempts.push(WorkloadAttempt {
            attempt,
            run_id,
            runtime,
            runtime_binding: None,
            state: AttemptState::Queued,
            queued_revision: revision,
            started_revision: None,
            last_revision: revision,
            cancellation_requested: false,
            suspension_reason: None,
            cancellation_stage: None,
            uncertainty_reason: None,
            failure: None,
            retry_run_id: None,
            executions: GovernedExecutionSummary::default(),
        });
        Ok(())
    }

    fn attempt_mut(
        &mut self,
        run_id: &RunId,
    ) -> Result<&mut WorkloadAttempt, AgentWorkloadProjectionError> {
        self.attempts
            .iter_mut()
            .find(|attempt| &attempt.run_id == run_id)
            .ok_or_else(|| AgentWorkloadProjectionError::AttemptNotFound {
                run_id: run_id.clone(),
            })
    }

    fn active_attempt(&self) -> Option<&WorkloadAttempt> {
        self.active_run_id.as_ref().and_then(|run_id| {
            self.attempts
                .iter()
                .find(|attempt| &attempt.run_id == run_id)
        })
    }

    fn active_attempt_mut(&mut self) -> Result<&mut WorkloadAttempt, AgentWorkloadProjectionError> {
        let run_id = self
            .active_run_id
            .clone()
            .ok_or(AgentWorkloadProjectionError::ActiveAttemptMissing)?;
        self.attempt_mut(&run_id)
    }

    fn touch_active(&mut self, revision: u64) -> Result<(), AgentWorkloadProjectionError> {
        if self.active_run_id.is_some() {
            self.active_attempt_mut()?.last_revision = revision;
        }
        Ok(())
    }
}

/// Incremental, transactional reducer for paged Task event streams.
#[derive(Debug, Clone)]
pub struct AgentWorkloadProjector {
    aggregate: Option<TaskAggregate>,
    view: Option<AgentWorkloadView>,
}

impl AgentWorkloadProjector {
    /// Creates an empty projector ready for the first Task event.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            aggregate: None,
            view: None,
        }
    }

    /// Applies one consecutive event without modifying the projector on error.
    ///
    /// # Errors
    ///
    /// Returns a Task invariant error or an Agent Workload projection violation.
    pub fn apply(
        &mut self,
        envelope: &TaskEventEnvelope,
    ) -> Result<(), AgentWorkloadProjectionError> {
        match (&self.aggregate, &self.view) {
            (None, None) => {
                let aggregate = TaskAggregate::replay(std::slice::from_ref(envelope))?;
                let view = AgentWorkloadView::from_aggregate(&aggregate);
                self.aggregate = Some(aggregate);
                self.view = Some(view);
                Ok(())
            }
            (Some(aggregate), Some(view)) => {
                let mut next_aggregate = aggregate.clone();
                next_aggregate.apply(envelope)?;
                let mut next_view = view.clone();
                next_view.reduce(envelope, &next_aggregate)?;
                self.aggregate = Some(next_aggregate);
                self.view = Some(next_view);
                Ok(())
            }
            _ => Err(AgentWorkloadProjectionError::ProjectorInvariant),
        }
    }

    /// Returns the current projection, if at least one event was applied.
    #[must_use]
    pub fn view(&self) -> Option<&AgentWorkloadView> {
        self.view.as_ref()
    }

    /// Finishes projection and returns the durable Agent Workload view.
    ///
    /// # Errors
    ///
    /// Returns [`AgentWorkloadProjectionError::EmptyHistory`] when no event was
    /// applied.
    pub fn finish(self) -> Result<AgentWorkloadView, AgentWorkloadProjectionError> {
        self.view.ok_or(AgentWorkloadProjectionError::EmptyHistory)
    }
}

impl Default for AgentWorkloadProjector {
    fn default() -> Self {
        Self::new()
    }
}

/// Failure while reducing a Task ledger into Agent Workload semantics.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AgentWorkloadProjectionError {
    /// Task event history is absent.
    #[error("Agent Workload projection requires at least one Task event")]
    EmptyHistory,
    /// The underlying Task ledger violates its durable invariants.
    #[error(transparent)]
    Aggregate(#[from] AggregateError),
    /// Submission appeared after the first event.
    #[error("Task submission may appear only as the first event")]
    DuplicateSubmission,
    /// One Run identity was allocated more than once.
    #[error("Run {run_id} was allocated more than once")]
    DuplicateAttempt {
        /// Duplicated Run identity.
        run_id: RunId,
    },
    /// An event referenced a Run absent from the projected attempt list.
    #[error("Run {run_id} is absent from the Agent Workload projection")]
    AttemptNotFound {
        /// Missing Run identity.
        run_id: RunId,
    },
    /// An event requires an active attempt but none is allocated.
    #[error("Agent Workload has no active attempt")]
    ActiveAttemptMissing,
    /// Attempt ordinal cannot be represented safely.
    #[error("Agent Workload attempt number overflowed")]
    AttemptNumberOverflow,
    /// Governed execution accounting cannot be represented safely.
    #[error("Agent Workload execution count overflowed")]
    ExecutionCountOverflow,
    /// Internal projector halves became inconsistent.
    #[error("Agent Workload projector invariant failed")]
    ProjectorInvariant,
}

fn workload_state(state: TaskState, active: Option<&WorkloadAttempt>) -> AgentWorkloadState {
    match state {
        TaskState::Submitted => AgentWorkloadState::Accepted,
        TaskState::Queued => AgentWorkloadState::Queued,
        TaskState::Running
            if active.is_some_and(|attempt| attempt.state == AttemptState::Succeeded) =>
        {
            AgentWorkloadState::ExecutionCompleted
        }
        TaskState::Running => AgentWorkloadState::Running,
        TaskState::WaitingApproval => AgentWorkloadState::WaitingApproval,
        TaskState::WaitingInput => AgentWorkloadState::WaitingInput,
        TaskState::Suspended => AgentWorkloadState::Suspended,
        TaskState::Succeeded => AgentWorkloadState::ExecutionCompleted,
        TaskState::Failed => AgentWorkloadState::Failed,
        TaskState::Cancelled => AgentWorkloadState::Cancelled,
    }
}

fn execution_state(state: TaskState, active: Option<&WorkloadAttempt>) -> WorkloadExecutionState {
    let Some(attempt) = active else {
        return if state == TaskState::Cancelled {
            WorkloadExecutionState::Cancelled
        } else {
            WorkloadExecutionState::NotStarted
        };
    };
    match attempt.state {
        AttemptState::Queued => WorkloadExecutionState::Queued,
        AttemptState::Running => WorkloadExecutionState::Running,
        AttemptState::WaitingApproval => WorkloadExecutionState::WaitingApproval,
        AttemptState::WaitingInput => WorkloadExecutionState::WaitingInput,
        AttemptState::Suspended => WorkloadExecutionState::Suspended,
        AttemptState::Succeeded => WorkloadExecutionState::Succeeded,
        AttemptState::Failed => WorkloadExecutionState::Failed,
        AttemptState::Cancelled => WorkloadExecutionState::Cancelled,
        AttemptState::Uncertain => WorkloadExecutionState::Uncertain,
    }
}

fn increment_execution_count(value: &mut u64) -> Result<(), AgentWorkloadProjectionError> {
    *value = value
        .checked_add(1)
        .ok_or(AgentWorkloadProjectionError::ExecutionCountOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests;
