//! Sysroot resolution for the tyu driver (PLAN-RELEASE-1 S5, FR-21).
//!
//! Resolution order: the explicit `--sysroot` flag, then the `TYU_SYSROOT`
//! environment variable, then the repository-anchored `cwd/sysroot`. The env
//! rung is the single step toward the relocatable future: a standalone
//! project outside a checkout can `export TYU_SYSROOT=$prefix/src/tyu/sysroot`
//! and build without the repo layout under the current directory.
//!
//! The env rung is honored even when the path does not exist — an explicit
//! override that points nowhere fails loudly at use time rather than
//! silently falling back to `cwd/sysroot` (which would mask the misconfig).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Resolve the sysroot for a driver invocation.
pub fn resolve(explicit: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    resolve_with_env(explicit, cwd, std::env::var_os("TYU_SYSROOT"))
}

/// Testable core: the env rung is injected so all three rungs and their
/// precedence are verifiable without mutating the process environment.
///
/// * `explicit`  — the `--sysroot=<dir>` flag value (wins over everything).
/// * `cwd`       — the driver's working directory (fallback anchor).
/// * `env_sysroot` — the `TYU_SYSROOT` value; empty counts as unset.
pub fn resolve_with_env(
    explicit: Option<&Path>,
    cwd: &Path,
    env_sysroot: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    match env_sysroot {
        Some(v) if !v.is_empty() => return Some(PathBuf::from(v)),
        _ => {}
    }
    let candidate = cwd.join("sysroot");
    candidate.is_dir().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique temp dir with a `sysroot/` subdir (mirrors a checkout).
    fn temp_cwd_with_sysroot(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        fs::create_dir_all(dir.join("sysroot")).unwrap();
        dir
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "tyu-sysroot-{}-{}-{:?}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn flag_beats_env_beats_cwd() {
        // temp_cwd_with_sysroot exists, so a cwd fallback would also match.
        let cwd = temp_cwd_with_sysroot("flag");
        let flag = cwd.join("explicit-sysroot");
        let got = resolve_with_env(Some(&flag), &cwd, Some(OsString::from("/env/sysroot")));
        assert_eq!(got, Some(flag));
        fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn env_beats_cwd_when_no_flag() {
        let cwd = temp_cwd_with_sysroot("env");
        let got = resolve_with_env(None, &cwd, Some(OsString::from("/env/sysroot")));
        assert_eq!(got, Some(PathBuf::from("/env/sysroot")));
        fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn env_does_not_need_to_exist() {
        let cwd = temp_cwd_with_sysroot("env-missing");
        let missing = cwd.join("does-not-exist");
        let got = resolve_with_env(None, &cwd, Some(missing.clone().into_os_string()));
        assert_eq!(got, Some(missing));
        fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn empty_env_treated_as_unset() {
        let cwd = temp_cwd_with_sysroot("empty-env");
        let got = resolve_with_env(None, &cwd, Some(OsString::new()));
        assert_eq!(got, Some(cwd.join("sysroot")));
        fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn cwd_fallback_when_no_flag_and_no_env() {
        let cwd = temp_cwd_with_sysroot("fallback");
        let got = resolve_with_env(None, &cwd, None);
        assert_eq!(got, Some(cwd.join("sysroot")));
        fs::remove_dir_all(&cwd).unwrap();
    }

    #[test]
    fn unresolvable_is_none() {
        let cwd = temp_dir("unresolvable");
        fs::create_dir_all(&cwd).unwrap(); // no sysroot child
        let got = resolve_with_env(None, &cwd, None);
        assert_eq!(got, None);
        fs::remove_dir_all(&cwd).unwrap();
    }
}
