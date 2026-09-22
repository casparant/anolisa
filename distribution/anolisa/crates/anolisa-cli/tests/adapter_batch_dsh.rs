//! End-to-end coverage for framework-wide adapter batches
//! (`adapter enable --all dsh`, its disable counterpart, and the
//! `adapter status --framework` filter).
//!
//! DSH is the motivating ecosystem entry: one command must wire every
//! installed component that declares a `dsh` adapter into the same explicit
//! profiles. The fixture installs two such components and points `DSH_BIN` at
//! a fake `dsh` CLI that keeps per-profile bundle registries, so the real
//! binary drives the real driver, receipts, and lock without a DSH install.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

use anolisa_core::adapter::claim::ClaimStatus;
use anolisa_core::state_store::StateStore;
use anolisa_core::{
    FileOwner, InstallMode as StateInstallMode, InstalledObject, InstalledState, ObjectKind,
    ObjectStatus, OwnedFile, OwnedFileKind, Ownership, SubscriptionScope,
};
use anolisa_platform::fs_layout::FsLayout;
use anolisa_platform::privilege;
use sha2::{Digest, Sha256};

mod common;

/// Components installed by the fixture, with their DSH package identities.
const TOKENLESS: (&str, &str) = ("tokenless-demo", "anolisa-tokenless");
const SIGHT: (&str, &str) = ("sight-demo", "agentsight-dsh");

/// Fake `dsh` CLI: records argv, then keeps one bundle registry per profile so
/// `plugin --profile <p> add link:<root>` and `remove <package>` are reflected
/// in the `profiles/<p>/package.json` the real driver reads back for status.
/// A marker file under `$FAKE_DSH_FAIL_DIR/<package>` makes `add` exit 1.
const FAKE_DSH: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_DSH_LOG"
[ "$1" = "plugin" ] || exit 0
shift
profile=""
action=""
target=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile) profile="$2"; shift 2 ;;
    add|remove)
      action="$1"
      shift
      if [ "$#" -gt 0 ]; then target="$1"; shift; fi
      ;;
    *) shift ;;
  esac
done
dir="$DSH_HOME/profiles/$profile"
mkdir -p "$dir/bundles" || exit 1
if [ "$action" = "add" ]; then
  root="${target#link:}"
  name=$(sed -n 's/.*"name":"\([^"]*\)".*/\1/p' "$root/package.json" | head -1)
  if [ -n "$FAKE_DSH_FAIL_DIR" ] && [ -f "$FAKE_DSH_FAIL_DIR/$name" ]; then
    echo "fake dsh refused to add $name" >&2
    exit 1
  fi
  printf '%s' "$root" > "$dir/bundles/$name"
elif [ "$action" = "remove" ]; then
  mkdir -p "$dir/retired"
  mv -f "$dir/bundles/$target" "$dir/retired/$target"
