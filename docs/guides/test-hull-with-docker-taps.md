# Manual Hull Test with Docker and TAPs

This guide shows a simple manual test for Hull without virtual machines.
It uses a root-owned `hulld` daemon to create host TAP interfaces and a rootless
`hull` client to manage the topology. Docker containers in the host network
namespace generate traffic.

## What This Tests

- Hull creates TAP interfaces on the host
- Hull attaches those TAPs to an OVS bridge
- OVS forwards traffic between the TAP-backed ports
- L2 ARP and ICMP work end to end

## Prerequisites

- `hull`, `hulld`, `ovs-vsctl`, `ovs-ofctl`, `ip`, and `docker` installed
- Root access for `hulld` and host interface configuration
- Docker running on the host

## Step 1: Create a Small Topology

Start the daemon, then create two TAP interfaces, one switch, and two ports:

```sh
sudo hulld
hull init
hull interface create tap0
hull interface create tap1
hull switch create sw0 10.0.0.0 24
hull switch port create p0 sw0 tap0
hull switch port create p1 sw0 tap1
```

Verify Hull recorded the port state:

```sh
hull switch port ls
```

## Step 2: Put IPs on the TAPs

Assign IP addresses on the host and bring the TAPs up:

```sh
sudo ip addr add 10.0.0.2/24 dev tap0
sudo ip addr add 10.0.0.3/24 dev tap1
sudo ip link set tap0 up
sudo ip link set tap1 up
```

## Step 3: Generate Traffic from Docker

Run Docker containers with host networking so they can see the host TAPs:

```sh
sudo docker run --rm --network=host --cap-add=NET_RAW alpine ping -c 3 -I tap0 10.0.0.3
sudo docker run --rm --network=host --cap-add=NET_RAW alpine ping -c 3 -I tap1 10.0.0.2
```

Expected result:

- both commands complete successfully
- ARP resolves for both TAPs
- packets traverse the Hull-managed OVS bridge

## Step 4: Inspect the Flows

Confirm the switch flows exist on the bridge:

```sh
sudo ovs-ofctl dump-flows hull0
```

If you set `HULL_BRIDGE`, replace `hull0` with that value.

## Step 5: Clean Up

Remove the topology and clear the host TAP configuration:

```sh
hull deinit
sudo ip addr del 10.0.0.2/24 dev tap0
sudo ip addr del 10.0.0.3/24 dev tap1
```

If `hull deinit` already removed the TAPs, the `ip addr del` commands may fail
harmlessly.

## Notes

- This is a host-network test, not isolated container networking.
- Docker is only used as a root traffic generator here.
- If you need isolated namespace endpoints, Hull would need namespace-aware TAP
  support or an external step to move TAPs into the target namespace.
