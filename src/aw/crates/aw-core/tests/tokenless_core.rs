#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use aw_contracts::common::{BoundedName, BoundedOpaque, TargetRef};
use aw_contracts::context::{ContextArtifactOrigin, ToolResultSubmission};
use aw_contracts::ids::{ActorId, AgentSessionId, EnvironmentId, ToolUseId, TurnId};
use aw_core::{Core, CoreConfig, PrepareToolResultOptions, SessionContextSpec};
use aw_provider_host::{ProviderAdmissionOptions, ProviderCatalog, ProviderManifestSource};

#[test]
#[ignore = "requires src/tokenless/target/debug/tokenless"]
fn core_prepares_a_real_tool_result_through_tokenless() {
    let repository = repository_root();
    let package = repository.join("providers/tokenless");
    let executable_root = repository.join("src/tokenless/target/debug");
    assert!(
        executable_root.join("tokenless").is_file(),
        "build Tokenless first: cd {} && cargo build --bin tokenless",
        repository.join("src/tokenless").display()
    );
    let catalog = ProviderCatalog::discover(
        ProviderManifestSource::File(package.join("provider.toml")),
        &ProviderAdmissionOptions {
            executable_roots: vec![executable_root],
        },
    )
    .expect("the real Tokenless package is admitted");
    let core = Core::with_config(
        catalog,
        CoreConfig {
            allow_unenforced_providers: true,
            ..CoreConfig::default()
        },
    )
    .expect("the trusted-Provider PoC configuration is valid");
    let context = core
        .establish_execution_context(SessionContextSpec {
            target: TargetRef {
                kind: BoundedName::new("host").expect("target kind is bounded"),
                authority: BoundedName::new("local").expect("target authority is bounded"),
                identifier: BoundedOpaque::new("tokenless-integration-host")
                    .expect("target identifier is bounded"),
            },
            environment_id: EnvironmentId::new(),
            actor_id: ActorId::new(),
            agent_session_id: Some(AgentSessionId::new()),
            work_id: None,
            attempt_id: None,
            execution_context_id: None,
        })
        .expect("the session context is valid");
    let fixture: serde_json::Value = serde_json::from_slice(
        &fs::read(package.join("fixtures/context-projection-prepare.json"))
            .expect("the canonical Tokenless fixture is readable"),
    )
    .expect("the canonical Tokenless fixture is JSON");
    let source = fixture
        .pointer("/artifact/content")
        .and_then(serde_json::Value::as_str)
        .expect("the fixture contains tool-result content")
        .to_owned();

    let prepared = core
        .prepare_tool_result(
            &context,
            TurnId::new(),
            ToolUseId::new(),
            ToolResultSubmission {
                content: source.clone(),
                media_type: BoundedName::new("application/json").expect("media type is bounded"),
                origin: ContextArtifactOrigin::ApiResponse,
                tool_name: Some(
                    BoundedName::new("list_recent_builds").expect("tool name is bounded"),
                ),
                allow_text_reencoding: true,
            },
            PrepareToolResultOptions::default(),
        )
        .expect("Core invokes Tokenless through the generic Provider Host");

    let candidate = prepared
        .candidate
        .expect("Tokenless produces a context projection for the fixture");
    assert_eq!(candidate.source_artifact_id, prepared.source_artifact_id);
    assert_eq!(candidate.source_digest, prepared.source_digest);
    assert!(!candidate.content.is_empty());
    assert_ne!(candidate.content, source);
    assert_eq!(prepared.receipt.provider_id.as_str(), "tokenless");
    assert_eq!(prepared.receipt.meters.len(), 2);
    let receipt = serde_json::to_string(&prepared.receipt).expect("receipt serializes");
    assert!(!receipt.contains("scheduler trace retained only for operator diagnostics"));
    assert!(!receipt.contains(&candidate.content));
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .expect("AW Core remains under src/aw/crates")
        .to_path_buf()
}
