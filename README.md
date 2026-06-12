# Hull

![rust edition](https://img.shields.io/badge/rust-2024-black)
![status](https://img.shields.io/badge/status-experimental-orange)

Hull is a single-node SDN controller.

## Architecture

Hull uses a client-daemon architecture:

- `hull` (client): sends JSON commands over a Unix socket
- `hulld` (daemon): owns SQLite state, TAP lifecycle, TC eBPF attach, and sync

The daemon must run as root.

## Build

Build both binaries:

```bash
make build
```

## Quick Start

1. Start the daemon:

```bash
sudo ./target/debug/hulld
```

2. Initialize:

```bash
./target/debug/hull init
```

3. Create a switch:

```bash
./target/debug/hull switch create sw0 10.0.0.0 24
```

4. Add switch ports (TAPs are managed automatically):

```bash
./target/debug/hull switch port create sw0 swp0
./target/debug/hull switch port create sw0 swp1
```

Optional static addressing:

```bash
./target/debug/hull switch port create sw0 swp0 --ip 10.0.0.10 --mac 52:54:00:12:34:56
```

Unknown L2 destinations are not flooded; forwarding only uses known MAC/router state.

## Paths

Hull uses environment variables to locate runtime state:

- `HULL_PATH` — root directory for the database and socket
- `HULL_SOCKET` — override the daemon socket path

Default paths (when `HULL_PATH` is not set):
- Data: `$XDG_DATA_HOME/hull` or `/var/lib/hull`

## CLI

See commands:

```bash
./target/debug/hull --help
```

## License

MIT License. See [LICENSE](LICENSE) for details.
