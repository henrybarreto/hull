mod common;

use common::TestFlowsHarness;

#[test]
fn test_router_create_flows() {
    let harness = TestFlowsHarness::new("router-create");

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).unwrap();
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).unwrap();
    harness.router_ops.create("test-rt").unwrap();

    harness.assert("router_create");
}

#[test]
fn test_router_attach_flows() {
    let harness = TestFlowsHarness::new("router-attach");

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).unwrap();
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).unwrap();
    harness.router_ops.create("test-rt").unwrap();
    harness.router_ops.attach("test-rt", "sw-a").unwrap();
    harness.router_ops.attach("test-rt", "sw-b").unwrap();

    harness.assert("router_attach");
}

#[test]
fn test_router_detach_flows() {
    let harness = TestFlowsHarness::new("router-detach");

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).unwrap();
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).unwrap();
    harness.router_ops.create("test-rt").unwrap();
    harness.router_ops.attach("test-rt", "sw-a").unwrap();
    harness.router_ops.attach("test-rt", "sw-b").unwrap();
    harness.router_ops.detach("test-rt", "sw-b").unwrap();

    harness.assert("router_detach");
}

#[test]
fn test_router_remove_flows() {
    let harness = TestFlowsHarness::new("router-remove");

    harness.switch_ops.create("sw-a", "10.0.0.0", 24).unwrap();
    harness.switch_ops.create("sw-b", "10.1.0.0", 24).unwrap();
    harness.router_ops.create("test-rt").unwrap();
    harness.router_ops.attach("test-rt", "sw-a").unwrap();
    harness.router_ops.attach("test-rt", "sw-b").unwrap();
    harness.router_ops.remove("test-rt").unwrap();

    harness.assert("router_remove");
}
