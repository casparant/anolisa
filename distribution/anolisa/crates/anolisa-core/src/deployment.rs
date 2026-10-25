//! Explicit service deployment plans and a bounded foreground supervisor.
//!
//! File installation never calls these APIs. Callers separately select a preset,
//! and systemd activation remains distinct from container process supervision.

mod model;
mod preset;
mod supervisor;
#[cfg(test)]
mod tests;

pub use model::{
    Communication, DeploymentConfig, DeploymentError, Preset, PresetService, Readiness, RunAs,
    Scope, Service,
};
pub(crate) use preset::bounded_command;
pub use preset::{
    PresetAction, PresetActionKind, PresetPlan, PresetReport, Systemctl, SystemctlRunner,
    UnitState, apply_preset, plan_preset,
};
pub use supervisor::{ServePlan, ServeReport, plan_serve, serve};
