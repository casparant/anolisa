//! Conservative summaries of claims, without executing historical migration rules.

use super::{finding, summary, wire, Record, Scope, Source};
use crate::{require, Error, Result};
use std::{collections::BTreeSet, path::PathBuf};

pub(super) fn summarize(bytes: &[u8], remaining: &mut usize) -> Result<Source> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::Invalid("state document is not valid UTF-8".into()))?;
    let state: wire::State = toml::from_str(text)
        .map_err(|_| Error::Invalid("invalid legacy state document structure or TOML".into()))?;
    require(
        (1..=6).contains(&state.schema_version),
        "unsupported legacy state schema version",
    )?;
    require(
        matches!(state.install_mode.as_str(), "user" | "system"),
        "invalid recorded install mode",
    )?;
    require(
        !state.updated_at.is_empty() && !state.anolisa_version.is_empty(),
        "missing state writer metadata",
    )?;
    summary(&state.prefix)?;
    let count = state.objects.len()
        + state.installations.len()
        + state.quarantined.len()
        + state.adapter_claims.len()
        + state.adapter_trust_roots.len()
        + state.backups.len()
        + state.operations.len();
    require(
        count <= *remaining,
        "state inspection exceeds aggregate record limit",
    )?;
    *remaining -= count;
    let old = state.schema_version <= 4;
    require(
        if old {
            state.installations.is_empty() && state.quarantined.is_empty()
        } else {
            state.objects.is_empty()
        },
        "state record layout does not match its schema version",
    )?;
    require(
        state.schema_version == 6 || state.adapter_trust_roots.is_empty(),
        "adapter trust roots require legacy state schema 6",
    )?;
    let mut records = Vec::new();
    for record in state.objects {
        records.push(old_record(record, &state.install_mode)?);
    }
    for record in state.installations {
        records.push(current_record(record, &state.install_mode)?);
    }
    let quarantined_count = state.quarantined.len();
    for quarantined in state.quarantined {
        require(
            !quarantined.reason.is_empty(),
            "quarantined record has no reason",
        )?;
        let mut record = old_record(quarantined.record, &state.install_mode)?;
        record.findings.push(finding(
            "quarantined_record",
            "Record was quarantined by the previous manager; no authority is acquired.",
        ));
        records.push(record);
    }
    let mut identities = BTreeSet::new();
    for record in &mut records {
        if !identities.insert((
            record.kind.clone(),
            record.name.clone(),
            record.recorded_scope.clone(),
        )) {
            record.findings.push(finding(
                "duplicate_record",
                "The same object and scope occur more than once in this source.",
            ));
        }
    }
    Ok(Source {
        path: PathBuf::new(),
        source_format: "legacy_distribution",
        schema_version: state.schema_version,
        file_owner_uid: 0,
        recorded_install_mode: state.install_mode,
        recorded_prefix: state.prefix,
        legacy_object_count: records
            .iter()
            .filter(|record| !matches!(record.kind.as_str(), "component" | "adapter"))
            .count(),
        records,
        adapter_claim_count: state.adapter_claims.len(),
        adapter_trust_root_count: state.adapter_trust_roots.len(),
        quarantined_count,
    })
}

fn base(kind: String, name: String, status: String, scope: Scope) -> Result<Record> {
    for value in [&kind, &name, &status] {
        summary(value)?;
    }
    require(
        !kind.is_empty() && !name.is_empty(),
        "state record has an empty identity",
    )?;
    require(
        matches!(
            status.as_str(),
            "installed" | "partial" | "disabled" | "failed" | "adopted"
        ),
        "invalid recorded lifecycle status",
    )?;
    let mut findings = Vec::new();
    if !matches!(kind.as_str(), "component" | "adapter") {
        findings.push(finding(
            "legacy_object_kind",
            "Legacy or unknown object kind is retained for inspection, not activated.",
        ));
    }
    Ok(Record {
        kind,
        name,
        recorded_status: status,
        recorded_scope: scope,
        recorded_version: None,
        recorded_backend: "unknown".into(),
        recorded_relation: "unknown".into(),
        recorded_package: None,
        file_count: 0,
        service_count: 0,
        findings,
    })
}

