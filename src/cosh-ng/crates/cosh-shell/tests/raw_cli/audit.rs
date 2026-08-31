use super::*;

#[test]
fn raw_cli_audit_status_is_bounded_and_restores_prompt() {
    let directory = std::env::temp_dir().join(format!(
        "cosh-shell-raw-audit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&directory).expect("create audit fixture directory");
    let cli = directory.join("cosh-audit");
    fs::write(
        &cli,
        "#!/bin/sh\nprintf '%s' '{\"ok\":true,\"data\":{\"mode\":\"best_effort\",\"root_source\":\"environment\"}}'\n",
    )
    .expect("write cosh-audit fixture");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).expect("mode fixture");

    let output = run_raw_cli_with_env(
        "fake",
        "/audit status\necho after-audit-status\nexit\n",
        &[("COSH_AUDIT_BIN", cli.to_str().expect("utf8 fixture path"))],
    );

    assert!(output.contains("Audit"), "{output}");
    assert!(output.contains("best_effort"), "{output}");
    assert!(output.contains("after-audit-status"), "{output}");
    assert!(!output.contains("bash: /audit"), "{output}");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn raw_cli_audit_export_forwards_a_destination_containing_spaces() {
    let directory = std::env::temp_dir().join(format!(
        "cosh-shell-raw-audit-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&directory).expect("create audit fixture directory");
    let argv = directory.join("argv");
    let cli = directory.join("cosh-audit");
    fs::write(
        &cli,
        format!(
            "#!/bin/sh\nfor argument in \"$@\"; do printf '%s\\n' \"$argument\" >>'{}'; done\nprintf '%s' '{{\"ok\":true,\"data\":{{\"bundle\":\"written\"}}}}'\n",
            argv.display()
        ),
    )
    .expect("write cosh-audit fixture");
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).expect("mode fixture");

    let output = run_raw_cli_with_env(
        "fake",
        "/audit export current /tmp/my dir\necho after-audit-export\nexit\n",
        &[("COSH_AUDIT_BIN", cli.to_str().expect("utf8 fixture path"))],
    );

    let forwarded = fs::read_to_string(&argv).expect("cosh-audit fixture recorded argv");
    let forwarded = forwarded.lines().collect::<Vec<_>>();
    assert_eq!(forwarded.len(), 5, "{forwarded:?}");
    assert_eq!(forwarded[..3], ["export", "--output", "/tmp/my dir"]);
    assert_eq!(forwarded[3], "--identity");
    assert!(output.contains("written"), "{output}");
    assert!(output.contains("after-audit-export"), "{output}");
    let _ = fs::remove_dir_all(directory);
}
