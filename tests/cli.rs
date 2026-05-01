//! CLI integration tests.

mod common;

use std::fs;

use anyhow::Result;
use hull::config::Config;
use hull::interfaces::Interface;
use tokio::runtime::Runtime;

use common::{CliTestHarness, HullOutputExt};

#[test]
fn test_help_without_subcommand() -> Result<()> {
    let harness = CliTestHarness::new()?;
    let output = harness.run(&[])?;

    assert!(
        !output.status.success(),
        "expected hull with no subcommand to exit non-zero"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("Usage: hull"),
        "expected help output, got:\n{combined}"
    );

    Ok(())
}

#[test]
fn test_init_and_deinit() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_startup_reconciles_missing_interfaces() -> Result<()> {
    let mut harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();

    harness.stop_daemon();
    let _ = std::process::Command::new("ip")
        .args(["link", "delete", &harness.iface])
        .status();
    harness.start_daemon()?;

    let rt = Runtime::new()?;
    assert!(
        rt.block_on(Interface::exists(&harness.iface)),
        "expected hulld startup to recreate missing interface"
    );

    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_init_honors_client_bridge_override() -> Result<()> {
    let harness = CliTestHarness::new()?;
    let client_bridge = format!("{}-client", harness.bridge);

    harness
        .run_with_bridge(&client_bridge, &["init"])?
        .assert_success();

    let config_path = harness.root.join("hull.json");
    let config = Config::load(&config_path)?;
    assert_eq!(config.bridge_name, client_bridge);

    harness.run(&["deinit"])?.assert_success();
    assert!(
        fs::metadata(&config_path).is_err(),
        "expected config to be removed"
    );

    Ok(())
}

#[test]
fn test_interface_create() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_switch_create() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_switch_port_create() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_router_create() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])?
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_router_attach() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])?
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])?
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_sync() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])?
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])?
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])?
        .assert_success();
    harness.run(&["sync"])?.assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_list_commands() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["interface", "create", &harness.iface])?
        .assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&[
            "switch",
            "port",
            "create",
            &harness.switch,
            &harness.port,
            &harness.iface,
        ])?
        .assert_success();
    harness
        .run(&["router", "create", &harness.router])?
        .assert_success();
    harness
        .run(&["router", "attach", &harness.router, &harness.switch])?
        .assert_success();
    harness.run(&["sync"])?.assert_success();
    harness.run(&["interface", "ls"])?.assert_success();
    harness.run(&["switch", "ls"])?.assert_success();
    harness.run(&["router", "ls"])?.assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}
