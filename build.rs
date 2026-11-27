//! Links a Windows version resource into `native_oidc.dll` (Issue #258).
//!
//! For `*-windows-msvc` targets, writes the resource as a compiled `.res`
//! file to `OUT_DIR` and passes it to the linker of the cdylib only; the
//! rlib, the staticlib, and the test binaries are unchanged. Every other
//! target is left as it is. The encoder is in
//! `build_support/windows_version_resource.rs`, which
//! `tests/windows_version_resource.rs` also checks.

use std::env;
use std::fs;
use std::path::PathBuf;

#[path = "build_support/windows_version_resource.rs"]
mod windows_version_resource;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/windows_version_resource.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    let crate_version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is set by Cargo");
    let version = [
        version_part("CARGO_PKG_VERSION_MAJOR"),
        version_part("CARGO_PKG_VERSION_MINOR"),
        version_part("CARGO_PKG_VERSION_PATCH"),
        0,
    ];
    let strings = windows_version_resource::native_oidc_strings(&crate_version);
    let res = windows_version_resource::encode_res(version, &strings);

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let path = out_dir.join("native_oidc.res");
    fs::write(&path, res).expect("write the Windows version resource");
    // link.exe and lld-link take a .res file as an input and convert it
    // into the image's resource section.
    println!("cargo:rustc-link-arg-cdylib={}", path.display());
}

/// One numeric part of the crate version. A Windows version field holds
/// 16 bits, so a larger part fails the build rather than wrap.
fn version_part(name: &str) -> u16 {
    let value = env::var(name).unwrap_or_else(|_| panic!("{name} is set by Cargo"));
    value
        .parse()
        .unwrap_or_else(|_| panic!("{name}={value} does not fit a Windows version field (0-65535)"))
}
