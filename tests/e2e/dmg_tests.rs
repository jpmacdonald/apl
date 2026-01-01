//! DMG handling end-to-end tests

use std::path::Path;

#[test]
fn test_dmg_attach_nonexistent_file() {
    use apl::io::dmg;

    let result = dmg::attach(Path::new("/tmp/does-not-exist-12345.dmg"));
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found"),
        "Error should mention file not found, got: {}",
        err_msg
    );
}

#[test]
fn test_dmg_detach_nonexistent_volume() {
    use apl::io::dmg;

    let result = dmg::detach(Path::new("/Volumes/NonexistentTestVolume12345"));
    assert!(result.is_err());

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Failed to detach"),
        "Error should mention detach failure, got: {}",
        err_msg
    );
}

// Note: Testing actual DMG mounting requires a real DMG file
// Commented out for now, but shows how to test when fixture is available
/*
#[test]
fn test_dmg_mount_unmount_cycle() {
    use apl::io::dmg;

    let dmg_path = Path::new("tests/fixtures/test.dmg");

    if !dmg_path.exists() {
        eprintln!("Skipping DMG test - fixture not found");
        eprintln!("To enable: hdiutil create -size 1m -volname TestVol -fs HFS+ tests/fixtures/test.dmg");
        return;
    }

    // Attach
    let mount = dmg::attach(dmg_path).expect("Should attach test DMG");
    assert!(mount.path.exists(), "Mount point should exist");
    assert!(mount.path.to_string_lossy().contains("/Volumes/"));

    // Drop should auto-detach
    drop(mount);

    // Give filesystem time to unmount
    std::thread::sleep(std::time::Duration::from_millis(200));
}
*/
