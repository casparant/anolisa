#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;

use aw_contracts::common::{BoundedName, BoundedOpaque, TargetRef};
use aw_contracts::context::{ContextArtifactOrigin, ToolResultSubmission};
use aw_contracts::ids::{
    ActorId, AgentSessionId, AgentWorkId, AttemptId, EnvironmentId, ExecutionContextId, ToolUseId,
    TurnId,
};
use aw_provider_host::{ProviderAdmissionOptions, ProviderCatalog, ProviderManifestSource};
use serde_json::json;

use super::{
    context_artifact_id, context_projection_input, sha256_digest, tool_result_idempotency_key,
    Core, CoreConfig, CoreError, PrepareToolResultOptions, SessionContextSpec,
};

const INPUT_SCHEMA: &str =
    include_str!("../../aw-contracts/schemas/context-projection-prepare-input-v1.schema.json");
const OUTPUT_SCHEMA: &str =
    include_str!("../../aw-contracts/schemas/context-projection-prepare-output-v1.schema.json");
const EMPTY_SCHEMA_SHA256: &str =
    "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

#[test]
fn execution_context_allocates_once_or_preserves_a_propagated_identity() {
    let (_packages, core) = core_fixture(&["projection-a"]);
    let propagated = ExecutionContextId::new();
    let resumed = core
        .establish_execution_context(context_spec(Some(propagated.clone())))
        .expect("a valid propagated execution context is admitted");
    let allocated = core
        .establish_execution_context(context_spec(None))
        .expect("Core allocates a missing execution context");

    assert_eq!(resumed.execution_context_id(), &propagated);
    assert_ne!(allocated.execution_context_id(), &propagated);
}

#[test]
fn attempt_scope_requires_work() {
    let (_packages, core) = core_fixture(&["projection-a"]);
    let mut spec = context_spec(None);
    spec.attempt_id = Some(AttemptId::new());

    assert!(matches!(
        core.establish_execution_context(spec),
        Err(CoreError::AttemptWithoutWork)
    ));
}

#[test]
fn default_core_refuses_content_provider_without_enforced_controls() {
    let root = tempfile::tempdir().expect("fixture root is created");
    write_provider(root.path(), "projection-a");
    let catalog = ProviderCatalog::discover(
        ProviderManifestSource::Directory(root.path().to_path_buf()),
        &ProviderAdmissionOptions::default(),
    )
    .expect("fixture Provider is admitted");
    let core = Core::new(catalog);
    let context = core
        .establish_execution_context(context_spec(None))
        .expect("session scope is valid");

    let error = core
        .prepare_tool_result(
            &context,
            TurnId::new(),
            ToolUseId::new(),
            submission("source"),
            PrepareToolResultOptions::default(),
        )
        .expect_err("unenforced Provider requires explicit trust");

    assert!(matches!(error, CoreError::ProviderControlsNotEnforced));
}

#[test]
fn canonical_tool_input_contains_the_source_artifact_and_post_tool_boundary() {
    let artifact_id = aw_contracts::ids::ArtifactId::new();
    let submission = submission("original command output");
    let source_digest = sha256_digest(submission.content.as_bytes()).expect("SHA-256 is canonical");
    let input = context_projection_input(&artifact_id, &source_digest, &submission)
        .expect("typed context input serializes");

    assert_eq!(input.pointer("/artifact/id"), Some(&json!(artifact_id)));
    assert_eq!(
        input.pointer("/artifact/digest"),
        Some(&json!(source_digest))
    );
    assert_eq!(
        input.pointer("/artifact/content"),
        Some(&json!("original command output"))
    );
    assert_eq!(
        input.pointer("/artifact/origin"),
        Some(&json!("command_output"))
    );
    assert_eq!(input.pointer("/artifact/tool_name"), Some(&json!("shell")));
    assert_eq!(input.pointer("/boundary"), Some(&json!("post_tool")));
    assert_eq!(
        input.pointer("/constraints/allow_text_reencoding"),
        Some(&json!(true))
    );
}

#[test]
fn tool_result_route_populates_exact_scope_and_returns_content_free_receipt() {
    let (_packages, core) = core_fixture(&["projection-a"]);
    let work_id = AgentWorkId::new();
    let attempt_id = AttemptId::new();
    let propagated = ExecutionContextId::new();
    let mut spec = context_spec(Some(propagated.clone()));
    spec.work_id = Some(work_id.clone());
    spec.attempt_id = Some(attempt_id.clone());
    let context = core
        .establish_execution_context(spec)
        .expect("managed Work scope is valid");
    let turn_id = TurnId::new();
    let tool_use_id = ToolUseId::new();

    let prepared = core
        .prepare_tool_result(
            &context,
            turn_id.clone(),
            tool_use_id.clone(),
            submission("sensitive original output"),
            PrepareToolResultOptions::default(),
        )
        .expect("the unique exact Provider is invoked");

    let candidate = prepared
        .candidate
        .as_ref()
        .expect("the fixture reports a produced candidate");
    assert_eq!(candidate.content, "projected output");
    assert_eq!(candidate.source_artifact_id, prepared.source_artifact_id);
    assert_eq!(candidate.source_digest, prepared.source_digest);
    assert_eq!(prepared.receipt.provider_id.as_str(), "projection-a");
    assert_eq!(prepared.receipt.scope.execution_context_id, propagated);
    assert_eq!(prepared.receipt.scope.work_id, Some(work_id));
    assert_eq!(prepared.receipt.scope.attempt_id, Some(attempt_id));
    assert_eq!(prepared.receipt.scope.turn_id, Some(turn_id));
    assert_eq!(prepared.receipt.scope.tool_use_id, Some(tool_use_id));

    let receipt = serde_json::to_string(&prepared.receipt).expect("receipt serializes");
    assert!(!receipt.contains("sensitive original output"));
    assert!(!receipt.contains("projected output"));
}

