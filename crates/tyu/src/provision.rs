//! Device identity registry for per-device key provisioning.
//!
//! Reads device KEKs from a directory, each file named `<device-id>.key`
//! containing a hex-encoded 32-byte key.  The registry maps device IDs
//! to raw KEK bytes for use by the deploy command.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A registry of device KEKs loaded from a directory.
#[derive(Clone, Debug)]
pub struct DeviceRegistry {
    /// Map from device ID (e.g. `"device-a"`) to 32-byte KEK.
    keys: HashMap<String, [u8; 32]>,
}

impl DeviceRegistry {
    /// Load device keys from a directory.
    ///
    /// Reads every file matching `*.key` in `dir`, treating the filename
    /// stem as the device ID and the file content as a hex-encoded 32-byte key.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut keys = HashMap::new();
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("reading device keys dir '{}': {}", dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("reading dir entry: {}", e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("key") {
                continue;
            }
            let device_id = path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| format!("invalid device key filename: {}", path.display()))?;

            let content = fs::read_to_string(&path)
                .map_err(|e| format!("reading '{}': {}", path.display(), e))?;
            let hex = content.trim();
            let bytes = hex::decode(hex)
                .map_err(|e| format!("invalid hex in '{}': {}", path.display(), e))?;
            if bytes.len() != 32 {
                return Err(format!(
                    "device key in '{}' must be 32 bytes, got {}",
                    path.display(), bytes.len()
                ));
            }
            let mut kek = [0u8; 32];
            kek.copy_from_slice(&bytes);
            keys.insert(device_id, kek);
        }

        Ok(DeviceRegistry { keys })
    }

    /// Look up a device's KEK by ID.
    pub fn kek(&self, device_id: &str) -> Option<&[u8; 32]> {
        self.keys.get(device_id)
    }

    /// Returns the number of registered devices.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns true if no devices are registered.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Iterate over all (device_id, kek) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8; 32])> {
        self.keys.iter().map(|(k, v)| (k.as_str(), v))
    }
}

// ---------------------------------------------------------------------------
// Unit tests (U-DR-1 through U-DR-7)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("tyu_provision_tests")
            .join(format!("{}_{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_all_keys_and_looks_up() {
        let dir = tmp_dir("dr_load");
        std::fs::write(dir.join("dev-a.key"), hex::encode([0xaa; 32])).unwrap();
        std::fs::write(dir.join("dev-b.key"), hex::encode([0xbb; 32])).unwrap();
        let reg = DeviceRegistry::load(&dir).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.kek("dev-a"), Some(&[0xaa; 32]));
        assert_eq!(reg.kek("dev-b"), Some(&[0xbb; 32]));
        assert_eq!(reg.kek("dev-c"), None);
    }

    #[test]
    fn rejects_short_key() {
        let dir = tmp_dir("dr_short");
        std::fs::write(dir.join("bad.key"), hex::encode([0xcc; 16])).unwrap();
        let result = DeviceRegistry::load(&dir);
        assert!(result.is_err(), "16-byte key should be rejected");
    }

    #[test]
    fn rejects_long_key() {
        let dir = tmp_dir("dr_long");
        std::fs::write(dir.join("bad.key"), hex::encode([0xdd; 48])).unwrap();
        let result = DeviceRegistry::load(&dir);
        assert!(result.is_err(), "48-byte key should be rejected");
    }

    #[test]
    fn rejects_invalid_hex() {
        let dir = tmp_dir("dr_hex");
        std::fs::write(dir.join("bad.key"), "zz1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
        let result = DeviceRegistry::load(&dir);
        assert!(result.is_err(), "invalid hex should be rejected");
    }

    #[test]
    fn ignores_non_key_files() {
        let dir = tmp_dir("dr_nonkey");
        std::fs::write(dir.join("a.key"), hex::encode([0xaa; 32])).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a key").unwrap();
        let reg = DeviceRegistry::load(&dir).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn tolerates_trailing_whitespace() {
        let dir = tmp_dir("dr_ws");
        let mut content = hex::encode([0xee; 32]);
        content.push('\n');
        std::fs::write(dir.join("dev-e.key"), &content).unwrap();
        let reg = DeviceRegistry::load(&dir).unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.kek("dev-e"), Some(&[0xee; 32]));
    }

    #[test]
    fn empty_dir_is_empty() {
        let dir = tmp_dir("dr_empty");
        std::fs::create_dir_all(&dir).unwrap();
        let reg = DeviceRegistry::load(&dir).unwrap();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }
}
