//! Managed Run CLI projection over paged daemon Task events.

use cosh_gateway::managed_run::ManagedRunProjector;

use super::*;

#[derive(Debug, Clone, Args)]
pub(super) struct ManagedRunArgs {
    /// Absolute Unix socket path; defaults below the user runtime directory.
    #[arg(long, value_name = "PATH")]
    pub(super) socket: Option<PathBuf>,
    /// Presentation format for Managed Run responses.
    #[arg(long, value_enum, default_value_t = Output::Human)]
    pub(super) output: Output,
    #[command(subcommand)]
    pub(super) command: ManagedRunCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub(super) enum ManagedRunCommand {
    /// Admit an intent and queue its first execution attempt.
    Start(TaskSubmitArgs),
    /// Project the complete durable Task ledger without overclaiming success.
    Inspect(TaskIdArgs),
    /// Answer the exact question blocking the active attempt.
    Answer(TaskAppendArgs),
    /// Resolve a pending Runtime or brokered approval.
    ResolveApproval(TaskResolveApprovalArgs),
    /// Request cancellation of the active attempt.
    Cancel(TaskCancelArgs),
    /// Queue a replacement for one exact suspended attempt.
    Retry(TaskRetryArgs),
}

pub(super) fn managed_run(args: ManagedRunArgs, reporter: &Reporter) -> Result<u8, CliError> {
    let ManagedRunArgs {
        socket,
        output,
        command,
    } = args;
    match command {
        ManagedRunCommand::Start(command) => task(
            TaskArgs {
                socket,
                output,
                command: TaskCommand::Submit(command),
            },
            reporter,
        ),
        ManagedRunCommand::Inspect(command) => inspect(socket, &command.task_id, reporter),
        ManagedRunCommand::Answer(command) => task(
            TaskArgs {
                socket,
                output,
                command: TaskCommand::Append(command),
            },
            reporter,
        ),
        ManagedRunCommand::ResolveApproval(command) => task(
            TaskArgs {
                socket,
                output,
                command: TaskCommand::ResolveApproval(command),
            },
            reporter,
        ),
        ManagedRunCommand::Cancel(command) => task(
            TaskArgs {
                socket,
                output,
                command: TaskCommand::Cancel(command),
            },
            reporter,
        ),
        ManagedRunCommand::Retry(command) => task(
            TaskArgs {
                socket,
                output,
                command: TaskCommand::Retry(command),
            },
            reporter,
        ),
    }
}

fn inspect(socket: Option<PathBuf>, task_id: &str, reporter: &Reporter) -> Result<u8, CliError> {
    let task_id = parse_managed_task(task_id)?;
    let socket = daemon_socket_path(socket.as_ref())?;
    let client = LocalGatewayClient::new(socket);
    let target_revision = match client
        .get(RequestId::new(), task_id.clone())
        .map_err(|error| CliError::Daemon(error.to_string()))?
    {
        GatewayResult::Task(task) if task.task_id == task_id => task.revision,
        GatewayResult::Task(_) => {
            return Err(CliError::ManagedRun(
                "Task projection identity does not match the request".to_owned(),
            ))
        }
        _ => {
            return Err(CliError::ManagedRun(
                "daemon returned a non-Task projection".to_owned(),
            ))
        }
    };
    if target_revision == 0 {
        return Err(CliError::ManagedRun(
            "Task projection has no durable event revision".to_owned(),
        ));
    }
    let mut projector = ManagedRunProjector::new();
    let mut after_revision = None;

    loop {
        let result = client
            .events(RequestId::new(), task_id.clone(), after_revision, 64)
            .map_err(|error| CliError::Daemon(error.to_string()))?;
        let GatewayResult::Events(page) = result else {
            return Err(CliError::ManagedRun(
                "daemon returned a non-event response".to_owned(),
            ));
        };
        if page.task_id != task_id {
            return Err(CliError::ManagedRun(
                "event page Task identity does not match the request".to_owned(),
            ));
        }
        let previous = after_revision.unwrap_or(0);
        if page.events.is_empty() {
            return Err(CliError::ManagedRun(
                "daemon returned an empty page before the inspected revision".to_owned(),
            ));
        }
        if let Some(last) = page.events.last() {
            if page.next_revision != last.revision {
                return Err(CliError::ManagedRun(
                    "event page cursor does not match its final revision".to_owned(),
                ));
            }
        }
        for event in &page.events {
            projector
                .apply(event)
                .map_err(|error| CliError::ManagedRun(error.to_string()))?;
        }
        if page.next_revision >= target_revision {
            break;
        }
        if !page.has_more {
            return Err(CliError::ManagedRun(
                "event history ended before the inspected Task revision".to_owned(),
            ));
        }
        if page.next_revision <= previous {
            return Err(CliError::ManagedRun(
                "event page cursor did not advance".to_owned(),
            ));
        }
        after_revision = Some(page.next_revision);
    }

    let view = projector
        .finish()
        .map_err(|error| CliError::ManagedRun(error.to_string()))?;
    reporter.event(
        "managed_run",
        serde_json::to_value(view).map_err(|error| CliError::ManagedRun(error.to_string()))?,
    )?;
    Ok(0)
}

pub(super) fn parse_managed_task(value: &str) -> Result<TaskId, CliError> {
    TaskId::parse(value).map_err(|error| CliError::InvalidInput(error.to_string()))
}