#[test]
fn ambiguous_routes_require_an_explicit_provider_preference() {
    let (_packages, core) = core_fixture(&["projection-a", "projection-b"]);
    let context = core
        .establish_execution_context(context_spec(None))
        .expect("session scope is valid");

    let error = core
        .prepare_tool_result(
            &context,
            TurnId::new(),
            ToolUseId::new(),
            submission("source"),
            PrepareToolResultOptions::default(),
        )
        .expect_err("Core must not pick arbitrarily between eligible Providers");
    assert!(matches!(
        error,
        CoreError::AmbiguousContextProviders { provider_ids }
            if provider_ids == "projection-a, projection-b"
    ));

    let prepared = core
        .prepare_tool_result(
            &context,
            TurnId::new(),
            ToolUseId::new(),
            submission("source"),
            PrepareToolResultOptions {
                preferred_provider_id: Some(
                    BoundedName::new("projection-b").expect("fixture name is bounded"),
                ),
            },
        )
        .expect("an eligible explicit preference resolves the route");
    assert_eq!(prepared.receipt.provider_id.as_str(), "projection-b");
}

#[test]
fn tool_result_idempotency_is_stable_across_invocation_retries() {
    let tool_use_id = ToolUseId::new();
    let input_digest = sha256_digest(b"same canonical input").expect("SHA-256 is canonical");

    let first = tool_result_idempotency_key(&tool_use_id, &input_digest)
        .expect("derived replay key is bounded");
    let second = tool_result_idempotency_key(&tool_use_id, &input_digest)
        .expect("derived replay key is bounded");

    assert_eq!(first, second);
    assert!(first.as_str().starts_with("tool-result:tol_"));
    assert!(first.as_str().ends_with(input_digest.as_str()));
}

#[test]
fn one_observed_tool_result_has_a_stable_artifact_identity() {
    let context_id = ExecutionContextId::new();
    let turn_id = TurnId::new();
    let tool_use_id = ToolUseId::new();
    let source_digest = sha256_digest(b"same source").expect("SHA-256 is canonical");

    let first = context_artifact_id(&context_id, &turn_id, &tool_use_id, &source_digest)
        .expect("derived artifact ID is canonical");
    let second = context_artifact_id(&context_id, &turn_id, &tool_use_id, &source_digest)
        .expect("derived artifact ID is canonical");
    let other_tool = context_artifact_id(&context_id, &turn_id, &ToolUseId::new(), &source_digest)
        .expect("derived artifact ID is canonical");

    assert_eq!(first, second);
    assert_ne!(first, other_tool);
}

#[test]
fn repeated_preparation_reuses_the_observed_artifact() {
    let (_packages, core) = core_fixture(&["projection-a"]);
    let context = core
        .establish_execution_context(context_spec(None))
        .expect("session scope is valid");
    let turn_id = TurnId::new();
    let tool_use_id = ToolUseId::new();

    let first = core
        .prepare_tool_result(
            &context,
            turn_id.clone(),
            tool_use_id.clone(),
            submission("same source"),
            PrepareToolResultOptions::default(),
        )
        .expect("first preparation succeeds");
    let retried = core
        .prepare_tool_result(
            &context,
            turn_id,
            tool_use_id,
            submission("same source"),
            PrepareToolResultOptions::default(),
        )
        .expect("retry preparation succeeds");

    assert_eq!(first.source_artifact_id, retried.source_artifact_id);
    assert_eq!(first.source_digest, retried.source_digest);
    assert_ne!(
        first.receipt.invocation_id, retried.receipt.invocation_id,
        "each local attempt keeps its own invocation fact"
    );
}

fn core_fixture(provider_ids: &[&str]) -> (tempfile::TempDir, Core) {
    let root = tempfile::tempdir().expect("fixture root is created");
    for provider_id in provider_ids {
        write_provider(root.path(), provider_id);
    }
    let catalog = ProviderCatalog::discover(
        ProviderManifestSource::Directory(root.path().to_path_buf()),
        &ProviderAdmissionOptions::default(),
    )
    .expect("fixture Providers are admitted");
    (
        root,
        Core::with_config(
            catalog,
            CoreConfig {
                allow_unenforced_providers: true,
                ..CoreConfig::default()
            },
        )
        .expect("fixture Core configuration is valid"),
    )
}

