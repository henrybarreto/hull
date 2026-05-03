//! Router flow snapshot tests.

mod common;

use anyhow::Result;

use common::TestFlowsHarness;

#[test]
fn test_router_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-create")?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24)?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24)?;
    harness.router_ops.create("test-rt")?;

    harness.assert("router_create")?;
    Ok(())
}

#[test]
fn test_router_attach_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-attach")?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24)?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24)?;
    harness.router_ops.create("test-rt")?;
    harness.router_ops.attach("test-rt", "sw-a")?;
    harness.router_ops.attach("test-rt", "sw-b")?;

    harness.assert("router_attach")?;
    Ok(())
}

#[test]
fn test_router_detach_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-detach")?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24)?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24)?;
    harness.router_ops.create("test-rt")?;
    harness.router_ops.attach("test-rt", "sw-a")?;
    harness.router_ops.attach("test-rt", "sw-b")?;
    harness.router_ops.detach("test-rt", "sw-b")?;

    harness.assert("router_detach")?;
    Ok(())
}

#[test]
fn test_router_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-remove")?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24)?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24)?;
    harness.router_ops.create("test-rt")?;
    harness.router_ops.attach("test-rt", "sw-a")?;
    harness.router_ops.attach("test-rt", "sw-b")?;
    harness.router_ops.remove("test-rt")?;

    harness.assert("router_remove")?;
    Ok(())
}
