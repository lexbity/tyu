//! Set-payload key selection (P7).
//!
//! Key material for an over-the-air set payload: the roster selection helpers
//! and the keys-section protocol (how a payload carries the key lane it
//! authenticates with), plus the conversion between key material and the
//! region request that selects a slot.

use super::region::{RegionReq, RegionState};
use super::registry::{KeyId, KeyRoster, RegistrySigningError, Roster, SET_KEY_BITS, SET_KEY_RANGE};

/// A key register for a set payload: the roster plus a currently-selected lane.
#[derive(Clone, Debug)]
pub struct KeyRegister {
    pub roster: KeyRoster,
    pub selected_lane: u8,
}

/// Errors from the keys-section protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeysSectionError {
    /// The keys section is shorter than the lane byte.
    Truncated,
    /// The lane byte is out of `SET_KEY_RANGE`.
    LaneOutOfRange { lane: u8 },
}

impl core::fmt::Display for KeysSectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => write!(f, "keys section truncated"),
            Self::LaneOutOfRange { lane } => {
                write!(f, "key lane {lane} out of range 0..{SET_KEY_RANGE}")
            }
        }
    }
}

impl KeyRegister {
    /// Create a register over a roster, defaulting to sk0.
    pub fn new(roster: KeyRoster) -> Self {
        Self {
            roster,
            selected_lane: 0,
        }
    }

    /// Register a key onto the roster's lane (P7 `register_key`).
    pub fn register_key(
        &mut self,
        lane: u8,
        key: [u8; 32],
    ) -> Result<(), RegistrySigningError> {
        self.roster.register_key(lane, key)
    }

    /// Select a lane's key (P7 `select_key`).
    pub fn select_key(&self, lane: u8) -> Result<[u8; 32], RegistrySigningError> {
        self.roster.select_key(lane)
    }

    /// Select the best sk0 on the roster (P7 `select_best_sk0`).
    pub fn select_best_sk0(&self) -> Result<[u8; 32], RegistrySigningError> {
        self.roster.select_best_sk0()
    }
}

/// Encode the keys section of a set payload (v2): the lane byte followed by
/// the set id (so the device can cross-check the roster).
pub fn encode_keys_section(lane: u8, set_id: u32) -> Result<[u8; 5], KeysSectionError> {
    if lane as usize >= SET_KEY_RANGE {
        return Err(KeysSectionError::LaneOutOfRange { lane });
    }
    let mut out = [0u8; 5];
    out[0] = lane;
    out[1..5].copy_from_slice(&set_id.to_le_bytes());
    Ok(out)
}

/// Decode the keys section of a set payload (P7).
pub fn decode_keys_section(bytes: &[u8]) -> Result<(u8, u32), KeysSectionError> {
    if bytes.len() < 5 {
        return Err(KeysSectionError::Truncated);
    }
    let lane = bytes[0];
    if lane as usize >= SET_KEY_RANGE {
        return Err(KeysSectionError::LaneOutOfRange { lane });
    }
    let set_id = u32::from_le_bytes(bytes[1..5].try_into().unwrap());
    Ok((lane, set_id))
}

/// Convert key material + a set id into the region request that selects the
/// slot for a payload (P7 `keys_to_region_request`).
///
/// The slot is `lane << SET_KEY_BITS | (set_id & 0x0f)`: the key lane routes
/// to the slot group, and the low nibble of the set id picks the member —
/// the set-table's per-key / per-bit routing.
pub fn keys_to_region_request(
    lane: u8,
    set_id: u32,
    state: RegionState,
) -> Result<RegionReq, KeysSectionError> {
    if lane as usize >= SET_KEY_RANGE {
        return Err(KeysSectionError::LaneOutOfRange { lane });
    }
    Ok(RegionReq {
        set_id,
        lane,
        slot: ((lane as u16) << SET_KEY_BITS) | (set_id as u16 & 0x0f),
        state,
    })
}

/// Convert a region request back into the key lane + set id it was derived
/// from (P7 `region_request_to_keys`).
pub fn region_request_to_keys(req: &RegionReq) -> (u8, u32) {
    (req.lane, req.set_id)
}

/// Resolve the roster for a request's key id from a set's keys section.
pub fn roster_key_for_request(
    roster: &Roster,
    req: &RegionReq,
) -> Result<[u8; 32], RegistrySigningError> {
    if roster.set_id != req.set_id {
        return Err(RegistrySigningError::SetMismatch {
            expected: req.set_id,
            found: roster.set_id,
        });
    }
    roster.key(req.lane)
}

/// A key identity for a request (P7).
pub fn key_id_for_request(req: &RegionReq) -> KeyId {
    KeyId {
        set_id: req.set_id,
        lane: req.lane,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::region::RegionState;

    #[test]
    fn keys_section_roundtrip() {
        let enc = encode_keys_section(3, 0xabc).unwrap();
        assert_eq!(decode_keys_section(&enc).unwrap(), (3, 0xabc));
        assert!(matches!(
            encode_keys_section(20, 0),
            Err(KeysSectionError::LaneOutOfRange { lane: 20 })
        ));
        assert!(matches!(
            decode_keys_section(&[0]),
            Err(KeysSectionError::Truncated)
        ));
    }

    #[test]
    fn keys_to_region_request_routes_lane_and_set_bits() {
        let req = keys_to_region_request(2, 0x5, RegionState::Naked).unwrap();
        // slot = (2 << 4) | (0x5 & 0xf) = 0x25
        assert_eq!(req.slot, 0x25);
        assert_eq!(region_request_to_keys(&req), (2, 0x5));
        assert!(matches!(
            keys_to_region_request(20, 0, RegionState::Naked),
            Err(KeysSectionError::LaneOutOfRange { lane: 20 })
        ));
    }

    #[test]
    fn register_select_cycle() {
        let mut reg = KeyRegister::new(KeyRoster::new(7));
        reg.register_key(0, [9; 32]).unwrap();
        reg.register_key(1, [4; 32]).unwrap();
        assert_eq!(reg.select_key(0).unwrap(), [9; 32]);
        assert_eq!(reg.select_key(1).unwrap(), [4; 32]);
        assert_eq!(reg.select_best_sk0().unwrap(), [9; 32]);
    }

    #[test]
    fn roster_key_for_request_checks_set() {
        let mut r = Roster {
            set_id: 1,
            keys: [None; SET_KEY_RANGE],
        };
        r.keys[0] = Some([2; 32]);
        let req = keys_to_region_request(0, 1, RegionState::Provisioned).unwrap();
        assert_eq!(roster_key_for_request(&r, &req).unwrap(), [2; 32]);
        let other = keys_to_region_request(0, 99, RegionState::Provisioned).unwrap();
        assert!(matches!(
            roster_key_for_request(&r, &other),
            Err(RegistrySigningError::SetMismatch { .. })
        ));
    }
}