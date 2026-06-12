//! Shared test utility functions for `tyu` integration tests.
//!
//! Consolidates the `workspace_root`, `tool_available`, `require_tools`,
//! `temp_dir`, `tyu_exe`, `langc_exe`, `introspect_lmod`, `LmodFacts`,
//! and `write_device_keys` helpers that were previously copy-pasted
//! into every test file.

use std::path::{Path, PathBuf};
use std::process::Command;

use lmod::enc::{EncMode, decode_enc_header};
use lmod::header::{LMOD_FLAG_ENCRYPTED, LMOD_FLAG_SIGNED, HEADER_SIZE};
use lmod::validate::Container;

/// Absolute path to the workspace root (two levels up from `CARGO_MANIFEST_DIR`).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

/// Path to the `test-goldens/` directory at the workspace root.
pub fn golden_dir() -> PathBuf {
    workspace_root().join("test-goldens")
}

/// Resolve a workspace binary for e2e use.
///
/// Priority:
///   1. `$TYU_BIN_DIR/<name>` — authoritative override for CI.
///   2. `target/debug/<name>` — local debug build.
///   3. `target/release/<name>` — local release build.
pub fn bin(name: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("TYU_BIN_DIR") {
        let p = PathBuf::from(&dir).join(name);
        assert!(
            p.is_file(),
            "TYU_BIN_DIR={dir} set but {name} not found at {p:?}"
        );
        return p;
    }
    let root = workspace_root();
    let p = root.join("target").join("debug").join(name);
    if p.exists() {
        return p;
    }
    root.join("target").join("release").join(name)
}

/// The `tyu` driver binary.
pub fn tyu_exe() -> PathBuf {
    bin("tyu")
}

/// The `langc` cross-compiler binary.
pub fn langc_exe() -> PathBuf {
    bin("langc")
}

/// Returns true if a named binary exists — either on `PATH`, in
/// `target/debug/`, or in `target/release/` (for workspace-built
/// binaries like `langc`, `tyu`).
pub fn tool_available(name: &str) -> bool {
    // Check PATH via which.
    if Command::new("which").arg(name).output()
        .map(|o| o.status.success()).unwrap_or(false)
    {
        return true;
    }
    // Check target/debug/ then target/release/ for workspace-built binaries.
    let root = workspace_root();
    let p = root.join("target").join("debug").join(name);
    if p.exists() {
        return true;
    }
    root.join("target").join("release").join(name).exists()
}

/// Environment-aware tool gating: requires all named tools, panics under CI
/// if missing, otherwise prints a skip message.
/// Returns `true` when all tools are present.
pub fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools.iter()
        .filter(|t| !tool_available(t))
        .copied().collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!("Required tools not available under CI: {}", missing.join(", "));
    }
    eprintln!("SKIP: required tools not available ({})", missing.join(", "));
    false
}

/// Create a temporary directory for a test, cleaning up any previous
/// directory with the same label.
pub fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// .lmod container introspection
// ---------------------------------------------------------------------------

/// Structural facts extracted from a `.lmod` file for test assertions.
pub struct LmodFacts {
    pub encrypted: bool,
    pub signed: bool,
    pub format_ver: u16,
    pub enc_mode: Option<EncMode>,
    pub slot_count: usize,
}

/// Parse a `.lmod` file and return its structural facts.
///
/// Never panics on valid input.  Panics on I/O or parse error so the
/// calling test fails clearly.
pub fn introspect_lmod(path: &Path) -> LmodFacts {
    let data = std::fs::read(path).expect("introspect_lmod: read");
    let container = Container::parse(&data).expect("introspect_lmod: parse");
    let hdr = container.header();
    let encrypted = (hdr.flags & LMOD_FLAG_ENCRYPTED) != 0;
    let signed = (hdr.flags & LMOD_FLAG_SIGNED) != 0;
    let format_ver = hdr.format_ver;
    let (enc_mode, slot_count) = if encrypted {
        let eh_bytes = &data[HEADER_SIZE as usize..];
        match decode_enc_header(eh_bytes) {
            Some(eh) => (Some(eh.enc_mode), eh.wrapped_slots.len()),
            None => (None, 0),
        }
    } else {
        (None, 0)
    };
    LmodFacts { encrypted, signed, format_ver, enc_mode, slot_count }
}

// ---------------------------------------------------------------------------
// Device key directory builder
// ---------------------------------------------------------------------------

