mod common;

use common::TestFlowsHarness;

#[test]
fn test_switch_create_flows() {
    let harness = TestFlowsHarness::new("switch-create");
    harness
        .switch_ops
        .create("test-sw", "10.0.0.0", 24)
        .unwrap();
    harness.assert("switch_create");
}

#[test]
fn test_switch_port_create_flows() {
    let harness = TestFlowsHarness::new("switch-port-create");
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap).unwrap();
    harness
        .switch_ops
        .create("test-sw", "10.0.0.0", 24)
        .unwrap();
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)
        .unwrap();

    harness.assert("switch_port_create");
}

#[test]
fn test_switch_port_remove_flows() {
    let harness = TestFlowsHarness::new("switch-port-remove");
    let tap = format!("tap-{}", harness.suffix);
    let port = format!("port-{}", harness.suffix);

    harness.interface_ops.create(&tap).unwrap();
    harness
        .switch_ops
        .create("test-sw", "10.0.0.0", 24)
        .unwrap();
    harness
        .switch_ops
        .create_switch_port(&port, "test-sw", &tap)
        .unwrap();
    harness
        .switch_ops
        .remove_switch_port("test-sw", &port)
        .unwrap();

    harness.assert("switch_port_remove");
}

#[test]
fn test_switch_remove_flows() {
    let harness = TestFlowsHarness::new("switch-remove");
    harness
        .switch_ops
        .create("test-sw", "10.0.0.0", 24)
        .unwrap();
    harness.switch_ops.remove("test-sw").unwrap();

    harness.assert("switch_remove");
}
