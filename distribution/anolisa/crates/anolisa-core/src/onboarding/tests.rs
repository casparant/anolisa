//! Configuration preparation stays private, inert and explicit across unattended invocations.

use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    _temporary: tempfile::TempDir,
    environment: Environment,
    command: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let command = root.join("agent");
        fs::write(&command, b"Agent fixture; never executed").unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
        let environment = Environment::from_values(root, |_| None).unwrap();
        Self {
            _temporary: temporary,
            environment,
            command,
        }
    }
    fn request(&self) -> SetupRequest<'_> {
        SetupRequest {
            environment: &self.environment,
            config: self.environment.config.join("anolisa/aw.yaml"),
            adapter: "example".into(),
            supported_adapters: vec!["example".into()],
            agent_argv: vec![self.command.display().to_string()],
            provider_template: None,
            providers: Vec::new(),
        }
    }
}

#[test]
fn dry_run_creates_nothing_and_configures_no_provider_or_policy_event() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let report = setup(&request, true).unwrap();
    assert!(report.created.is_empty() && report.error.is_none());
    assert!(!request.config.exists());
    assert!(!fixture.environment.state.exists());
    assert_eq!(report.plan.document["spec"]["providers"], json!({}));
    assert_eq!(report.plan.document["spec"]["events"], json!({}));
    assert_eq!(
        report.plan.document["spec"]["agents"]["agent"]["argv"][0],
        fixture.command.to_str().unwrap()
    );
}

#[test]
fn user_config_is_private_and_existing_config_is_never_overwritten() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let report = setup(&request, false).unwrap();
    assert_eq!(report.created, vec![request.config.clone()]);
    assert_eq!(request.config.metadata().unwrap().mode() & 0o777, 0o600);
    let before = fs::read(&request.config).unwrap();
    assert!(setup(&request, false).is_err());
    assert_eq!(fs::read(&request.config).unwrap(), before);
    assert!(!fixture.environment.state.exists());
}

#[test]
fn unknown_adapter_and_symlink_destination_fail_before_writing() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.adapter = "not-in-installed-catalog".into();
    assert!(plan_setup(&request).is_err());
    request.adapter = "example".into();
    let alias = fixture.environment.home.join("linked-config");
    symlink(&fixture.environment.home, &alias).unwrap();
    request.config = alias.join("aw.yaml");
    assert!(setup(&request, false).is_err());
    assert!(!fixture.environment.home.join("aw.yaml").exists());
}

#[test]
fn explicitly_selected_provider_is_rendered_into_separate_disabled_definitions() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    let template = fixture.environment.home.join("providers.json");
    fs::write(&template, serde_json::to_vec(&json!({"schema_version":1,"kind":"anolisa-provider-template","enabled":false,
        "providers":{"example":{"protocol":"aw-provider/v1alpha1","transport_template":{"type":"stdio","location":"agent",
        "argv":[fixture.command,"--data-dir","{user_data_dir}/example"]}}}})).unwrap()).unwrap();
    request.provider_template = Some(template);
    request.providers = vec!["example".into()];
    let report = setup(&request, false).unwrap();
    assert!(report.error.is_none());
    let definitions = report.plan.provider_definitions.unwrap();
    assert_eq!(definitions.metadata().unwrap().mode() & 0o777, 0o600);
    let value: Value = serde_json::from_slice(&fs::read(definitions).unwrap()).unwrap();
    assert_eq!(value["enabled"], false);
    assert_eq!(
        value["providers"]["example"]["transport"]["argv"][2],
        fixture
            .environment
            .data
            .join("anolisa/example")
            .to_str()
            .unwrap()
    );
    let config: Value = serde_json::from_slice(&fs::read(request.config).unwrap()).unwrap();
    assert_eq!(config["spec"]["providers"], json!({}));
    assert_eq!(config["spec"]["events"], json!({}));
}

#[test]
fn runtime_placeholder_requires_secure_invoking_user_runtime_before_any_publication() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    let template = fixture.environment.home.join("providers.json");
    fs::write(&template, serde_json::to_vec(&json!({"schema_version":1,"kind":"anolisa-provider-template","enabled":false,
        "providers":{"example":{"protocol":"aw-provider/v1alpha1","transport_template":{"type":"stdio","location":"agent",
        "argv":[fixture.command,"--socket","{user_runtime_dir}/service.sock"]}}}})).unwrap()).unwrap();
    request.provider_template = Some(template);
    request.providers = vec!["example".into()];
    assert!(setup(&request, false).is_err());
    assert!(!request.config.exists());
}

#[test]
fn agent_catalog_uses_the_installed_read_only_command_and_rejects_unsupported_output() {
    let fixture = Fixture::new();
    fs::write(&fixture.command, b"#!/bin/sh\n[ \"$*\" = 'agent list --json' ] || exit 3\nprintf '%s\\n' '{\"format\":1,\"adapters\":[{\"name\":\"example\"}]}'\n").unwrap();
    assert_eq!(agent_catalog(&fixture.command).unwrap(), vec!["example"]);
    fs::write(
        &fixture.command,
        b"#!/bin/sh\nprintf '%s\\n' '{\"format\":1,\"adapters\":[]}'\n",
    )
    .unwrap();
    assert!(agent_catalog(&fixture.command).is_err());
}
