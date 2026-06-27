//! Secure key material abstraction.
//!
//! Provides safe key sourcing (`file:`, `env:`, `fd:`) and rejects
//! bare-hex `argv` keys with a diagnostic that names the safe alternatives.
//! Key buffers are zeroized on drop.

use crate::error::TyuError;
use std::fs;
use std::io::Read;
use std::path::Path;

use zeroize::Zeroizing;

// ---------------------------------------------------------------------------
// KeyRef — parsed key reference string
// ---------------------------------------------------------------------------

/// A parsed key reference.
#[derive(Clone, Debug)]
pub enum KeyRef {
    /// `file:<path>` — read key from a file.
    File(std::path::PathBuf),
    /// `env:<VAR>` — read key from an environment variable.
    Env(String),
    /// `fd:<n>` — read key from a file descriptor number.
    Fd(u32),
}

impl KeyRef {
    /// Parse a key reference string.
    ///
    /// Accepted formats:
    ///   - `file:<path>`     — read from file at `path`
    ///   - `env:<VAR>`       — read from environment variable `VAR`
    ///   - `fd:<n>`          — read from file descriptor `n` (0 = stdin)
    ///
    /// A bare hex string (e.g. `--key=abab...`) is **rejected** with a
    /// diagnostic message naming the safe alternatives.
    pub fn parse(s: &str) -> Result<Self, TyuError> {
        if let Some(path) = s.strip_prefix("file:") {
            return Ok(KeyRef::File(std::path::PathBuf::from(path)));
        }
        if let Some(var) = s.strip_prefix("env:") {
            if var.is_empty() {
                return Err(TyuError::Key(
                    "empty environment variable name after 'env:'".into(),
                ));
            }
            return Ok(KeyRef::Env(var.to_string()));
        }
        if let Some(n_str) = s.strip_prefix("fd:") {
            let n: u32 = n_str.parse().map_err(|_| {
                TyuError::Key(format!(
                    "invalid file descriptor number '{}' after 'fd:'",
                    n_str
                ))
            })?;
            return Ok(KeyRef::Fd(n));
        }
        // If it has no recognized prefix, reject bare hex / raw string.
        Err(TyuError::Key(format!(
            "refusing to read key from command-line argument.\n  \
             Use one of:\n    \
             --key=file:<path>   (read key from a file)\n    \
             --key=env:<VAR>     (read key from environment variable)\n    \
             --key=fd:<n>        (read key from file descriptor)"
        )))
    }
}

// ---------------------------------------------------------------------------
// KeyMaterial — zeroizing key buffer
// ---------------------------------------------------------------------------

/// A secure key buffer that zeroizes its contents on drop.
#[derive(Clone, Debug)]
pub struct KeyMaterial(Zeroizing<Vec<u8>>);

impl KeyMaterial {
    /// Create a `KeyMaterial` from a byte slice (copies the data).
    pub fn new(bytes: &[u8]) -> Self {
        KeyMaterial(Zeroizing::new(bytes.to_vec()))
    }

    /// Create a `KeyMaterial` from a file path.  The file content is
    /// hex-decoded after trimming whitespace.
    pub fn from_file(path: &Path) -> Result<Self, TyuError> {
        let hex = fs::read_to_string(path)
            .map_err(|e| TyuError::Key(format!("reading key file '{}': {}", path.display(), e)))?;
        let bytes = hex::decode(hex.trim()).map_err(|e| {
            TyuError::Key(format!(
                "invalid hex in key file '{}': {}",
                path.display(),
                e
            ))
        })?;
        Ok(KeyMaterial::new(&bytes))
    }

    /// Create a `KeyMaterial` from an environment variable.
    /// The variable value is hex-decoded after trimming whitespace.
    pub fn from_env(var: &str) -> Result<Self, TyuError> {
        let hex = std::env::var(var)
            .map_err(|_| TyuError::Key(format!("environment variable '{}' not set", var)))?;
        let bytes = hex::decode(hex.trim())
            .map_err(|e| TyuError::Key(format!("invalid hex in env var '{}': {}", var, e)))?;
        Ok(KeyMaterial::new(&bytes))
    }

