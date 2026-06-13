//! Single-source-of-truth binary resolver for e2e tests.
//!
//! Resolution order:
//!   1. `$TYU_BIN_DIR/<name>` — CI sets this to `$GITHUB_WORKSPACE/target/release`
//!      so tests consume the same artifact that CI built.
//!   2. `target/debug/<name>` — local debug build (user runs `cargo build` first).
//!   3. `target/release/<name>` — local release build.
//!
//! The build step happens outside tests (CI pipeline or `cargo build`).  This
//! resolver never spawns cargo itself, avoiding the target-directory lock
//! contention between the parent `cargo test` and a child `cargo build`.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn profile_dir() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Resolve a workspace binary for e2e use.
pub fn resolve(name: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("TYU_BIN_DIR") {
        let raw = PathBuf::from(&dir);
        let p = if raw.is_relative() {
            workspace_root().join(&raw).join(name)
        } else {
            raw.join(name)
        };
        assert!(
            p.is_file(),
            "TYU_BIN_DIR={dir} set but {name} not found at {p:?} \
             (resolved from workspace root {})",
            workspace_root().display()
        );
        return p;
    }
    // Check the active profile directory first, then the other.
    let root = workspace_root();
    let p = root.join("target").join(profile_dir()).join(name);
    if p.is_file() {
        return p;
    }
    let other_dir = if cfg!(debug_assertions) {
        "release"
    } else {
        "debug"
    };
    let p = root.join("target").join(other_dir).join(name);
    assert!(
        p.is_file(),
        "{name} not found. Run `cargo build -p {name}` first."
    );
    p
}
