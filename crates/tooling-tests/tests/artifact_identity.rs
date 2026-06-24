//! Identity proof: `TYU_BIN_DIR` override is authoritative.
//!
//! With `TYU_BIN_DIR` set to a directory containing a stub script that
//! prints a sentinel and exits with a distinctive code, `bin("langc")`
//! must return that stub and invoking it must surface the sentinel exit
//! code — proving the override is authoritative and not silently bypassed
//! by a self-build fallback.

use std::process::Command;

mod common;

#[test]
fn tyu_bin_dir_override_is_authoritative() {
    let dir = common::fresh_dir("identity_proof");

    // Create a stub script that prints a sentinel and exits with 209.
    let stub_path = dir.join("langc");
    let stub_script = "#!/bin/sh\necho 'TYU_BIN_DIR_PROOF'\nexit 209\n";
    std::fs::write(&stub_path, stub_script).unwrap();

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // Set TYU_BIN_DIR and invoke bin resolver
    std::env::set_var("TYU_BIN_DIR", dir.to_str().unwrap());
    let resolved = common::bin::resolve("langc");

    // The resolved path must point to our stub.
    assert_eq!(
        resolved, stub_path,
        "bin(\"langc\") must return the stub path when TYU_BIN_DIR is set"
    );

    // Running the stub must surface exit code 209.
    let output = Command::new(&resolved).output().expect("stub must execute");
    assert_eq!(
        output.status.code(),
        Some(209),
        "stub must exit with distinctive code 209"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("TYU_BIN_DIR_PROOF"),
        "stub must print sentinel, got: {stdout}"
    );

    std::env::remove_var("TYU_BIN_DIR");
}
