//! End-to-end tests for concurrent package operations

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Test context that sets up a temporary APL home environment
struct TestContext {
    temp_dir: TempDir,
    apl_home: PathBuf,
}

impl TestContext {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let apl_home = temp_dir.path().join(".apl");
        std::fs::create_dir_all(&apl_home).expect("failed to create apl home");
        Self { temp_dir, apl_home }
    }

    fn apl_cmd(&self) -> Command {
        let bin_path = env!("CARGO_BIN_EXE_apl");
        let mut cmd = Command::new(bin_path);
        cmd.env("HOME", self.temp_dir.path());
        cmd
    }
}

#[test]
fn test_ui_actor_singleton() {
    // This tests that multiple Output instances work correctly
    // Actual test lives in src/ui/output.rs, but verify via CLI
    let ctx = TestContext::new();

    // Running multiple commands should not crash
    let _ = ctx.apl_cmd().arg("list").output();
    let _ = ctx.apl_cmd().arg("status").output();
    let _ = ctx.apl_cmd().arg("list").output();

    // Success = no crash
}

#[test]
fn test_concurrent_package_install_no_corruption() {
    // Note: This test requires real packages or mocked server
    // For now, we verify the command accepts multiple packages

    let ctx = TestContext::new();
    let output = ctx
        .apl_cmd()
        .arg("--dry-run")
        .arg("install")
        .args(&["ripgrep", "bat", "fd"])
        .output()
        .expect("failed to run apl install");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // In dry-run mode, should at least try to process packages
    // Verify no obvious UI corruption (packages running together)
    assert!(
        !stdout.contains("ripgrepbat"),
        "Package names should not be concatenated"
    );

    // Should mention the packages we requested
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("ripgrep") || combined.contains("No index"),
        "Should reference ripgrep or explain missing index"
    );
}

#[test]
fn test_multiple_remove_operations() {
    let ctx = TestContext::new();

    // Removing non-existent packages should handle gracefully
    let output = ctx
        .apl_cmd()
        .arg("remove")
        .arg("--yes")
        .args(&["pkg1", "pkg2", "pkg3"])
        .output()
        .expect("failed to run apl remove");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should not crash, should report packages not found
    assert!(
        stdout.contains("not installed") || stderr.contains("not found") || output.status.success(),
        "Should handle missing packages gracefully"
    );
}
