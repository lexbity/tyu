//! Set signing-key registry (P7).
//!
//! A *set* is an over-the-air payload family identified by a `set_id`. Its
//! signing keys live on a fixed roster lane (`sk0` is the primary/latest
//! signing key, `sk1..sk15` are older or secondary lanes). The set-table in
//! the payload routes a *key lane* and a *bit of the set id* to a slot; the
//! registry resolves which signing key authenticates a payload and is the
//! single place a key range is defined.
//!
//! `sniff`/`peek` read a set-payload's header/keys without a full parse, so
//! a consumer can decide *which* registry to use before authenticating.

use crate::error::TyuError;
use std::collections::HashMap;
use std::path::Path;

/// The number of set signing-key lanes a roster can hold (`sk0..sk15`).
pub const SET_KEY_RANGE: usize = 16;

/// Bits needed to index a set-key lane (log2 of `SET_KEY_RANGE`).
pub const SET_KEY_BITS: u8 = 4;

/// Byte length of a raw set signing key.
pub const KEY_LEN: usize = 32;

/// How a set signing key is identified on the roster.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct KeyId {
    /// The set this key belongs to.
    pub set_id: u32,
    /// The lane on the roster (`0` = sk0, the primary/latest).
    pub lane: u8,
}

/// Errors resolving a set's signing key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrySigningError {
    /// The set has no signing key at all.
    NoSigningKey { set_id: u32 },
    /// The requested lane is out of `SET_KEY_RANGE`.
    LaneOutOfRange { lane: u8 },
    /// The roster holds keys for a different set than requested.
    SetMismatch { expected: u32, found: u32 },
}

impl core::fmt::Display for RegistrySigningError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSigningKey { set_id } => write!(f, "set {set_id:#x} has no signing key"),
            Self::LaneOutOfRange { lane } => {
                write!(f, "set-key lane {lane} out of range 0..{SET_KEY_RANGE}")
            }
            Self::SetMismatch { expected, found } => {
                write!(f, "roster is for set {found:#x}, expected {expected:#x}")
            }
        }
    }
}

/// A set's signing-key roster: one key per lane, `sk0` always present.
#[derive(Clone, Debug)]
pub struct Roster {
    /// The set this roster authenticates.
    pub set_id: u32,
    /// Per-lane signing keys; lanes above `len` are vacant.
    pub keys: [Option<[u8; KEY_LEN]>; SET_KEY_RANGE],
}

impl Roster {
    /// The primary signing key (sk0). Every set has one.
    pub fn sk0(&self) -> [u8; KEY_LEN] {
        self.keys[0].expect("a roster always holds sk0")
    }

    /// The signing key for a lane.
    pub fn key(&self, lane: u8) -> Result<[u8; KEY_LEN], RegistrySigningError> {
        if (lane as usize) >= SET_KEY_RANGE {
            return Err(RegistrySigningError::LaneOutOfRange { lane });
        }
        self.keys[lane as usize].ok_or(RegistrySigningError::NoSigningKey {
            set_id: self.set_id,
        })
    }

    /// The highest lane with a key present (the newest secondary key), or 0.
    pub fn highest_present_lane(&self) -> u8 {
        self.keys
            .iter()
            .rposition(|k| k.is_some())
            .map(|i| i as u8)
            .unwrap_or(0)
    }
}

/// The on-disk key roster: a directory of `<set>_sk<lane>.key` files
/// (hex-encoded 32-byte keys). `register_key`/`select_key` operate on this.
#[derive(Clone, Debug)]
pub struct KeyRoster {
    set_id: u32,
    keys: HashMap<u8, [u8; KEY_LEN]>,
}

impl KeyRoster {
    /// Create an empty roster for a set.
    pub fn new(set_id: u32) -> Self {
        Self {
            set_id,
            keys: HashMap::new(),
        }
    }

    /// The set this roster belongs to.
    pub fn set_id(&self) -> u32 {
        self.set_id
    }

    /// Register a lane key (P7 `register_key`).
    pub fn register_key(&mut self, lane: u8, key: [u8; KEY_LEN]) -> Result<(), RegistrySigningError> {
        if (lane as usize) >= SET_KEY_RANGE {
            return Err(RegistrySigningError::LaneOutOfRange { lane });
        }
        self.keys.insert(lane, key);
        Ok(())
    }