fn old_record(old: wire::OldRecord, mode: &str) -> Result<Record> {
    require(
        !old.installed_at.is_empty(),
        "state record has no installation timestamp",
    )?;
    summary(&old.version)?;
    let mut record = base(
        old.kind,
        old.name,
        old.status,
        Scope {
            kind: mode.into(),
            uid: None,
        },
    )?;
    record.recorded_version = Some(old.version);
    record.file_count = old.files.len();
    record.service_count = old.services.len();
    let backend = old.install_backend.as_deref();
    if let Some(value) = backend {
        summary(value)?;
    }
    if let Some(value) = &old.ownership {
        summary(value)?;
    }
    let has_rpm_metadata = old.rpm_metadata.is_some();
    if let Some(metadata) = old.rpm_metadata {
        summary(&metadata.package_name)?;
        if !metadata.package_name.is_empty() {
            record.recorded_package = Some(metadata.package_name);
        }
    }
    let rpm = matches!(backend, Some("rpm" | "yum"))
        || has_rpm_metadata
        || matches!(
            old.ownership.as_deref(),
            Some("rpm_managed" | "rpm_observed")
        );
    let raw = backend == Some("raw") || old.ownership.as_deref() == Some("raw_managed");
    let unsupported = backend.is_some_and(|value| !matches!(value, "raw" | "rpm" | "yum"));
    let unsupported_ownership = old
        .ownership
        .as_deref()
        .is_some_and(|value| !matches!(value, "raw_managed" | "rpm_managed" | "rpm_observed"));
    record.recorded_backend = match backend {
        Some("rpm" | "yum") => "rpm".into(),
        Some("raw") => "raw".into(),
        Some("deb") => "deb".into(),
        Some(_) => "unknown".into(),
        None if rpm && !raw => "rpm".into(),
        None if raw && !rpm => "raw".into(),
        None => "unknown".into(),
    };
    let ambiguous = (raw && rpm)
        || unsupported_ownership
        || (raw && (!old.managed || old.adopted || record.recorded_status == "adopted"));
    if unsupported {
        record.findings.push(finding(
            "unsupported_backend",
            "The recorded installation backend is not supported by this inspector.",
        ));
    }
    if ambiguous {
        record.findings.push(finding("ambiguous_record", "Ownership and backend claims are incomplete or contradictory; authority is not inferred."));
    } else if !unsupported {
        record.recorded_relation = match old.ownership.as_deref() {
            Some("raw_managed") => "owned",
            Some("rpm_managed") => "managed",
            Some("rpm_observed") if old.adopted || record.recorded_status == "adopted" => "adopted",
            Some("rpm_observed") => "observed",
            None if matches!(backend, Some("rpm" | "yum")) && old.adopted => "adopted",
            None if matches!(backend, Some("rpm" | "yum")) && !old.managed => "observed",
            None if matches!(backend, Some("rpm" | "yum")) => "managed",
            None if backend == Some("raw") => "owned",
            _ => "unknown",
        }
        .into();
    }
    if record.recorded_relation == "unknown" && !ambiguous && !unsupported {
        record.findings.push(finding(
            "ambiguous_record",
            "No explicit supported ownership evidence establishes a management relation.",
        ));
    }
    Ok(finish(record, rpm))
}

