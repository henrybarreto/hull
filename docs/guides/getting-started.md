# Hull Getting Started

## Requirements

- Linux host
- Root access for `hulld`
- `iproute2` tools (`ip`, `tc`)

## Build

```bash
make build
```

## Start daemon

```bash
sudo ./target/debug/hulld
```

## Initialize state

```bash
./target/debug/hull init
```

## Create topology

```bash
./target/debug/hull switch create sw0 10.0.0.0 24
./target/debug/hull switch port create sw0 swp0
./target/debug/hull switch port create sw0 swp1
```

## Program dataplane

```bash
./target/debug/hull sync
```

## Inspect state

```bash
./target/debug/hull switch ls
./target/debug/hull switch port ls
```

## Teardown

```bash
./target/debug/hull deinit
```
