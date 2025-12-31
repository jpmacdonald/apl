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

        // Mock index if needed?
        // For now, we assume we want to test against real world or mocked index.
        // If we want isolated tests, we should probably generate a dummy index using our own tools.
        // But for "Integration", let's start with basic help/version to verify binary launches.

        Self { temp_dir, apl_home }
    }

    fn apl_cmd(&self) -> Command {
        // Find the binary built by cargo
        let bin_path = env!("CARGO_BIN_EXE_apl");
        let mut cmd = Command::new(bin_path);
        cmd.env("HOME", self.temp_dir.path());
        cmd.env("APL_HOME", &self.apl_home); // If strict override supported, otherwise HOME is enough
        cmd
    }
}

#[test]
fn test_help_command() {
    let ctx = TestContext::new();
    let output = ctx
        .apl_cmd()
        .arg("--help")
        .output()
        .expect("failed to run apl");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
}

#[test]
fn test_version_command() {
    let ctx = TestContext::new();
    let output = ctx
        .apl_cmd()
        .arg("--version")
        .output()
        .expect("failed to run apl");
    assert!(output.status.success());
}

#[test]
fn test_init_creates_state_db() {
    let ctx = TestContext::new();
    // Running list should trigger DB init if not present
    let output = ctx
        .apl_cmd()
        .arg("list")
        .output()
        .expect("failed to run apl");
    assert!(output.status.success());

    let db_path = ctx.apl_home.join("state.db");
    assert!(
        db_path.exists(),
        "state.db should be created after running list"
    );
}

// TODO: Add test for 'install' using a mocked index/server once we have that infrastructure.