    /// Select the key for a lane (P7 `select_key`).
    pub fn select_key(&self, lane: u8) -> Result<[u8; KEY_LEN], RegistrySigningError> {
        if (lane as usize) >= SET_KEY_RANGE {
            return Err(RegistrySigningError::LaneOutOfRange { lane });
        }
        self.keys
            .get(&lane)
            .copied()
            .ok_or(RegistrySigningError::NoSigningKey {
                set_id: self.set_id,
            })
    }

    /// Select the *best* sk0: the primary lane, falling back to the highest
    /// present lane if sk0 was never registered (P7 `select_best_sk0`).
    pub fn select_best_sk0(&self) -> Result<[u8; KEY_LEN], RegistrySigningError> {
        if let Some(k) = self.keys.get(&0) {
            return Ok(*k);
        }
        let best = self
            .keys
            .iter()
            .max_by_key(|(&lane, _)| lane)
            .map(|(&lane, k)| (lane, *k))
            .ok_or(RegistrySigningError::NoSigningKey {
                set_id: self.set_id,
            })?;
        let _ = best.0;
        Ok(best.1)
    }

    /// Materialize a [`Roster`] from this roster (sk0 must be present).
    pub fn into_roster(&self) -> Result<Roster, RegistrySigningError> {
        let mut keys = [None; SET_KEY_RANGE];
        for (&lane, k) in &self.keys {
            if (lane as usize) < SET_KEY_RANGE {
                keys[lane as usize] = Some(*k);
            }
        }
        if keys[0].is_none() {
            return Err(RegistrySigningError::NoSigningKey {
                set_id: self.set_id,
            });
        }
        Ok(Roster {
            set_id: self.set_id,
            keys,
        })
    }

    /// Load a roster from a directory of `<set>_sk<lane>.key` files.
    ///
    /// The directory path encodes the set: its parent is the set id, and each
    /// file is `sk<lane>.key`. Returns the roster and the set id it read.
    pub fn load(dir: &Path) -> Result<(Self, u32), TyuError> {
        let mut set_id: u32 = 0;
        let mut roster = KeyRoster::new(0);
        for entry in std::fs::read_dir(dir)
            .map_err(|e| TyuError::Provision(format!("reading keys dir '{}': {}", dir.display(), e)))?
        {
            let entry = entry.map_err(|e| TyuError::Provision(format!("dir entry: {e}")))?;
            let path = entry.path();
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Accept `<set>_sk<lane>` or bare `sk<lane>`.
            let lane = if let Some(rest) = name.strip_prefix("sk") {
                rest.parse::<u8>().ok()
            } else if let Some(idx) = name.find("_sk") {
                let (set_part, lane_part) = name.split_at(idx);
                let parsed_set = set_part.parse::<u32>().ok();
                if let Some(ps) = parsed_set {
                    set_id = ps;
                    roster.set_id = ps;
                }
                lane_part[3..].parse::<u8>().ok()
            } else {
                None
            };
            let Some(lane) = lane else { continue };
            let hex = std::fs::read_to_string(&path)
                .map_err(|e| TyuError::Provision(format!("reading '{}': {}", path.display(), e)))?;
            let bytes = hex::decode(hex.trim())
                .map_err(|e| TyuError::Provision(format!("invalid hex in '{}': {}", path.display(), e)))?;
            if bytes.len() != KEY_LEN {
                return Err(TyuError::Provision(format!(
                    "set key '{}' must be {KEY_LEN} bytes, got {}",
                    path.display(),
                    bytes.len()
                )));
            }
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            roster.register_key(lane, key).map_err(|e| {
                TyuError::Provision(format!("registering set key '{}': {e}", path.display()))
            })?;
        }
        Ok((roster, set_id))
    }
}

/// Resolve the signing key for a payload (P7). The payload's header declares
/// the set and the key lane; the registry resolves the matching key. The
/// default is `sk0`.
pub fn sign_key_for_payload(
    roster: &Roster,
    lane: u8,
) -> Result<[u8; KEY_LEN], RegistrySigningError> {
    roster.key(lane)
}

/// Peek at a set-payload's header without authenticating it (P7).
///
/// Returns `(set_id, format_ver, slot_count)` for a payload that begins with
/// the v2 header. `None` if the buffer is too small or the magic is wrong.
pub fn peek(payload: &[u8]) -> Option<(u32, u16, u16)> {
    if payload.len() < 12 || &payload[..4] != SET_PAYLOAD_MAGIC {
        return None;
    }
    let set_id = u32::from_le_bytes(payload[4..8].try_into().ok()?);
    let ver = u16::from_le_bytes(payload[8..10].try_into().ok()?);
    let slots = u16::from_le_bytes(payload[10..12].try_into().ok()?);
    Some((set_id, ver, slots))
}

