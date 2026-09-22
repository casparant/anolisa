//! Framework-wide adapter batches (`adapter enable --all --framework <fw>`).
//!
//! A batch is a sequence of independent single-component operations: every
//! member keeps its own driver call, its own receipt, and its own failure.
//! Targets come from the same read-only `scan` the single-component surface
//! uses, so a batch never enables an adapter that `adapter enable <component>`
//! would refuse, and never disables a receipt that is not there.
//!
//! ## Empty selection
//!
//! The two verbs differ deliberately. An enable batch with no actionable target
//! means the request cannot be satisfied — a misspelled framework, a component
//! that is not installed, or a framework absent from this host — so it is an
//! argument error. A disable batch with no receipt is already in its desired
//! end state, mirroring the idempotent single-component disable.

use serde::Serialize;

use anolisa_core::adapter::manager::{EnableOptions, ScanEntry, ScanReport};
use anolisa_core::execution::CommandOutcomeStatus;
use anolisa_core::manifest::AdapterNotice;

use crate::color::Palette;
use crate::commands::common;
use crate::context::CliContext;
use crate::response::{CliError, render_json_with_status};

use super::application::{
    AdapterApplicationOutcome, AdapterApplied, AdapterPreview, AdapterRequest, run,
};
use super::{NoticeRow, execution_intent, map_err, print_notices};

/// Terminal state of one batch member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberStatus {
    /// The adapter was enabled and its receipt persisted.
    Enabled,
    /// The receipt was removed after cleanup completed.
    Disabled,
    /// Dry-run member: a plan was produced and nothing was applied.
    Planned,
    /// Disable found no receipt to remove.
    Noop,
    /// Cleanup did not complete; the receipt was kept for retry.
    CleanupFailed,
    /// The member's operation failed.
    Failed,
}

impl MemberStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Planned => "planned",
            Self::Noop => "noop",
            Self::CleanupFailed => "cleanup_failed",
            Self::Failed => "failed",
        }
    }

    /// Whether the member counts toward the batch's success total. An
    /// incomplete cleanup is not a success: framework state and receipt
    /// disagree until a retry finishes the job.
    fn is_success(self) -> bool {
        !matches!(self, Self::CleanupFailed | Self::Failed)
    }
}

/// One batch member, holding the declared notices until rendering so human
/// output can escape them and `--json` can project the stable wire shape.
struct BatchMember {
    component: String,
    status: MemberStatus,
    reason: Option<String>,
    plan: Option<Vec<String>>,
    notices: Vec<AdapterNotice>,
}

/// Why an enable batch has no target it may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectionGap {
    /// No installed component declares this framework's adapter.
    NotDeclared,
    /// Declarations exist but no built-in driver can act on them.
    NoDriver,
    /// Declarations and a driver exist, but the framework is absent here.
    NotDetected,
}

impl SelectionGap {
    fn reason(&self, framework: &str) -> String {
        match self {
            Self::NotDeclared => {
                format!("no installed component declares a '{framework}' adapter")
            }
            Self::NoDriver => format!("no built-in driver for framework '{framework}'"),
            Self::NotDetected => format!("framework '{framework}' not detected on this host"),
        }
    }
}

/// One row of a batch's `--json` output.
#[derive(Serialize)]
struct BatchMemberRow {
    component: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Plan actions, present only for dry-run members.
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<Vec<String>>,
    /// Always present so consumers can rely on the field layout.
    notices: Vec<NoticeRow>,
}

/// A batch's `--json` output.
#[derive(Serialize)]
struct BatchPayload {
    framework: String,
    dry_run: bool,
    total: usize,
    succeeded: usize,
    failed: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    items: Vec<BatchMemberRow>,
}

/// Components whose declared `framework` adapter an enable batch may act on.
///
/// Only entries that are manifest-declared, driver-backed, and detected on
/// this host qualify; the rest would fail inside the driver anyway, and
/// reporting the gap once is more useful than N identical member failures.
fn select_enable_targets(
    report: &ScanReport,
    framework: &str,
) -> Result<Vec<String>, SelectionGap> {
    let declared: Vec<&ScanEntry> = report
        .entries
        .iter()
        .filter(|entry| entry.framework == framework && entry.declared)
        .collect();
    if declared.is_empty() {
        return Err(SelectionGap::NotDeclared);
    }
    let targets: Vec<String> = declared
        .iter()
        .filter(|entry| entry.driver_available && entry.framework_detected)
        .map(|entry| entry.component.clone())
        .collect();
    if !targets.is_empty() {
        return Ok(targets);
    }
    if declared.iter().any(|entry| entry.driver_available) {
        Err(SelectionGap::NotDetected)
    } else {
        Err(SelectionGap::NoDriver)
    }
}

