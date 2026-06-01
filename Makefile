.PHONY: help build-ebpf build release clean

EBPF_DIR := ebpf
RUST_NIGHTLY ?= +nightly
WORKSPACE_TARGET_DIR := $(abspath target)
EBPF_TARGET_DIR := $(abspath $(EBPF_DIR)/target)

help:
	@echo "Targets:"
	@echo "  make build-ebpf   Build eBPF object (nightly) to local ebpf/target"
	@echo "  make build        Build daemon (hulld) and client (hull)"
	@echo "                    (daemon build.rs builds eBPF locally in ebpf/target)"
	@echo "  make release      Build all components in release mode"
	@echo "  make clean        Clean all artifacts"

build-ebpf:
	CARGO_TARGET_DIR=$(EBPF_TARGET_DIR) cargo $(RUST_NIGHTLY) build --manifest-path $(EBPF_DIR)/Cargo.toml --release --target bpfel-unknown-none -Z build-std=core

build-ebpf-release:
	CARGO_TARGET_DIR=$(EBPF_TARGET_DIR) cargo $(RUST_NIGHTLY) build --manifest-path $(EBPF_DIR)/Cargo.toml --release --target bpfel-unknown-none -Z build-std=core

build:
	CARGO_TARGET_DIR=$(WORKSPACE_TARGET_DIR) cargo build -p daemon
	CARGO_TARGET_DIR=$(WORKSPACE_TARGET_DIR) cargo build -p client

release:
	CARGO_TARGET_DIR=$(WORKSPACE_TARGET_DIR) cargo build --release -p daemon
	CARGO_TARGET_DIR=$(WORKSPACE_TARGET_DIR) cargo build --release -p client

clean:
	CARGO_TARGET_DIR=$(WORKSPACE_TARGET_DIR) cargo clean
	CARGO_TARGET_DIR=$(EBPF_TARGET_DIR) cargo clean --manifest-path $(EBPF_DIR)/Cargo.toml
