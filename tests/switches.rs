//! Switch flow snapshot tests.

mod common;

use anyhow::Result;

use common::TestFlowsHarness;

#[tokio::test]
async fn test_switch_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-create").await?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24).await?;
    harness.assert("switch_create")?;
    Ok(())
}

#[tokio::test]
async fn test_switch_port_create_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-port-create").await?;
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap, None).await?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24).await?;
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)
        .await?;

    harness.assert("switch_port_create")?;
    Ok(())
}

#[tokio::test]
async fn test_switch_port_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-port-remove").await?;
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap, None).await?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24).await?;
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)
        .await?;
    harness
        .switch_ops
        .remove_switch_port("test-sw", &port)
        .await?;

    harness.assert("switch_port_remove")?;
    Ok(())
}

#[tokio::test]
async fn test_switch_remove_flows() -> Result<()> {
    let harness = TestFlowsHarness::new("switch-remove").await?;
    harness.switch_ops.create("test-sw", "10.0.0.0", 24).await?;
    harness.switch_ops.remove("test-sw").await?;

    harness.assert("switch_remove")?;
    Ok(())
}