/// Components holding a live `framework` receipt a disable batch may remove.
///
/// Orphaned receipts — whose source component or bundle disappeared — are
/// included on purpose: disable works from the receipt alone, and skipping
/// them would strand framework-side state with no way to reach it.
fn select_disable_targets(report: &ScanReport, framework: &str) -> Vec<String> {
    report
        .entries
        .iter()
        .filter(|entry| entry.framework == framework && entry.enabled)
        .map(|entry| entry.component.clone())
        .collect()
}

/// Entry point for `adapter enable --all --framework <fw>`.
pub(super) fn handle_enable_all(
    ctx: &CliContext,
    framework: &str,
    allow_unsafe_plugin_install: bool,
    profiles: Vec<String>,
) -> Result<(), CliError> {
    const COMMAND: &str = "adapter enable --all";
    let report = scan(ctx, COMMAND)?;
    let targets = match select_enable_targets(&report, framework) {
        Ok(targets) => targets,
        Err(gap) => {
            return Err(CliError::InvalidArgument {
                command: COMMAND.to_string(),
                reason: gap.reason(framework),
            });
        }
    };

    let mut members = Vec::with_capacity(targets.len());
    for component in &targets {
        let request = AdapterRequest::Enable {
            component: component.as_str(),
            framework: Some(framework),
            options: EnableOptions {
                allow_unsafe_plugin_install,
                profiles: profiles.clone(),
            },
            intent: execution_intent(ctx.dry_run),
        };
        match run(request, ctx) {
            Ok(outcome) => members.push(enable_member(outcome)),
            Err(err) => members.push(failed_member(component, err.reason())),
        }
    }
    render_batch(ctx, COMMAND, framework, report.warnings, members)
}

/// Entry point for `adapter disable --all --framework <fw>`.
pub(super) fn handle_disable_all(ctx: &CliContext, framework: &str) -> Result<(), CliError> {
    const COMMAND: &str = "adapter disable --all";
    let report = scan(ctx, COMMAND)?;
    let targets = select_disable_targets(&report, framework);
    if targets.is_empty() {
        if ctx.json {
            return render_batch(ctx, COMMAND, framework, report.warnings, Vec::new());
        }
        if !ctx.quiet {
            let color = Palette::new(ctx.no_color);
            println!(
                "{}",
                color.muted(format!(
                    "no enabled '{framework}' adapter receipts; nothing to disable"
                ))
            );
        }
        return Ok(());
    }

    let mut members = Vec::with_capacity(targets.len());
    for component in &targets {
        let request = AdapterRequest::Disable {
            component: component.as_str(),
            framework: Some(framework),
            intent: execution_intent(ctx.dry_run),
        };
        match run(request, ctx) {
            Ok(outcome) => members.push(disable_member(outcome)),
            Err(err) => members.push(failed_member(component, err.reason())),
        }
    }
    render_batch(ctx, COMMAND, framework, report.warnings, members)
}

fn scan(ctx: &CliContext, command: &str) -> Result<ScanReport, CliError> {
    let manager = common::build_adapter_manager(ctx);
    manager.scan().map_err(|err| map_err(command, err))
}

fn enable_member(outcome: AdapterApplicationOutcome) -> BatchMember {
    match outcome {
        AdapterApplicationOutcome::Preview(AdapterPreview::Enable { plan, notices }) => {
            BatchMember {
                component: plan.component.clone(),
                status: MemberStatus::Planned,
                reason: None,
                plan: Some(plan.actions.clone()),
                notices,
            }
        }
        AdapterApplicationOutcome::Applied {
            result: AdapterApplied::Enable { claim, notices },
            outcome,
        } => {
            let (status, reason) = match outcome.status() {
                CommandOutcomeStatus::Completed => (MemberStatus::Enabled, None),
                CommandOutcomeStatus::Partial { reason }
                | CommandOutcomeStatus::Failed { reason } => {
                    (MemberStatus::Failed, Some(reason.clone()))
                }
            };
            BatchMember {
                component: claim.component.clone(),
                status,
                reason,
                plan: None,
                notices,
            }
        }
        AdapterApplicationOutcome::Preview(AdapterPreview::Disable(_))
        | AdapterApplicationOutcome::Applied {
            result: AdapterApplied::Disable(_),
            ..
        } => unreachable!("an enable request cannot return a disable result"),
    }
}