    /// Create a `KeyMaterial` from a file descriptor number.
    /// `fd:0` reads from stdin.  Reads all data until EOF, trims whitespace,
    /// and hex-decodes the result.
    pub fn from_fd(fd: u32) -> Result<Self, TyuError> {
        let mut hex = String::new();
        if fd == 0 {
            std::io::stdin()
                .read_to_string(&mut hex)
                .map_err(|e| TyuError::Key(format!("reading fd {}: {}", fd, e)))?;
        } else {
            let mut file = fs::File::open(format!("/proc/self/fd/{fd}"))
                .map_err(|e| TyuError::Key(format!("opening fd {}: {}", fd, e)))?;
            file.read_to_string(&mut hex)
                .map_err(|e| TyuError::Key(format!("reading fd {}: {}", fd, e)))?;
        }
        let bytes = hex::decode(hex.trim())
            .map_err(|e| TyuError::Key(format!("invalid hex from fd {}: {}", fd, e)))?;
        Ok(KeyMaterial::new(&bytes))
    }

    /// Resolve a `KeyRef` to a `KeyMaterial` by reading the key from
    /// the specified source.
    pub fn resolve(r: &KeyRef) -> Result<Self, TyuError> {
        match r {
            KeyRef::File(path) => KeyMaterial::from_file(path),
            KeyRef::Env(var) => KeyMaterial::from_env(var),
            KeyRef::Fd(n) => KeyMaterial::from_fd(*n),
        }
    }