fi
deps=""
bundles=""
for entry in "$dir/bundles"/*; do
  [ -e "$entry" ] || continue
  name=$(basename "$entry")
  root=$(cat "$entry")
  deps="$deps\"$name\":\"link:$root\","
  bundles="$bundles\"$name\","
done
printf '{"name":"profile-%s","dependencies":{%s},"dsh":{"profile":{"bundles":[%s]}}}\n' \
  "$profile" "${deps%,}" "${bundles%,}" > "$dir/package.json"
"#;

struct BatchFixture {
    _tmp: tempfile::TempDir,
    system_prefix: PathBuf,
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    state_home: PathBuf,
    cache_home: PathBuf,
    runtime_dir: PathBuf,
    user_layout: FsLayout,
    dsh_bin: PathBuf,
    dsh_home: PathBuf,
    dsh_log: PathBuf,
    fail_dir: PathBuf,
}

impl BatchFixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let system_prefix = root.join("system");
        let home = root.join("home");
        let data_home = root.join("xdg-data");
        let config_home = root.join("xdg-config");
        let state_home = root.join("xdg-state");
        let cache_home = root.join("xdg-cache");
        let runtime_dir = root.join("xdg-runtime");
        let user_layout = FsLayout::user_with_overrides(
            home.clone(),
            Some(data_home.clone()),
            Some(config_home.clone()),
            Some(state_home.clone()),
            Some(cache_home.clone()),
            Some(runtime_dir.clone()),
        );
        let system_layout = FsLayout::system(Some(system_prefix.clone()));

        let bin_dir = root.join("fake-bin");
        std::fs::create_dir_all(&bin_dir).expect("fake bin dir");
        let dsh_bin = bin_dir.join("dsh");
        std::fs::write(&dsh_bin, FAKE_DSH).expect("write fake dsh");
        std::fs::set_permissions(&dsh_bin, std::fs::Permissions::from_mode(0o755))
            .expect("fake dsh must be executable");

        let dsh_home = root.join("dsh-home");
        let fail_dir = root.join("dsh-failures");
        let dsh_log = root.join("dsh-invocations.log");
        std::fs::create_dir_all(&fail_dir).expect("failure marker dir");

        let mut objects = Vec::new();
        for (component, package) in [TOKENLESS, SIGHT] {
            objects.push(install_dsh_component(&user_layout, component, package));
        }
        write_state(&user_layout, StateInstallMode::User, objects);
        write_state(&system_layout, StateInstallMode::System, Vec::new());

        Self {
            _tmp: tmp,
            system_prefix,
            home,
            data_home,
            config_home,
            state_home,
            cache_home,
            runtime_dir,
            user_layout,
            dsh_bin,
            dsh_home,
            dsh_log,
            fail_dir,
        }
    }

    /// Run `anolisa [flags] adapter [sub_args]` against this fixture.
    ///
    /// `--no-color` keeps human-output assertions byte-stable: the palette is
    /// on by default and emits escapes even when stdout is a pipe.
    fn run(&self, flags: &[&str], sub_args: &[&str]) -> Output {
        let prefix = self.system_prefix.to_string_lossy();
        let mut args: Vec<&str> = vec!["--no-color"];
        args.extend_from_slice(flags);
        args.extend(["--install-mode", "user", "--prefix", &prefix, "adapter"]);
        args.extend_from_slice(sub_args);
        common::run_with_path_env(&args, &self.env())
    }

    fn enable_all(&self, flags: &[&str]) -> Output {
        let mut sub_args: Vec<&str> = vec!["enable", "--all", "dsh"];
        sub_args.extend_from_slice(flags);
        self.run(&[], &sub_args)
    }

    fn env(&self) -> Vec<(&str, &Path)> {
        vec![
            ("HOME", self.home.as_path()),
            ("XDG_DATA_HOME", self.data_home.as_path()),
            ("XDG_CONFIG_HOME", self.config_home.as_path()),
            ("XDG_STATE_HOME", self.state_home.as_path()),
            ("XDG_CACHE_HOME", self.cache_home.as_path()),
            ("XDG_RUNTIME_DIR", self.runtime_dir.as_path()),
            ("DSH_BIN", self.dsh_bin.as_path()),
            ("DSH_HOME", self.dsh_home.as_path()),
            ("FAKE_DSH_LOG", self.dsh_log.as_path()),
            ("FAKE_DSH_FAIL_DIR", self.fail_dir.as_path()),
        ]
    }

    /// Make `dsh plugin add` fail for one package.
    fn fail_package(&self, package: &str) {
        std::fs::write(self.fail_dir.join(package), b"fail").expect("failure marker");
    }

    fn store(&self) -> StateStore {
        StateStore::load(
            &self.user_layout.state_dir.join("installed.toml"),
            privilege::effective_uid(),
        )
        .expect("load state")
    }

    fn has_receipt(&self, component: &str) -> bool {
        self.receipt_status(component).is_some()
    }

    /// Lifecycle status of a component's `dsh` receipt, when one exists.
    fn receipt_status(&self, component: &str) -> Option<ClaimStatus> {
        self.store()
            .find_adapter_claim(component, "dsh")
            .map(|claim| claim.status)
    }

    /// The profile manifest the fake `dsh` maintains.
    fn profile_manifest(&self, profile: &str) -> String {
        std::fs::read_to_string(
            self.dsh_home
                .join("profiles")
                .join(profile)
                .join("package.json"),
        )
        .unwrap_or_default()
    }

    fn dsh_invocations(&self) -> String {
        std::fs::read_to_string(&self.dsh_log).unwrap_or_default()
    }
}

/// Lay out one installed component: contract snapshot declaring a `dsh`
/// adapter, the immutable bundle it points at, and the ownership record.
fn install_dsh_component(layout: &FsLayout, component: &str, package: &str) -> InstalledObject {
    let manifest = format!(
        r#"[component]
name = "{component}"
version = "0.1.0"

[[adapters]]
framework = "dsh"
adapter_type = "plugin"
plugin_id = "{package}"
source = "adapters/{component}/dsh"
dest = "{{datadir}}/adapters/{{component}}/dsh/"

[adapters.bundle]
entry = "package.json"
"#
    );
    let manifest_path = layout.snapshot_path(component);
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("manifest dir");
    std::fs::write(&manifest_path, manifest).expect("component manifest");

    let bundle = layout.datadir.join("adapters").join(component).join("dsh");
    std::fs::create_dir_all(&bundle).expect("bundle root");
    let package_json = bundle.join("package.json");
    let patch = bundle.join("cordis.patch.yml");
    std::fs::write(
        &package_json,
        format!(r#"{{"name":"{package}","dsh":{{"bundle":{{"patch":"./cordis.patch.yml"}}}}}}"#),
    )
    .expect("bundle package.json");
    std::fs::write(
        &patch,
        format!("- insert:\n    - id: {package}\n      name: '@anolisa/{package}'\n"),
    )
    .expect("bundle patch");

    let mut object = component_record(component);
    object.files.push(owned_file(&manifest_path));
    object.files.push(owned_file(&package_json));
    object.files.push(owned_file(&patch));
    object
}

fn owned_file(path: &Path) -> OwnedFile {
    let bytes = std::fs::read(path).expect("read owned file");
    OwnedFile {
        path: path.to_path_buf(),
        owner: FileOwner::Anolisa,
        sha256: Some(format!("{:x}", Sha256::digest(bytes))),
        kind: OwnedFileKind::File,
        referent: None,
        mode: None,
        capabilities: Vec::new(),
    }
}

fn component_record(name: &str) -> InstalledObject {
    InstalledObject {
        kind: ObjectKind::Component,
        name: name.to_string(),
        version: "0.1.0".to_string(),
        status: ObjectStatus::Installed,
        manifest_digest: None,
        distribution_source: None,
        raw_package: None,
        install_backend: Some("raw".to_string()),
        ownership: Some(Ownership::RawManaged),
        rpm_metadata: None,
        installed_at: "2026-01-01T00:00:00Z".to_string(),
        last_operation_id: None,
        managed: true,
        adopted: false,
        subscription_scope: SubscriptionScope::None,
        enabled_features: Vec::new(),
        component_refs: Vec::new(),
        files: Vec::new(),
        external_modified_files: Vec::new(),
        services: Vec::new(),
        health: Vec::new(),
        provisioned_packages: Vec::new(),
    }
}

fn write_state(layout: &FsLayout, mode: StateInstallMode, objects: Vec<InstalledObject>) {
    std::fs::create_dir_all(&layout.state_dir).expect("state dir");
    std::fs::write(
        layout.state_dir.join("installed.toml"),
        toml::to_string_pretty(&InstalledState {
            install_mode: mode,
            prefix: layout.prefix.clone(),
            objects,
            ..InstalledState::default()
        })
        .expect("legacy fixture"),
    )
    .expect("state");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn envelope(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("json envelope")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "exit {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn enable_all_registers_every_declared_dsh_component_in_the_same_profiles() {
    let fixture = BatchFixture::new();
    let output = fixture.enable_all(&["--profile", "web", "--profile", "dev"]);
    assert_success(&output);

    let out = stdout(&output);
    assert!(out.contains("enabled tokenless-demo/dsh"), "{out}");
    assert!(out.contains("enabled sight-demo/dsh"), "{out}");
    assert!(out.contains("summary: total=2"), "{out}");

    // Both packages reached both profiles through the framework's own CLI.
    let invocations = fixture.dsh_invocations();
    let adds = invocations
        .lines()
        .filter(|line| line.contains(" add link:"))
        .count();
    assert_eq!(adds, 4, "two components x two profiles: {invocations}");
    for profile in ["web", "dev"] {
        let manifest = fixture.profile_manifest(profile);
        assert!(manifest.contains(TOKENLESS.1), "{manifest}");
        assert!(manifest.contains(SIGHT.1), "{manifest}");
    }

    assert_eq!(
        fixture.receipt_status(TOKENLESS.0),
        Some(ClaimStatus::Enabled)
    );
    assert_eq!(fixture.receipt_status(SIGHT.0), Some(ClaimStatus::Enabled));
}

#[test]
fn enable_all_dry_run_plans_every_member_without_touching_framework_state() {
    let fixture = BatchFixture::new();
    let output = fixture.enable_all(&["--profile", "web", "--dry-run", "--json"]);
    assert_success(&output);

    let data = &envelope(&output)["data"];
    assert_eq!(data["dry_run"], true);
    assert_eq!(data["framework"], "dsh");
    assert_eq!(data["total"], 2);
    assert_eq!(data["failed"], 0);
    let items = data["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(item["status"], "planned", "{item}");
        assert!(
            item["plan"]
                .as_array()
                .is_some_and(|plan| plan.iter().any(|action| action
                    .as_str()
                    .is_some_and(|text| text.contains("profile 'web'")))),
            "{item}"
        );
    }

    assert!(!fixture.has_receipt(TOKENLESS.0));
    assert!(!fixture.has_receipt(SIGHT.0));
    assert!(
        fixture.dsh_invocations().is_empty(),
        "dry-run must not call the framework CLI"
    );
    assert!(
        !fixture.dsh_home.join("profiles").exists(),
        "dry-run must not create profile state"
    );
}

#[test]
fn enable_all_reports_partial_failure_and_keeps_the_healthy_member() {
    let fixture = BatchFixture::new();
    fixture.fail_package(SIGHT.1);

    let output = fixture.enable_all(&["--profile", "web", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));

    let json = envelope(&output);
    // A partial batch reports through `ok: false`, the exit code, and the
    // per-member statuses; like `install --all` it adds no error object.
    assert_eq!(json["ok"], false);
    assert!(json["error"].is_null(), "{json}");
    assert_eq!(json["data"]["total"], 2);
    assert_eq!(json["data"]["succeeded"], 1);
    assert_eq!(json["data"]["failed"], 1);
    let items = json["data"]["items"].as_array().expect("items");
    let failed = items
        .iter()
        .find(|item| item["component"] == SIGHT.0)
        .expect("failed member");
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("plugin add")),
        "{failed}"
    );

    // The healthy member is enabled. The failed member keeps a
    // `cleanup_failed` receipt rather than none: the driver may have
    // registered earlier profiles before failing, and only that receipt can
    // drive the cleanup retry.
    assert_eq!(
        fixture.receipt_status(TOKENLESS.0),
        Some(ClaimStatus::Enabled)
    );
    assert_eq!(
        fixture.receipt_status(SIGHT.0),
        Some(ClaimStatus::CleanupFailed)
    );
}

#[test]
fn enable_all_without_explicit_profiles_fails_every_member() {
    let fixture = BatchFixture::new();
    let output = fixture.enable_all(&["--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));

    let json = envelope(&output);
    assert_eq!(json["data"]["failed"], 2);
    assert_eq!(json["data"]["succeeded"], 0);
    for item in json["data"]["items"].as_array().expect("items") {
        assert_eq!(item["status"], "failed", "{item}");
        assert!(
            item["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("at least one explicit --profile")),
            "the driver's profile requirement must stay visible: {item}"
        );
    }
    assert!(!fixture.has_receipt(TOKENLESS.0));
    assert!(!fixture.has_receipt(SIGHT.0));
}

#[test]
fn enable_all_rejects_a_framework_no_component_declares() {
    let fixture = BatchFixture::new();
    let output = fixture.run(
        &["--json"],
        &["enable", "--all", "hermes", "--profile", "web"],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));

    let json = envelope(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        json["error"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no installed component declares")),
        "{json}"
    );
    assert!(
        fixture.dsh_invocations().is_empty(),
        "a rejected batch must not reach the framework CLI"
    );
}

#[test]
fn disable_all_removes_every_receipt_and_the_second_run_is_a_noop() {
    let fixture = BatchFixture::new();
    assert_success(&fixture.enable_all(&["--profile", "web"]));

    let output = fixture.run(&["--json"], &["disable", "--all", "dsh"]);
    assert_success(&output);
    let json = envelope(&output);
    assert_eq!(json["data"]["total"], 2);
    assert_eq!(json["data"]["failed"], 0);
    for item in json["data"]["items"].as_array().expect("items") {
        assert_eq!(item["status"], "disabled", "{item}");
    }
    assert!(!fixture.has_receipt(TOKENLESS.0));
    assert!(!fixture.has_receipt(SIGHT.0));
    assert!(
        !fixture.profile_manifest("web").contains(TOKENLESS.1),
        "the framework registry must drop the package"
    );

    // Idempotent: nothing enabled means nothing to do, not an error.
    let again = fixture.run(&[], &["disable", "--all", "dsh"]);
    assert_success(&again);
    assert!(
        stdout(&again).contains("nothing to disable"),
        "{}",
        stdout(&again)
    );
}

#[test]
fn status_framework_filter_narrows_the_receipt_set() {
    let fixture = BatchFixture::new();
    assert_success(&fixture.enable_all(&["--profile", "web"]));

    let dsh = fixture.run(&["--json"], &["status", "--framework", "dsh"]);
    assert_success(&dsh);
    let receipts = envelope(&dsh)["data"]["receipts"].clone();
    let receipts = receipts.as_array().expect("receipts");
    assert_eq!(receipts.len(), 2, "{receipts:?}");
    assert!(
        receipts.iter().all(|row| row["framework"] == "dsh"),
        "{receipts:?}"
    );

    let other = fixture.run(&["--json"], &["status", "--framework", "openclaw"]);
    assert_success(&other);
    assert_eq!(
        envelope(&other)["data"]["receipts"],
        serde_json::json!([]),
        "a filter with no match must return an empty list"
    );

    let human = fixture.run(&[], &["status", "--framework", "openclaw"]);
    assert_success(&human);
    assert!(
        stdout(&human).contains("No adapter receipts for framework 'openclaw'."),
        "{}",
        stdout(&human)
    );
}

#[test]
fn batch_arguments_reject_a_missing_framework_or_a_mixed_target() {
    let fixture = BatchFixture::new();

    // `--all` carries the framework, so omitting its value is a usage error.
    let missing_framework = fixture.run(&[], &["enable", "--all", "--profile", "web"]);
    assert_eq!(missing_framework.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&missing_framework.stderr);
    assert!(stderr.contains("--all"), "{stderr}");

    // A named component and a framework-wide batch are mutually exclusive.
    let conflicting = fixture.run(
        &[],
        &["enable", TOKENLESS.0, "--all", "dsh", "--profile", "web"],
    );
    assert_eq!(conflicting.status.code(), Some(2));

    // Neither a component nor `--all`: there is nothing to act on.
    let missing_target = fixture.run(&[], &["enable", "--profile", "web"]);
    assert_eq!(missing_target.status.code(), Some(2));
}