fn disable_member(outcome: AdapterApplicationOutcome) -> BatchMember {
    match outcome {
        AdapterApplicationOutcome::Preview(AdapterPreview::Disable(result)) => BatchMember {
            component: result.component.clone(),
            status: MemberStatus::Planned,
            reason: None,
            plan: Some(result.report.messages.clone()),
            notices: result.notices.clone(),
        },
        AdapterApplicationOutcome::Applied {
            result: AdapterApplied::Disable(result),
            outcome,
        } => {
            let (status, reason) = match outcome.status() {
                CommandOutcomeStatus::Completed if result.claim_removed => {
                    (MemberStatus::Disabled, None)
                }
                CommandOutcomeStatus::Completed => (MemberStatus::Noop, None),
                CommandOutcomeStatus::Partial { reason } => {
                    (MemberStatus::CleanupFailed, Some(reason.clone()))
                }
                CommandOutcomeStatus::Failed { reason } => {
                    (MemberStatus::Failed, Some(reason.clone()))
                }
            };
            BatchMember {
                component: result.component.clone(),
                status,
                reason,
                plan: None,
                notices: result.notices.clone(),
            }
        }
        AdapterApplicationOutcome::Preview(AdapterPreview::Enable { .. })
        | AdapterApplicationOutcome::Applied {
            result: AdapterApplied::Enable { .. },
            ..
        } => unreachable!("a disable request cannot return an enable result"),
    }
}

fn failed_member(component: &str, reason: String) -> BatchMember {
    BatchMember {
        component: component.to_string(),
        status: MemberStatus::Failed,
        reason: Some(reason),
        plan: None,
        notices: Vec::new(),
    }
}

/// Render the aggregate result and map member failures onto the batch exit
/// status. The handler has already printed every member, so a partial batch
/// only propagates the exit signal.
fn render_batch(
    ctx: &CliContext,
    command: &str,
    framework: &str,
    warnings: Vec<String>,
    members: Vec<BatchMember>,
) -> Result<(), CliError> {
    let total = members.len();
    let failed = members
        .iter()
        .filter(|member| !member.status.is_success())
        .count();

    if ctx.json {
        let items = members
            .iter()
            .map(|member| BatchMemberRow {
                component: member.component.clone(),
                status: member.status.as_str(),
                reason: member.reason.clone(),
                plan: member.plan.clone(),
                notices: member.notices.iter().map(NoticeRow::from).collect(),
            })
            .collect();
        render_json_with_status(
            command,
            failed == 0,
            BatchPayload {
                framework: framework.to_string(),
                dry_run: ctx.dry_run,
                total,
                succeeded: total - failed,
                failed,
                warnings,
                items,
            },
        )?;
        return terminal(command, failed);
    }

    if !ctx.quiet {
        let color = Palette::new(ctx.no_color);
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        for member in &members {
            println!(
                "{} {}/{}",
                color.label(member.status.as_str()),
                member.component,
                framework
            );
            for action in member.plan.iter().flatten() {
                println!("  - {action}");
            }
            if let Some(reason) = &member.reason {
                println!("  {}", color.err(reason));
            }
            print_notices(&member.notices, ctx.dry_run, ctx.quiet);
        }
        let failed_names: Vec<&str> = members
            .iter()
            .filter(|member| !member.status.is_success())
            .map(|member| member.component.as_str())
            .collect();
        if failed_names.is_empty() {
            println!("{} total={total}  ok={total}", color.label("summary:"));
        } else {
            println!(
                "{} total={total}  ok={}  failed={} ({})",
                color.label("summary:"),
                total - failed,
                failed,
                failed_names.join(", ")
            );
        }
    }

    terminal(command, failed)
}

