# Hull

![rust edition](https://img.shields.io/badge/rust-2024-black)
![status](https://img.shields.io/badge/status-experimental-orange)

Hull is a lightweight Open vSwitch (OVS) network management tool. It provides a
CLI for managing virtual networks with switches, routers, and network interfaces.

## Architecture

Hull uses a client-daemon architecture:

- **hull** (client) — CLI tool that sends commands to the daemon
- **hulld** (daemon) — Privileged backend that owns SQLite state, TAP interfaces, and OVS sync

The daemon must run as root to manage OVS bridges and TAP interfaces.

## Build

Build with Cargo:

```bash
cargo build --manifest-path client/Cargo.toml --bin hull
cargo build --manifest-path daemon/Cargo.toml --bin hulld
```

## Quick Start

1. Start the daemon (as root):

```bash
sudo ./daemon/target/debug/hulld
```

2. Initialize the project:

```bash
./client/target/debug/hull init
```

3. Create a switch:

```bash
./client/target/debug/hull switch create sw0 10.0.0.0/24
```

4. Create an interface:

```bash
./client/target/debug/hull interface create tap0
```

5. Add port to switch:

```bash
./client/target/debug/hull switch port create port0 sw0 tap0
```

6. Create a router:

```bash
./client/target/debug/hull router create router0
```

7. Attach switch to router:

```bash
./client/target/debug/hull router attach router0 sw0
```

## Configuration

Hull uses environment variables to locate its data directory:

- `HULL_PATH` — root directory for Hull data (config, database, socket)
- `HULL_BRIDGE` — override the OVS bridge name (default: `hull0`)
- `HULL_SOCKET` — override the daemon socket path

Default paths (when `HULL_PATH` is not set):
- Data: `$XDG_DATA_HOME/hull` or `/var/lib/hull`
- Config: `{HULL_PATH}/hull.json`

## CLI

To know more about the CLI commands, run:

```bash
bash./client/target/debug/hulld --help
```

or

```bash
bash./client/target/debug/hull --help
```


## License

MIT License. See [LICENSE](LICENSE) for details.