fn write_provider(root: &std::path::Path, provider_id: &str) {
    let package = root.join(provider_id);
    fs::create_dir(&package).expect("Provider package directory is created");
    fs::write(package.join("input.schema.json"), INPUT_SCHEMA).expect("input schema is written");
    fs::write(package.join("output.schema.json"), OUTPUT_SCHEMA).expect("output schema is written");
    fs::write(package.join("native.schema.json"), "{}").expect("native schema is written");
    let executable = package.join("fake-provider.sh");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s' '{\"disposition\":\"applied\",\"output\":\"projected output\"}'\n",
    )
    .expect("fixture executable is written");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("fixture executable is made executable");
    fs::write(package.join("provider.toml"), manifest(provider_id))
        .expect("fixture manifest is written");
}

fn manifest(provider_id: &str) -> String {
    format!(
        r#"api_version = "providers.agentic-os.sh/v1"
provider_id = "{provider_id}"
provider_version = "1.0.0"
driver = "exec-json/v1"
lifecycle = "one_shot"

[executable]
command = "./fake-provider.sh"
args = []

[limits]
wall_time_ms = 1000
input_bytes = 1048576
output_bytes = 1048576

[permissions]
network = "none"
inherit_environment = false
filesystem_read = []
filesystem_write = []

[data]
reads = ["model_visible_context"]
writes = []
sensitivity = "inherits_input"
retention = "none"
telemetry = "disabled"

[[capabilities]]
capability = "context.projection.prepare/v1"
authority = "advise"
scopes = ["tool_call"]
input_contract = {{ schema = "context.projection.prepare.input/v1", resource = "input.schema.json", sha256 = "bdd09189791e34ce768e624bc19a5bf0d9569b8886b2a5f1c2408aeb8b8b5d9f" }}
output_contract = {{ schema = "context.projection.prepare.output/v1", resource = "output.schema.json", sha256 = "a295cf2b855899f9dfe5f1dda242d803af81852e6677526f79457c3214288028" }}
native_input = {{ resource = "native.schema.json", sha256 = "{EMPTY_SCHEMA_SHA256}" }}
native_output = {{ resource = "native.schema.json", sha256 = "{EMPTY_SCHEMA_SHA256}" }}

[capabilities.codec]
kind = "json-map/v1"

[[capabilities.codec.request.fields]]
target = "/content"
source = {{ kind = "input", pointer = "/artifact/content" }}
on_missing = "reject"

[capabilities.codec.response.disposition]
source = "/disposition"
on_unknown = "fail"

[capabilities.codec.response.disposition.values]
applied = "produced"

[[capabilities.codec.response.output_fields]]
target = "/candidate/source_artifact_id"
source = {{ kind = "input", pointer = "/artifact/id" }}
when_disposition = ["produced"]

[[capabilities.codec.response.output_fields]]
target = "/candidate/source_digest"
source = {{ kind = "input", pointer = "/artifact/digest" }}
when_disposition = ["produced"]

[[capabilities.codec.response.output_fields]]
target = "/candidate/content"
source = {{ kind = "response", pointer = "/output" }}
when_disposition = ["produced"]

[[capabilities.codec.response.output_fields]]
target = "/candidate/media_type"
source = {{ kind = "const", value = "text/plain" }}
when_disposition = ["produced"]

[[capabilities.codec.response.output_fields]]
target = "/candidate/transform_chain"
source = {{ kind = "const", value = ["fixture"] }}
when_disposition = ["produced"]

[[capabilities.codec.response.output_fields]]
target = "/candidate/reversibility"
source = {{ kind = "const", value = "lossless" }}
when_disposition = ["produced"]
"#
    )
}

fn context_spec(execution_context_id: Option<ExecutionContextId>) -> SessionContextSpec {
    SessionContextSpec {
        target: TargetRef {
            kind: BoundedName::new("host").expect("fixture target kind is bounded"),
            authority: BoundedName::new("local").expect("fixture target authority is bounded"),
            identifier: BoundedOpaque::new("fixture-host")
                .expect("fixture target identifier is bounded"),
        },
        environment_id: EnvironmentId::new(),
        actor_id: ActorId::new(),
        agent_session_id: Some(AgentSessionId::new()),
        work_id: None,
        attempt_id: None,
        execution_context_id,
    }
}

fn submission(content: &str) -> ToolResultSubmission {
    ToolResultSubmission {
        content: content.to_owned(),
        media_type: BoundedName::new("text/plain").expect("fixture media type is bounded"),
        origin: ContextArtifactOrigin::CommandOutput,
        tool_name: Some(BoundedName::new("shell").expect("fixture tool name is bounded")),
        allow_text_reencoding: true,
    }
}