fn terminal(command: &str, failed: usize) -> Result<(), CliError> {
    if failed > 0 {
        return Err(CliError::BatchPartial {
            command: command.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anolisa_core::adapter::claim::ClaimStatus;
    use anolisa_core::adapter::manager::AdapterSourceStatus;

    use super::*;

    fn entry(component: &str, framework: &str, declared: bool, enabled: bool) -> ScanEntry {
        ScanEntry {
            component: component.to_string(),
            framework: framework.to_string(),
            declared,
            resource_root: Some(PathBuf::from(format!("/tmp/{component}/{framework}"))),
            driver_available: true,
            framework_detected: true,
            adapter_type: Some("plugin".to_string()),
            enabled,
            claim_status: enabled.then_some(ClaimStatus::Enabled),
            source_status: enabled.then_some(AdapterSourceStatus::Available),
            source_reason: None,
        }
    }

    fn scan_report(entries: Vec<ScanEntry>) -> ScanReport {
        ScanReport {
            entries,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn enable_selection_keeps_only_declared_detected_components() {
        let report = scan_report(vec![
            entry("tokenless", "dsh", true, false),
            entry("memory", "dsh", false, false),
            entry("sight", "openclaw", true, false),
            entry("sight", "dsh", true, true),
        ]);

        assert_eq!(
            select_enable_targets(&report, "dsh").expect("targets"),
            ["tokenless", "sight"]
        );
    }

    #[test]
    fn enable_selection_reports_the_most_specific_gap() {
        assert_eq!(
            select_enable_targets(&scan_report(Vec::new()), "dsh"),
            Err(SelectionGap::NotDeclared)
        );
        assert_eq!(
            select_enable_targets(
                &scan_report(vec![entry("memory", "dsh", false, false)]),
                "dsh"
            ),
            Err(SelectionGap::NotDeclared)
        );

        let mut no_driver = entry("tokenless", "dsh", true, false);
        no_driver.driver_available = false;
        no_driver.framework_detected = false;
        assert_eq!(
            select_enable_targets(&scan_report(vec![no_driver]), "dsh"),
            Err(SelectionGap::NoDriver)
        );

        let mut not_detected = entry("tokenless", "dsh", true, false);
        not_detected.framework_detected = false;
        assert_eq!(
            select_enable_targets(&scan_report(vec![not_detected.clone()]), "dsh"),
            Err(SelectionGap::NotDetected)
        );
        assert!(
            select_enable_targets(&scan_report(vec![not_detected]), "dsh")
                .expect_err("gap")
                .reason("dsh")
                .contains("not detected")
        );
    }

    #[test]
    fn disable_selection_uses_receipts_and_ignores_declaration_state() {
        // An orphaned receipt (declaration and bundle gone) must still be
        // disable-able, or its framework-side state becomes unreachable.
        let mut orphan = entry("memory", "dsh", false, true);
        orphan.source_status = Some(AdapterSourceStatus::Missing);
        let report = scan_report(vec![
            entry("tokenless", "dsh", true, true),
            entry("sight", "dsh", true, false),
            orphan,
            entry("ckpt", "openclaw", true, true),
        ]);

        assert_eq!(
            select_disable_targets(&report, "dsh"),
            ["tokenless", "memory"]
        );
        assert!(select_disable_targets(&report, "hermes").is_empty());
    }

    #[test]
    fn member_success_excludes_failed_and_incomplete_cleanup() {
        for status in [
            MemberStatus::Enabled,
            MemberStatus::Disabled,
            MemberStatus::Planned,
            MemberStatus::Noop,
        ] {
            assert!(status.is_success(), "{status:?} must count as success");
        }
        for status in [MemberStatus::CleanupFailed, MemberStatus::Failed] {
            assert!(!status.is_success(), "{status:?} must count as failure");
        }
    }

    /// `plan` is dry-run-only wire surface, and `notices` is always present so
    /// consumers never branch on a missing key.
    #[test]
    fn member_row_serializes_plan_only_when_present() {
        let applied = BatchMemberRow {
            component: "tokenless".to_string(),
            status: MemberStatus::Enabled.as_str(),
            reason: None,
            plan: None,
            notices: Vec::new(),
        };
        let json = serde_json::to_value(&applied).expect("serialize");
        assert_eq!(json["status"], "enabled");
        assert_eq!(json["notices"], serde_json::json!([]));
        assert!(json.get("plan").is_none(), "{json}");
        assert!(json.get("reason").is_none(), "{json}");

        let previewed = BatchMemberRow {
            component: "tokenless".to_string(),
            status: MemberStatus::Planned.as_str(),
            reason: None,
            plan: Some(vec!["register dsh plugin".to_string()]),
            notices: Vec::new(),
        };
        let json = serde_json::to_value(&previewed).expect("serialize");
        assert_eq!(json["plan"][0], "register dsh plugin");
    }
}
