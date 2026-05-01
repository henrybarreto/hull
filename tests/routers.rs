//! Router flow snapshot tests.

mod common;

use anyhow::Result;

use common::TestFlowsHarness;

#[tokio::test]
async fn test_router_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-create").await?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).await?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).await?;
    harness.router_ops.create("test-rt").await?;

    harness.assert("router_create")?;
    Ok(())
}

#[tokio::test]
async fn test_router_attach_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-attach").await?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).await?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).await?;
    harness.router_ops.create("test-rt").await?;
    harness.router_ops.attach("test-rt", "sw-a").await?;
    harness.router_ops.attach("test-rt", "sw-b").await?;

    harness.assert("router_attach")?;
    Ok(())
}

#[tokio::test]
async fn test_router_detach_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-detach").await?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).await?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).await?;
    harness.router_ops.create("test-rt").await?;
    harness.router_ops.attach("test-rt", "sw-a").await?;
    harness.router_ops.attach("test-rt", "sw-b").await?;
    harness.router_ops.detach("test-rt", "sw-b").await?;

    harness.assert("router_detach")?;
    Ok(())
}

#[tokio::test]
async fn test_router_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("router-remove").await?;

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).await?;
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).await?;
    harness.router_ops.create("test-rt").await?;
    harness.router_ops.attach("test-rt", "sw-a").await?;
    harness.router_ops.attach("test-rt", "sw-b").await?;
    harness.router_ops.remove("test-rt").await?;

    harness.assert("router_remove")?;
    Ok(())
}