fn current_record(current: wire::CurrentRecord, mode: &str) -> Result<Record> {
    require(
        !current.installed_at.is_empty(),
        "state record has no installation timestamp",
    )?;
    require(
        current.status != "adopted",
        "adoption is a relation, not a current-schema lifecycle status",
    )?;
    let scope = match current.scope {
        wire::WireScope::System(value) if value == "system" => Scope {
            kind: value,
            uid: None,
        },
        wire::WireScope::User(mut values) if values.len() == 1 && values.contains_key("user") => {
            let user = values
                .remove("user")
                .ok_or_else(|| Error::Invalid("missing user scope".into()))?;
            Scope {
                kind: "user".into(),
                uid: Some(user.uid),
            }
        }
        _ => return Err(Error::Invalid("invalid recorded installation scope".into())),
    };
    let mut record = base(current.kind, current.name, current.status, scope)?;
    if record.recorded_scope.kind != mode {
        record.findings.push(finding(
            "scope_mismatch",
            "Record scope differs from the document's declared installation mode.",
        ));
    }
    let binding = current.binding;
    summary(&binding.authority)?;
    let mut rpm = false;
    match binding.authority.as_str() {
        "owned" => {
            let artifact = binding
                .artifact
                .ok_or_else(|| Error::Invalid("owned record has no artifact".into()))?;
            summary(&artifact.version)?;
            record.recorded_version = Some(artifact.version);
            record.recorded_backend = "raw".into();
            record.recorded_relation = "owned".into();
            record.file_count = artifact.files.len();
            record.service_count = artifact.services.len();
            if binding.pm.is_some()
                || binding.package.is_some()
                || binding.relation.is_some()
                || binding.last_observed.is_some()
            {
                record.recorded_relation = "unknown".into();
                record.findings.push(finding(
                    "ambiguous_record",
                    "Owned authority also carries delegated-package fields.",
                ));
            }
        }
        "delegated" => {
            let pm = binding
                .pm
                .ok_or_else(|| Error::Invalid("delegated record has no package manager".into()))?;
            summary(&pm)?;
            rpm = pm == "rpm";
            record.recorded_backend = match pm.as_str() {
                "rpm" => "rpm",
                "deb" => "deb",
                _ => "unknown",
            }
            .into();
            let package = binding
                .package
                .ok_or_else(|| Error::Invalid("delegated record has no package identity".into()))?;
            match package.resolution.as_str() {
                "resolved" => {
                    let name = package
                        .name
                        .ok_or_else(|| Error::Invalid("resolved package has no name".into()))?;
                    summary(&name)?;
                    require(!name.is_empty(), "resolved package has an empty name")?;
                    record.recorded_package = Some(name);
                }
                "unresolved" => {
                    let hint = package.component_hint.ok_or_else(|| {
                        Error::Invalid("unresolved package has no identity hint".into())
                    })?;
                    summary(&hint)?;
                    record.findings.push(finding(
                        "unresolved_package",
                        "Native package identity was not resolved in the source record.",
                    ));
                }
                _ => return Err(Error::Invalid("invalid package identity resolution".into())),
            }
            let relation = binding.relation.ok_or_else(|| {
                Error::Invalid("delegated record has no management relation".into())
            })?;
            require(
                matches!(relation.kind.as_str(), "managed" | "adopted" | "observed"),
                "invalid recorded management relation",
            )?;
            if relation.kind != "observed" {
                require(
                    relation.since.is_some_and(|value| !value.is_empty()),
                    "management relation has no timestamp",
                )?;
            }
            record.recorded_relation = relation.kind;
            if let Some(observed) = binding.last_observed {
                require(
                    !observed.observed_at.is_empty(),
                    "native observation has no timestamp",
                )?;
                summary(&observed.version)?;
                record.recorded_version = Some(observed.version);
            }
            if !rpm {
                record.findings.push(finding(
                    "unsupported_backend",
                    "The recorded native package manager is not supported by this inspector.",
                ));
            }
            if binding.artifact.is_some() {
                record.recorded_relation = "unknown".into();
                record.findings.push(finding(
                    "ambiguous_record",
                    "Delegated authority also carries an owned artifact.",
                ));
            }
        }
        _ => record.findings.push(finding(
            "unsupported_authority",
            "The recorded installation authority is unknown; no relation is inferred.",
        )),
    }
    Ok(finish(record, rpm))
}

fn finish(mut record: Record, rpm_evidence: bool) -> Record {
    if rpm_evidence && record.recorded_scope.kind == "user" {
        record.findings.push(finding(
            "native_backend_in_user_scope",
            "RPM evidence appears in user scope; scope is not changed automatically.",
        ));
    }
    if record.recorded_scope.kind == "user" && record.recorded_scope.uid.is_none() {
        record.findings.push(finding(
            "unrecorded_user_identity",
            "The legacy record does not identify a uid; file ownership is reported separately.",
        ));
    }
    record
}