/// Sniff the key lane a payload authenticates with (P7).
///
/// The v2 keys section begins at a fixed offset after the header; its first
/// byte is the key lane. Returns `None` for a truncated payload.
pub fn sniff_key_lane(payload: &[u8]) -> Option<u8> {
    if payload.len() < 13 {
        return None;
    }
    Some(payload[12])
}

/// The set-payload wire-format magic (P7).
pub const SET_PAYLOAD_MAGIC: &[u8; 4] = b"TYSP";

#[cfg(test)]
mod tests {
    use super::*;

    fn key(v: u8) -> [u8; 32] {
        [v; 32]
    }

    #[test]
    fn roster_selects_sk0_and_highest_lane() {
        let mut r = Roster {
            set_id: 0x5,
            keys: [None; SET_KEY_RANGE],
        };
        r.keys[0] = Some(key(1));
        r.keys[2] = Some(key(3));
        assert_eq!(r.sk0(), key(1));
        assert_eq!(r.key(2).unwrap(), key(3));
        assert_eq!(r.highest_present_lane(), 2);
        assert!(matches!(
            r.key(9),
            Err(RegistrySigningError::NoSigningKey { set_id: 0x5 })
        ));
        assert!(matches!(
            r.key(20),
            Err(RegistrySigningError::LaneOutOfRange { lane: 20 })
        ));
    }

    #[test]
    fn key_roster_select_best_sk0_falls_back() {
        let mut kr = KeyRoster::new(0x9);
        assert!(matches!(kr.select_best_sk0(), Err(_)));
        kr.register_key(3, key(7)).unwrap();
        assert_eq!(kr.select_best_sk0().unwrap(), key(7));
        kr.register_key(0, key(1)).unwrap();
        assert_eq!(kr.select_best_sk0().unwrap(), key(1));
    }

    #[test]
    fn sign_key_for_payload_resolves() {
        let mut r = Roster {
            set_id: 0x1,
            keys: [None; SET_KEY_RANGE],
        };
        r.keys[0] = Some(key(2));
        r.keys[1] = Some(key(9));
        assert_eq!(sign_key_for_payload(&r, 0).unwrap(), key(2));
        assert_eq!(sign_key_for_payload(&r, 1).unwrap(), key(9));
    }

    #[test]
    fn peek_and_sniff_read_v2_header() {
        let mut payload = Vec::new();
        payload.extend_from_slice(SET_PAYLOAD_MAGIC);
        payload.extend_from_slice(&0x2Au32.to_le_bytes()); // set_id
        payload.extend_from_slice(&2u16.to_le_bytes()); // format_ver
        payload.extend_from_slice(&3u16.to_le_bytes()); // slots
        payload.push(5); // key lane (keys section start)
        assert_eq!(peek(&payload), Some((0x2a, 2, 3)));
        assert_eq!(sniff_key_lane(&payload), Some(5));
        assert_eq!(peek(&[0; 4]), None);
        assert_eq!(sniff_key_lane(&[0; 4]), None);
    }

    #[test]
    fn sign_key_for_payload_wires_into_real_signing() {
        // The registry resolves the signing key; lmod-sign consumes it. The
        // signature must be the same as signing directly with sk0 (same key
        // material -> deterministic signature), proving the resolution is the
        // one the payload actually authenticates with.
        let mut r = Roster {
            set_id: 0x1,
            keys: [None; SET_KEY_RANGE],
        };
        r.keys[0] = Some(key(0xAA));
        let mut hdr = lmod::header::LmodHeader::new();
        hdr.total_len = lmod::header::HEADER_SIZE;
        let mut payload = [0u8; lmod::header::HEADER_SIZE as usize];
        lmod::header::encode_header(&mut payload, &hdr);
        let via_registry =
            lmod_sign::sign(&payload, &sign_key_for_payload(&r, 0).unwrap()).unwrap();
        let via_sk0 = lmod_sign::sign(&payload, &r.sk0()).unwrap();
        assert_eq!(via_registry, via_sk0);
        // A different lane key must produce a different signature.
        r.keys[1] = Some(key(0xBB));
        let via_lane1 = lmod_sign::sign(&payload, &sign_key_for_payload(&r, 1).unwrap()).unwrap();
        assert_ne!(via_registry, via_lane1);
    }
}