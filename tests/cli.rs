//! CLI integration tests for the switch/switch-port model.

mod common;

use anyhow::Result;
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
fn test_switch_lifecycle() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness.run(&["switch", "ls"])?.assert_success();
    harness
        .run(&["switch", "rm", &harness.switch])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_switch_port_lifecycle_and_sync() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&["switch", "port", "create", &harness.switch, &harness.port])?
        .assert_success();
    harness.run(&["switch", "port", "ls"])?.assert_success();
    harness.run(&["switch", "ls"])?.assert_success();
    harness.run(&["sync"])?.assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_switch_create_defaults_l3_fields() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.10.0.0", "24"])?
        .assert_success();
    harness.run(&["switch", "ls"])?.assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}

#[test]
fn test_switch_port_remove() -> Result<()> {
    let harness = CliTestHarness::new()?;
    harness.run(&["init"])?.assert_success();
    harness
        .run(&["switch", "create", &harness.switch, "10.0.0.0", "24"])?
        .assert_success();
    harness
        .run(&["switch", "port", "create", &harness.switch, &harness.port])?
        .assert_success();
    harness
        .run(&["switch", "port", "rm", &harness.switch, &harness.port])?
        .assert_success();
    harness.run(&["deinit"])?.assert_success();
    Ok(())
}
