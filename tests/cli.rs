mod common;

use common::{CliTestHarness, HullOutputExt};

#[test]
fn test_help_without_subcommand() {
    let harness = CliTestHarness::new();
    let output = harness.run(&[]);

    assert!(
        !output.status.success(),
        "expected hull with no subcommand to exit non-zero"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);
    assert!(
        combined.contains("Usage: hull"),
        "expected help output, got:\n{}",
        combined
    );
}

#[test]
fn test_init_and_deinit() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_interface_create() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_switch_create() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_switch_port_create() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])
        .assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_router_create() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])
        .assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_router_attach() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])
        .assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_sync() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])
        .assert_success();
    harness.run(&["sync"]).assert_success();
    harness.run(&["deinit"]).assert_success();
}

#[test]
fn test_list_commands() {
    let harness = CliTestHarness::new();
    harness.run(&["init"]).assert_success();
    harness
        .run(&["interface", "create", &harness.iface])
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])
        .assert_success();
    harness.run(&["sync"]).assert_success();
    harness.run(&["interface", "ls"]).assert_success();
    harness.run(&["switch", "ls"]).assert_success();
    harness.run(&["router", "ls"]).assert_success();
    harness.run(&["deinit"]).assert_success();
}
