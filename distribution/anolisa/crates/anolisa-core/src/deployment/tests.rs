//! Contract and process-lifecycle tests avoid starting any installed backend.

use super::*;
use serde_json::json;
use std::cell::RefCell;
use std::path::Path;

fn document() -> serde_json::Value {
    json!({
        "schema_version":1,
        "services":[{
            "id":"fixture", "package":"fixture-core", "program":"/bin/sh",
            "args":["-c","sleep 60"], "scope":"user", "run_as":"same-user",
            "communication":"none", "container_allowed":true, "unit":"fixture.service",
            "depends_on":[], "required_paths":[],
            "readiness":{"kind":"process","delay_ms":20,"timeout_ms":500},
            "stop_timeout_ms":100
        }],
        "presets":{"container":{"services":[{"id":"fixture","enable":false,"start":true}]},
            "disabled":{"services":[{"id":"fixture","enable":false,"start":false}]}}
    })
}

fn config(document: serde_json::Value) -> DeploymentConfig {
    DeploymentConfig::parse(&serde_json::to_vec(&document).unwrap()).unwrap()
}

#[test]
fn description_rejects_schema_changes_unknown_fields_privilege_and_stdio() {
    for (field, value) in [
        ("scope", json!("system")),
        ("run_as", json!("root")),
        ("communication", json!("mcp-stdio")),
        ("unit", json!("--system.service")),
        ("stop_timeout_ms", json!(0)),
    ] {
        let mut value_document = document();
        value_document["services"][0][field] = value;
        assert!(
            DeploymentConfig::parse(&serde_json::to_vec(&value_document).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut value = document();
    value["schema_version"] = json!(2);
    assert!(DeploymentConfig::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    value["schema_version"] = json!(1);
    value["unrecognized"] = json!(true);
    assert!(DeploymentConfig::parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn dependency_graph_requires_explicit_selection_and_rejects_cycles() {
    let mut value = document();
    let mut second = value["services"][0].clone();
    second["id"] = json!("second");
    second["unit"] = json!("second.service");
    value["services"].as_array_mut().unwrap().push(second);
    value["services"][0]["depends_on"] = json!(["second"]);
    let catalog = config(value.clone());
    assert!(
        plan_serve(&catalog, "container")
            .unwrap_err()
            .to_string()
            .contains("explicit selection")
    );
    value["services"][1]["depends_on"] = json!(["fixture"]);
    assert!(
        DeploymentConfig::parse(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn preflight_is_read_only_and_checks_real_paths_without_shell_evaluation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("should-not-exist");
    let mut value = document();
    value["services"][0]["args"] = json!(["-c", format!("touch {}", marker.display())]);
    let plan = plan_serve(&config(value.clone()), "container").unwrap();
    assert_eq!(plan.services.len(), 1);
    assert!(!marker.exists());
    value["services"][0]["required_paths"] = json!([marker]);
    assert!(plan_serve(&config(value), "container").is_err());
    assert!(!marker.exists());
}

#[derive(Default)]
struct FakeSystemctl {
    calls: RefCell<Vec<String>>,
    masked: bool,
}

impl Systemctl for FakeSystemctl {
    fn inspect(&self, scope: Scope, unit: &str) -> Result<UnitState, DeploymentError> {
        self.calls
            .borrow_mut()
            .push(format!("inspect:{scope:?}:{unit}"));
        Ok(UnitState {
            loaded: true,
            enable_state: if self.masked { "masked" } else { "disabled" }.into(),
            active: false,
        })
    }
    fn preset(&self, scope: Scope, unit: &str, enable: bool) -> Result<(), DeploymentError> {
        self.calls
            .borrow_mut()
            .push(format!("preset:{scope:?}:{unit}:{enable}"));
        Ok(())
    }
    fn start(&self, scope: Scope, unit: &str) -> Result<(), DeploymentError> {
        self.calls
            .borrow_mut()
            .push(format!("start:{scope:?}:{unit}"));
        Ok(())
    }
}

#[test]
fn native_plan_keeps_enable_and_start_separate_and_never_writes() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("missing/state.json");
    let native = FakeSystemctl::default();
    let plan = plan_preset(
        &config(document()),
        "container",
        Scope::User,
        &state,
        false,
        &native,
    )
    .unwrap();
    assert!(matches!(plan.actions[0].kind, PresetActionKind::Preserve));
    assert!(matches!(plan.actions[1].kind, PresetActionKind::Start));
    assert!(!state.parent().unwrap().exists());
    assert_eq!(*native.calls.borrow(), ["inspect:User:fixture.service"]);
    let plan = plan_preset(
        &config(document()),
        "disabled",
        Scope::User,
        &state,
        true,
        &native,
    )
    .unwrap();
    assert!(matches!(
        plan.actions[0].kind,
        PresetActionKind::Initialize { enable: false }
    ));
    assert_eq!(plan.actions.len(), 1);
}

#[test]
fn masks_are_preserved_and_block_explicit_start() {
    let directory = tempfile::tempdir().unwrap();
    let native = FakeSystemctl {
        masked: true,
        ..FakeSystemctl::default()
    };
    let state = directory.path().join("state.json");
    let plan = plan_preset(
        &config(document()),
        "disabled",
        Scope::User,
        &state,
        true,
        &native,
    )
    .unwrap();
    assert!(matches!(plan.actions[0].kind, PresetActionKind::Preserve));
    assert!(
        plan_preset(
            &config(document()),
            "container",
            Scope::User,
            &state,
            true,
            &native
        )
        .unwrap_err()
        .to_string()
        .contains("masked")
    );
}

#[test]
fn applied_defaults_are_recorded_once_and_later_admin_choices_survive() {
    #[cfg(unix)]
    if nix::unistd::Uid::effective().is_root() || std::env::var_os("SUDO_UID").is_some() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().canonicalize().unwrap().join("state.json");
    let native = FakeSystemctl::default();
    let catalog = config(document());
    apply_preset(&catalog, "disabled", Scope::User, &state, true, &native).unwrap();
    apply_preset(&catalog, "disabled", Scope::User, &state, true, &native).unwrap();
    assert_eq!(
        native
            .calls
            .borrow()
            .iter()
            .filter(|call| call.starts_with("preset:"))
            .count(),
        1
    );
    assert!(
        std::fs::read_to_string(state)
            .unwrap()
            .contains("default_applied")
    );
}

#[cfg(unix)]
fn write_script(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{content}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

// The isolated child owns signal handlers; the parent test never changes the
// harness's handlers or signals a process outside the temporary fixture group.
#[test]
fn supervisor_child() {
    let Some(path) = std::env::var_os("ANOLISA_DEPLOYMENT_TEST_CONFIG") else {
        return;
    };
    let catalog = DeploymentConfig::read(Path::new(&path)).unwrap();
    let result = serve(&catalog, "container");
    match result {
        Err(DeploymentError::Interrupted(signal)) => std::process::exit(128 + signal),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(41);
        }
        Ok(_) => std::process::exit(0),
    }
}

#[cfg(unix)]
fn spawn_supervisor(value: serde_json::Value, directory: &Path) -> std::process::Child {
    let path = directory.join("catalog.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "deployment::tests::supervisor_child",
            "--nocapture",
        ])
        .env("ANOLISA_DEPLOYMENT_TEST_CONFIG", path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

#[cfg(unix)]
fn wait_file(path: &Path, child: &mut std::process::Child) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "supervisor exited before {}",
            path.display()
        );
        assert!(
            std::time::Instant::now() < deadline,
            "fixture did not become ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn wait_child(child: &mut std::process::Child) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("supervisor did not stop before deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn term_and_int_stop_process_groups_and_force_kill_ignoring_descendants() {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    for signal in [Signal::SIGTERM, Signal::SIGINT] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("child.pid");
        let script = directory.path().join("fixture.sh");
        write_script(
            &script,
            &format!(
                "trap 'exit 0' TERM INT\n(trap '' TERM INT; exec sleep 60) &\necho $! > '{}'\nwait",
                marker.display()
            ),
        );
        let mut value = document();
        value["services"][0]["program"] = json!(script);
        value["services"][0]["args"] = json!([]);
        let mut child = spawn_supervisor(value, directory.path());
        wait_file(&marker, &mut child);
        let descendant: i32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        kill(Pid::from_raw(child.id() as i32), signal).unwrap();
        assert_eq!(wait_child(&mut child).code(), Some(128 + signal as i32));
        // Grandchildren are adopted by the host/OCI init. A zombie has already
        // terminated; macOS normally reaps it before this bounded check ends.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if kill(Pid::from_raw(descendant), None).is_err() {
                break;
            }
            #[cfg(target_os = "linux")]
            if std::fs::read_to_string(format!("/proc/{descendant}/stat"))
                .is_ok_and(|line| line.split_whitespace().nth(2) == Some("Z"))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "descendant {descendant} survived process-group shutdown"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
#[test]
fn readiness_timeout_stops_children_and_required_exit_stops_siblings() {
    for timeout in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fixture.sh");
        let marker = directory.path().join("pid");
        write_script(
            &script,
            &format!(
                "echo $$ > '{}'\n{}",
                marker.display(),
                if timeout { "exec sleep 60" } else { "exit 7" }
            ),
        );
        let mut value = document();
        value["services"][0]["program"] = json!(script);
        value["services"][0]["args"] = json!([]);
        if timeout {
            value["services"][0]["readiness"] = json!({"kind":"unix-socket","path":directory.path().join("missing.sock"),"timeout_ms":1000});
        }
        let mut child = spawn_supervisor(value, directory.path());
        assert_eq!(wait_child(&mut child).code(), Some(41));
        let pid: i32 = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err());
    }
}

#[cfg(unix)]
#[test]
fn socket_server_child() {
    let Some(path) = std::env::var_os("ANOLISA_DEPLOYMENT_TEST_CONFIG") else {
        return;
    };
    let catalog = DeploymentConfig::read(Path::new(&path)).unwrap();
    let Readiness::UnixSocket { path, .. } = &catalog.services[0].readiness else {
        return;
    };
    let _listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[cfg(unix)]
#[test]
fn real_socket_readiness_precedes_dependents_and_failed_sibling_stops_the_group() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ready.sock");
    let sibling = directory.path().join("sibling.sh");
    let marker = directory.path().join("sibling-started");
    write_script(
        &sibling,
        &format!(
            "test -S '{}' || exit 87\necho ready > '{}'\nexit 7",
            socket.display(),
            marker.display()
        ),
    );
    let mut value = document();
    value["services"][0]["program"] = json!(std::env::current_exe().unwrap());
    value["services"][0]["args"] = json!([
        "--exact",
        "deployment::tests::socket_server_child",
        "--nocapture"
    ]);
    value["services"][0]["readiness"] =
        json!({"kind":"unix-socket","path":socket,"timeout_ms":3000});
    let mut second = value["services"][0].clone();
    second["id"] = json!("sibling");
    second["unit"] = json!("sibling.service");
    second["program"] = json!(sibling);
    second["args"] = json!([]);
    second["depends_on"] = json!(["fixture"]);
    second["readiness"] = json!({"kind":"process","delay_ms":20,"timeout_ms":1000});
    value["services"].as_array_mut().unwrap().push(second);
    value["presets"]["container"]["services"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"sibling","enable":false,"start":true}));
    let mut child = spawn_supervisor(value, directory.path());
    assert_eq!(wait_child(&mut child).code(), Some(41));
    assert_eq!(std::fs::read_to_string(marker).unwrap().trim(), "ready");
    assert!(
        std::os::unix::net::UnixStream::connect(socket).is_err(),
        "server sibling survived required service failure"
    );
}

#[test]
fn shipped_catalog_has_safe_defaults_and_does_not_share_stdio_or_privileged_backends() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = [
        workspace.join("../release/services.json"),
        workspace.join("distribution/release/services.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .expect("source and standalone snapshots carry the same product catalog");
    let catalog = DeploymentConfig::read(&source).unwrap();
    assert!(
        catalog.presets["disabled"]
            .services
            .iter()
            .all(|item| !item.enable && !item.start)
    );
    for item in &catalog.presets["container"].services {
        let service = catalog
            .services
            .iter()
            .find(|service| service.id == item.id)
            .unwrap();
        assert!(service.container_allowed);
        assert_eq!(service.run_as, RunAs::SameUser);
        assert_ne!(service.communication, Communication::McpStdio);
    }
    assert!(
        catalog
            .services
            .iter()
            .all(|service| service.id != "agent-memory")
    );
}