/// Write device key files into `dir` and return `dir`.
///
/// Each entry `(id, key)` produces `<dir>/<id>.key` containing the
/// hex-encoded 32-byte key.
pub fn write_device_keys(dir: &Path, ids_and_keys: &[(&str, [u8; 32])]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    for (id, key) in ids_and_keys {
        let path = dir.join(format!("{}.key", id));
        std::fs::write(&path, hex::encode(key)).unwrap();
    }
    dir.to_path_buf()
}

// ---------------------------------------------------------------------------
// Self-tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lmod::enc::{EncHeader, WrappedCekSlot, WRAPPED_SLOT_SIZE, WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, WRAP_LEN};
    use lmod::header::{self, FORMAT_VER, compute_layout, encode_header};

    /// Build a minimal valid plaintext .lmod container on disk.
    fn write_plaintext_lmod(path: &Path) {
        let layout = compute_layout(42, 16, 8, 0, 0, 0, 0, 0);
        let mut buf = vec![0u8; layout.total_len as usize];
        encode_header(&mut buf, &layout);
        let mi_start = layout.modinfo_off as usize;
        buf[mi_start..mi_start + 4].copy_from_slice(b"MODI");
        std::fs::write(path, &buf).unwrap();
    }

    /// Build a minimal fleet-encrypted .lmod container on disk.
    fn write_encrypted_fleet_lmod(path: &Path) {
        let eh_len = lmod::enc::enc_header_len(1) as u32;
        let mut hdr = compute_layout(42, 16, 8, 0, 0, 0, 0, eh_len);
        hdr.flags |= header::LMOD_FLAG_ENCRYPTED;
        let mut buf = vec![0u8; hdr.total_len as usize];
        encode_header(&mut buf, &hdr);
        let slot = WrappedCekSlot {
            key_id: 0,
            wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
            wrapped: [0xABu8; WRAP_LEN],
        };
        let eh = EncHeader {
            enc_mode: EncMode::Fleet,
            aead_id: lmod::enc::AEAD_CHACHA20POLY1305,
            nonce: [0u8; 12],
            tag: [0u8; 16],
            wrapped_slots: vec![slot],
        };
        let eh_start = header::HEADER_SIZE as usize;
        lmod::enc::encode_enc_header(&mut buf[eh_start..], &eh).unwrap();
        let mi_start = hdr.modinfo_off as usize;
        buf[mi_start..mi_start + 4].copy_from_slice(b"MODI");
        std::fs::write(path, &buf).unwrap();
    }

    #[test]
    fn introspect_plaintext_reports_unencrypted() {
        let dir = temp_dir("introspect_plain");
        let path = dir.join("test.lmod");
        write_plaintext_lmod(&path);
        let facts = introspect_lmod(&path);
        assert!(!facts.encrypted, "plaintext: encrypted must be false");
        assert!(!facts.signed, "plaintext: signed must be false");
        assert_eq!(facts.format_ver, FORMAT_VER);
        assert!(facts.enc_mode.is_none(), "plaintext: no enc_mode");
        assert_eq!(facts.slot_count, 0);
    }

    #[test]
    fn introspect_fleet_reports_one_slot() {
        let dir = temp_dir("introspect_fleet");
        let path = dir.join("test.lmod");
        write_encrypted_fleet_lmod(&path);
        let facts = introspect_lmod(&path);
        assert!(facts.encrypted, "encrypted: encrypted must be true");
        assert_eq!(facts.enc_mode, Some(EncMode::Fleet), "encrypted: enc_mode=Fleet");
        assert_eq!(facts.slot_count, 1, "encrypted: slot_count=1");
        assert_eq!(facts.format_ver, FORMAT_VER);
    }

    #[test]
    fn write_device_keys_creates_files() {
        let dir = temp_dir("write_keys");
        let ids = &[("dev-a", [0xAAu8; 32]), ("dev-b", [0xBBu8; 32])];
        let path = write_device_keys(&dir, ids);
        assert!(path.join("dev-a.key").exists());
        assert!(path.join("dev-b.key").exists());
        let content = std::fs::read_to_string(path.join("dev-a.key")).unwrap();
        assert_eq!(content.trim(), hex::encode([0xAAu8; 32]));
    }

    #[test]
    fn write_device_keys_ignores_extra_suffixes() {
        let dir = temp_dir("write_keys_suffix");
        let ids = &[("dev-a", [0xAAu8; 32])];
        let path = write_device_keys(&dir, ids);
        let files: Vec<_> = std::fs::read_dir(&path).unwrap().collect();
        assert_eq!(files.len(), 1, "only one .key file should exist");
    }
}
