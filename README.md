# Hull

Hull is a simple utility tool that simplifies creating and managing virtual networks using Open vSwitch.

## Requirements

- rust
- ovs-vsctl, ovs-ofctl
- ip
- ping

## Installation

1. Clone this repository:

```sh
git clone https://github.com/henrybarreto/hull
```

2. Install the CLI tool using Cargo:

```sh
cargo install --path client
```

This installs the `hull` client from the `client` workspace member.

Install the daemon separately:

```sh
cargo install --path daemon
```

Start `hulld` first, then use `hull` from another terminal.

3. Test it:

```sh
hull --version
```

## Configuration & environment

Hull determines its on-disk layout and configuration file location from the
following sources (in precedence order):

- CLI `--config <path>` argument (highest precedence)
- `HULL_PATH` environment variable (root directory for Hull data)
- XDG data directory & defaults described below

Environment variables supported by the codebase:

- `HULL_PATH` — root directory used for images, instances, locks and the
  default `hull.json`. Resolution order when this is not set:
    1. `$XDG_DATA_HOME/hull` if `XDG_DATA_HOME` is set
  2. `/var/lib/hull`

The configuration file loaded is either the path passed via `--config` or
`{HULL_PATH}/hull.json` (or the default root described above). Use
`HULL_PATH` to keep Hull data in a custom workspace, or pass `--config` to load
an explicit JSON file.

## Example usage

1. Start the daemon:

```sh
sudo hulld
```

2. Initialize the Hull project:

```sh
hull init
```

3. Create a network switch:

```sh
hull switch create sw0 10.0.0.0 24
```

4. Create a network interface:

```sh
hull interface create tap0
```

5. Add a port to the switch:

```sh
hull switch port create port0 sw0 tap0
```

6. Create a router:

```sh
hull router create router0
```

7. Attach the switch to the router:

```sh
hull router attach router0 sw0
```

Now, every VM connected to `sw0` can communicate with each other and the router.
You can further configure the router to enable external connectivity by setting
a link interface.

# License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
