//! Build script to derive version from git tags
//!
//! This allows the binary to report its version based on git tags,
//! so you don't need to manually sync Cargo.toml version with tags.

fn main() {
    // Rerun if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");

    // Try to get version from git describe
    let version = std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty=-dev"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_start_matches('v').to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    println!("cargo:rustc-env=APL_VERSION={}", version);
}
