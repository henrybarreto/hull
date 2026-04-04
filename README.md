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
cargo install --path .
```

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
  default `hull.yaml`. Resolution order when this is not set:
  1. `$XDG_DATA_HOME/hull` if `XDG_DATA_HOME` is set
  2. `/var/lib/hull` when running as root
  3. `$HOME/.local/share/hull` for a regular user

The configuration file loaded is either the path passed via `--config` or
`{HULL_PATH}/hull.yaml` (or the default root described above). Use
`HULL_PATH` to keep Hull data in a custom workspace, or pass `--config` to load
an explicit YAML file.

## Example usage

1. Initialize hull project:

```sh
sudo hull init
```

2. Create a network switch:

```sh
sudo hull switch create sw0 10.0.0.0 24
```

3. Create a network interface:

```sh
sudo hull interface create tap0
```

4. Add a port to the switch:

```sh
sudo hull switch port create port0 sw0 tap0
```

5. Create a router:

```sh
sudo hull router create router0
```

6. Attach the switch to the router:

```sh
sudo hull router attach router0 sw0
```

7. Start a VM with the created network interface:

```sh
 qemu-system-x86_64 \
  -enable-kvm \
  -m 1G \
  -cpu host \
  -drive file=overlay.qcow2,format=qcow2 \
  -drive file=seed.iso,format=raw \
  -nic tap,ifname=tap0,script=no,downscript=no,mac=<tap0-mac> \
  -nographic
```

Now, every VM connected to `sw0` can communicate with each other and the router.
You can further configure the router to enable external connectivity by setting
a link interface.


# License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
