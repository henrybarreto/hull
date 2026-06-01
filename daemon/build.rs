use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .expect("daemon crate should live under workspace root");
    let ebpf_dir = workspace_root.join("ebpf");
    let ebpf_target_dir = ebpf_dir.join("target");
    let ebpf_object = ebpf_target_dir.join("bpfel-unknown-none/release/libebpf.so");

    println!(
        "cargo:rerun-if-changed={}",
        ebpf_dir.join("Cargo.toml").display()
    );
    println!("cargo:rerun-if-changed={}", ebpf_dir.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("common/src").display()
    );

    if env::var_os("HULL_SKIP_EBPF_BUILD").is_some() {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        let placeholder = out_dir.join("libebpf-placeholder.so");
        fs::write(&placeholder, []).expect("failed to write placeholder eBPF object");
        println!("cargo:rustc-env=HULL_EBPF_OBJECT={}", placeholder.display());
    } else {
        run_cargo_build(&ebpf_dir, &ebpf_target_dir);
        println!("cargo:rustc-env=HULL_EBPF_OBJECT={}", ebpf_object.display());
    }
}

fn run_cargo_build(ebpf_dir: &Path, ebpf_target_dir: &Path) {
    let status = Command::new("rustup")
        .args([
            "run",
            "nightly",
            "cargo",
            "build",
            "--release",
            "--target",
            "bpfel-unknown-none",
            "-Z",
            "build-std=core",
        ])
        .current_dir(ebpf_dir)
        .env("CARGO_TARGET_DIR", ebpf_target_dir)
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .env_remove("RUSTUP_TOOLCHAIN")
        .status()
        .expect("failed to start eBPF build command");

    assert!(
        status.success(),
        "failed to build eBPF object with rustup nightly cargo"
    );
}
