//! Switch flow snapshot tests.

mod common;

use anyhow::Result;

use common::TestFlowsHarness;

#[test]
fn test_switch_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-create")?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24)?;
    harness.assert("switch_create")?;
    Ok(())
}

#[test]
fn test_switch_port_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-port-create")?;
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap, None)?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24)?;
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)?;

    harness.assert("switch_port_create")?;
    Ok(())
}

#[test]
fn test_switch_port_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-port-remove")?;
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap, None)?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24)?;
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)?;
    harness.switch_ops.remove_switch_port("test-sw", &port)?;

    harness.assert("switch_port_remove")?;
    Ok(())
}

#[test]
fn test_switch_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-remove")?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24)?;
    harness.switch_ops.remove("test-sw")?;

    harness.assert("switch_remove")?;
    Ok(())
}