    /// Borrow the key bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// If the key is exactly 32 bytes, return a reference to a `[u8; 32]`.
    pub fn try_as_32bytes(&self) -> Result<&[u8; 32], TyuError> {
        let slice: &[u8] = &self.0;
        <&[u8; 32]>::try_from(slice)
            .map_err(|_| TyuError::Key(format!("key must be 32 bytes, got {} bytes", slice.len())))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("tyu_keys_tests").join(format!(
            "{}_{}",
            label,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_file_ref() {
        let r = KeyRef::parse("file:/tmp/key.bin").unwrap();
        assert!(matches!(r, KeyRef::File(_)));
        if let KeyRef::File(p) = r {
            assert_eq!(p, std::path::PathBuf::from("/tmp/key.bin"));
        }
    }

    #[test]
    fn parse_env_ref() {
        let r = KeyRef::parse("env:MY_KEY").unwrap();
        assert!(matches!(r, KeyRef::Env(_)));
        if let KeyRef::Env(v) = r {
            assert_eq!(v, "MY_KEY");
        }
    }

    #[test]
    fn parse_fd_ref() {
        let r = KeyRef::parse("fd:3").unwrap();
        assert!(matches!(r, KeyRef::Fd(3)));
    }

    #[test]
    fn parse_fd_zero() {
        let r = KeyRef::parse("fd:0").unwrap();
        assert!(matches!(r, KeyRef::Fd(0)));
    }

    #[test]
    fn bare_hex_rejected() {
        let err = KeyRef::parse("abababababababababababababababab").unwrap_err();
        assert!(
            err.to_string().contains("refusing to read key"),
            "bare hex must be rejected"
        );
        assert!(
            err.to_string().contains("file:"),
            "error must mention file: alternative"
        );
        assert!(
            err.to_string().contains("env:"),
            "error must mention env: alternative"
        );
        assert!(
            err.to_string().contains("fd:"),
            "error must mention fd: alternative"
        );
    }

    #[test]
    fn empty_env_rejected() {
        let err = KeyRef::parse("env:").unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "empty env var name must be rejected"
        );
    }

    #[test]
    fn invalid_fd_rejected() {
        let err = KeyRef::parse("fd:xyz").unwrap_err();
        assert!(
            err.to_string().contains("invalid file descriptor"),
            "non-numeric fd must be rejected"
        );
    }

    #[test]
    fn no_prefix_rejected() {
        let err = KeyRef::parse("some-random-string").unwrap_err();
        assert!(err.to_string().contains("refusing to read key"));
    }

    #[test]
    fn key_material_from_file() {
        let dir = tmp_dir("from_file");
        let path = dir.join("key.bin");
        std::fs::write(&path, hex::encode([0xabu8; 32])).unwrap();
        let km = KeyMaterial::from_file(&path).unwrap();
        assert_eq!(km.as_bytes(), &[0xabu8; 32]);
    }

    #[test]
    fn key_material_from_env() {
        std::env::set_var("TYU_TEST_KEY", hex::encode([0xbbu8; 32]));
        let km = KeyMaterial::from_env("TYU_TEST_KEY").unwrap();
        assert_eq!(km.as_bytes(), &[0xbbu8; 32]);
        std::env::remove_var("TYU_TEST_KEY");
    }

    #[test]
    fn key_material_from_fd_does_not_take_ownership() {
        let dir = tmp_dir("from_fd");
        let path = dir.join("key.bin");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(hex::encode([0xddu8; 32]).as_bytes())
            .unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();

        let fd = file.as_raw_fd() as u32;
        let km = KeyMaterial::from_fd(fd).unwrap();

        assert_eq!(km.as_bytes(), &[0xddu8; 32]);
        file.seek(SeekFrom::End(0))
            .expect("caller-owned fd must remain open");
    }

    #[test]
    fn key_material_from_fd_rejects_short_key() {
        let dir = tmp_dir("from_fd_short");
        let path = dir.join("key.bin");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(hex::encode([0xeeu8; 16]).as_bytes())
            .unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();

        let fd = file.as_raw_fd() as u32;
        let km = KeyMaterial::from_fd(fd).unwrap();

        assert!(km.try_as_32bytes().is_err());
        file.seek(SeekFrom::End(0))
            .expect("caller-owned fd must remain open");
    }

    #[test]
    fn key_material_try_as_32bytes() {
        let km = KeyMaterial::new(&[0xccu8; 32]);
        let arr = km.try_as_32bytes().unwrap();
        assert_eq!(*arr, [0xccu8; 32]);
    }

    #[test]
    fn key_material_wrong_length_rejected() {
        let km = KeyMaterial::new(&[0xccu8; 16]);
        assert!(km.try_as_32bytes().is_err());
    }

    #[test]
    fn resolve_file_ref() {
        let dir = tmp_dir("resolve_file");
        let path = dir.join("key.bin");
        std::fs::write(&path, hex::encode([0xaau8; 32])).unwrap();
        let r = KeyRef::File(path);
        let km = KeyMaterial::resolve(&r).unwrap();
        assert_eq!(km.as_bytes(), &[0xaau8; 32]);
    }

    #[test]
    fn resolve_env_ref() {
        std::env::set_var("TYU_TEST_RESOLVE", hex::encode([0xbbu8; 32]));
        let r = KeyRef::Env("TYU_TEST_RESOLVE".into());
        let km = KeyMaterial::resolve(&r).unwrap();
        assert_eq!(km.as_bytes(), &[0xbbu8; 32]);
        std::env::remove_var("TYU_TEST_RESOLVE");
    }

    #[test]
    fn zeroize_on_drop() {
        // Verify the KeyMaterial uses Zeroizing<Vec<u8>> internally.
        // Zeroizing guarantees the buffer is cleared on drop — testing via
        // raw pointer reads is UB because the allocator may unmap the page.
        // We instead trust the `zeroize` crate and verify our type API.
        let km = KeyMaterial::new(&[0xabu8; 32]);
        assert_eq!(km.as_bytes(), &[0xabu8; 32]);
        // Zeroizing contract: on drop, memory is overwritten before freeing.
        // The crate is well-tested; we accept that guarantee.
    }
}
