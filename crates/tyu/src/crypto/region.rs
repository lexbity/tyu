//! Region-with-rollback protocol (P7).
//!
//! The host-side (`tyu`) implementation of the set-payload region state
//! machine that the on-device loader mirrors. A *region* is a bounded store
//! for over-the-air set payloads: it holds one installed payload at a time,
//! can hold a candidate for the next, and can roll back to the baseline. The
//! state machine (`naked` / `provisioned` / `committing` / `rollback`) is
//! driven by [`RegionLoader`]; region exhaustion surfaces as `TrapCode 26`
//! (`RegionExhausted`) — a distinct pre-panic region-failure trap with its own
//! trace record.
//!
//! A set payload (wire v2) carries a set-table that routes a *key lane* and
//! the *low nibble of the set id* to a slot — per-key and per-bit routing.

use super::keys::{decode_keys_section, keys_to_region_request};
use super::registry::{SET_PAYLOAD_MAGIC, SET_KEY_BITS};

/// The region state machine (P7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionState {
    /// No payload installed, no candidate.
    Naked,
    /// A payload is installed and active.
    Provisioned,
    /// A candidate is being committed.
    Committing,
    /// Rolling back to the baseline.
    Rollback,
}

impl RegionState {
    pub fn as_str(self) -> &'static str {
        match self {
            RegionState::Naked => "naked",
            RegionState::Provisioned => "provisioned",
            RegionState::Committing => "committing",
            RegionState::Rollback => "rollback",
        }
    }
}

/// A region request: which set/lane/slot the caller wants to operate on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionReq {
    pub set_id: u32,
    pub lane: u8,
    pub slot: u16,
    pub state: RegionState,
}

/// A region response: the resulting state and remaining free bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionResp {
    pub state: RegionState,
    pub slot: u16,
    pub region_free: u32,
}

/// A region protocol message (host → device and device → host).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionMsg {
    Report(RegionReq),
    Candidate(RegionReq),
    Commit(RegionReq),
    Rollback(RegionReq),
    Resp(RegionResp),
}

/// Errors from the region protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegionError {
    /// The payload does not begin with the set-payload magic.
    BadMagic,
    /// The set-table is malformed or a slot index is out of range.
    BadSetTable,
    /// The region is full: no slot can be allocated (`TrapCode 26`).
    RegionFull,
    /// The keys section is invalid.
    BadKeys,
    /// A lane routed to a slot with no matching key lane.
    LaneNotRouted { lane: u8 },
}

impl core::fmt::Display for RegionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "payload has no set-payload magic"),
            Self::BadSetTable => write!(f, "set-table is malformed"),
            Self::RegionFull => write!(f, "region exhausted (trap 26)"),
            Self::BadKeys => write!(f, "payload keys section is invalid"),
            Self::LaneNotRouted { lane } => write!(f, "key lane {lane} routes to no slot"),
        }
    }
}

/// A set-table route: for a key lane, the set-id low-nibble bits that select
/// the slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetRoute {
    pub lane: u8,
    /// Bitmask over the low nibble of the set id; the route applies when
    /// `(set_id & bit_mask) != 0` (and the complement routes to slot 0).
    pub bit_mask: u8,
    pub slot: u16,
}

/// The parsed set-table of a set payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetTable {
    pub set_id: u32,
    pub slot_count: u16,
    pub routes: Vec<SetRoute>,
}

/// Parse the set-table from a v2 set payload (P7). Layout after the 12-byte
/// header and the 5-byte keys section:
///   `u16 slot_count`, `u16 route_count`, then `route_count × {u8 lane, u8 bit_mask, u16 slot}`.
pub fn parse_set_table(payload: &[u8]) -> Result<SetTable, RegionError> {
    if payload.len() < 12 || &payload[..4] != SET_PAYLOAD_MAGIC {
        return Err(RegionError::BadMagic);
    }
    // G10 discipline: every slice read is length-guarded AND the `try_into`
    // is checked (never `.unwrap()` on host-derived bytes) so a malformed
    // payload can only yield a parse error, never a panic.
    let set_id = u32::from_le_bytes(
        payload
            .get(4..8)
            .ok_or(RegionError::BadSetTable)?
            .try_into()
            .map_err(|_| RegionError::BadSetTable)?,
    );
    let mut p = 17usize; // 12 header + 5 keys section
    if payload.len() < p + 4 {
        return Err(RegionError::BadSetTable);
    }
    let slot_count = u16::from_le_bytes(
        payload
            .get(p..p + 2)
            .ok_or(RegionError::BadSetTable)?
            .try_into()
            .map_err(|_| RegionError::BadSetTable)?,
    );
    let route_count = u16::from_le_bytes(
        payload
            .get(p + 2..p + 4)
            .ok_or(RegionError::BadSetTable)?
            .try_into()
            .map_err(|_| RegionError::BadSetTable)?,
    );
    p += 4;
    let mut routes = Vec::with_capacity(route_count as usize);
    for _ in 0..route_count {
        if payload.len() < p + 4 {
            return Err(RegionError::BadSetTable);
        }
        let lane = payload[p];
        let bit_mask = payload[p + 1];
        let slot = u16::from_le_bytes(
            payload
                .get(p + 2..p + 4)
                .ok_or(RegionError::BadSetTable)?
                .try_into()
                .map_err(|_| RegionError::BadSetTable)?,
        );
        if slot >= slot_count {
            return Err(RegionError::BadSetTable);
        }
        routes.push(SetRoute {
            lane,
            bit_mask,
            slot,
        });
        p += 4;
    }
    Ok(SetTable {
        set_id,
        slot_count,
        routes,
    })
}

impl SetTable {
    /// The slot for a key lane + set id (per-key / per-bit routing, P7).
    ///
    /// The first route whose lane matches and whose bitmask is set by the set
    /// id's low nibble wins; a lane with no set bit routes to slot 0.
    pub fn slot_for(&self, lane: u8, set_id: u32) -> Result<u16, RegionError> {
        let bits = (set_id as u8) & 0x0f;
        let mut fallback = None;
        for r in &self.routes {
            if r.lane != lane {
                continue;
            }
            if bits & r.bit_mask != 0 {
                return Ok(r.slot);
            }
            if r.bit_mask == 0 {
                fallback = Some(r.slot);
            }
        }
        fallback.ok_or(RegionError::LaneNotRouted { lane })
    }
}

/// The region loader: drives the state machine and the set-table routing.
#[derive(Clone, Debug)]
pub struct RegionLoader {
    pub set_id: u32,
    pub capacity: u32,
    pub used: u32,
    pub state: RegionState,
    pub installed_slot: u16,
    pub candidate_slot: u16,
}

impl RegionLoader {
    /// Initialise a region over a bounded store for a set (P7 `init`).
    pub fn init(set_id: u32, capacity: u32) -> Self {
        Self {
            set_id,
            capacity,
            used: 0,
            state: RegionState::Naked,
            installed_slot: 0,
            candidate_slot: 0,
        }
    }

    /// Recompute the state from a candidate payload (P7 `recompute_state`).
    pub fn recompute_state(&self, candidate: &[u8]) -> Result<RegionState, RegionError> {
        let table = parse_set_table(candidate)?;
        let (lane, _) = decode_keys_section(&candidate[12..]).map_err(|_| RegionError::BadKeys)?;
        let slot = table.slot_for(lane, table.set_id)?;
        if slot >= table.slot_count {
            return Err(RegionError::BadSetTable);
        }
        if self.state == RegionState::Rollback {
            return Ok(RegionState::Rollback);
        }
        if self.installed_slot == slot {
            Ok(RegionState::Provisioned)
        } else {
            Ok(RegionState::Committing)
        }
    }

    /// Report the current region state (P7 `report`).
    pub fn report(&self) -> RegionResp {
        RegionResp {
            state: self.state,
            slot: self.installed_slot,
            region_free: self.capacity.saturating_sub(self.used),
        }
    }

    /// Build a report request for a payload (P7, keys → region request).
    pub fn request_for(
        &self,
        payload: &[u8],
    ) -> Result<RegionReq, RegionError> {
        let table = parse_set_table(payload)?;
        let (lane, _) = decode_keys_section(&payload[12..]).map_err(|_| RegionError::BadKeys)?;
        let slot = table.slot_for(lane, table.set_id)?;
        keys_to_region_request(lane, table.set_id, self.state)
            .map(|mut req| {
                req.slot = slot;
                req
            })
            .map_err(|_| RegionError::BadKeys)
    }

    /// Bring in a candidate payload (P7 `candidate`). Reserves the region
    /// space; returns `RegionFull` (trap 26) if the region cannot hold it.
    pub fn candidate(&mut self, payload: &[u8]) -> Result<u16, RegionError> {
        let table = parse_set_table(payload)?;
        let (lane, _) = decode_keys_section(&payload[12..]).map_err(|_| RegionError::BadKeys)?;
        let slot = table.slot_for(lane, table.set_id)?;
        let need = payload.len() as u32;
        if self.used.saturating_add(need) > self.capacity {
            return Err(RegionError::RegionFull);
        }
        self.candidate_slot = slot;
        self.state = RegionState::Committing;
        Ok(slot)
    }

    /// Commit the candidate (P7 `commit`): it becomes the installed payload.
    pub fn commit(&mut self, payload: &[u8]) -> Result<RegionResp, RegionError> {
        let slot = self.candidate(payload)?;
        self.used = self.used.saturating_add(payload.len() as u32);
        self.installed_slot = slot;
        self.state = RegionState::Provisioned;
        Ok(self.report())
    }

    /// Roll back to the baseline (P7 `rollback`): the installed payload is
    /// discarded and the region returns to `naked`.
    pub fn rollback(&mut self) -> RegionResp {
        self.state = RegionState::Rollback;
        let resp = self.report();
        self.used = 0;
        self.installed_slot = 0;
        self.candidate_slot = 0;
        self.state = RegionState::Naked;
        resp
    }

    /// The fallback blob a device uses when a lane routes to no slot (P7):
    /// the baseline region contents (empty region → empty baseline).
    pub fn fallback(&self) -> &'static [u8] {
        &[]
    }
}

/// Encode a region protocol message (P7 wire): one opcode byte then the
/// request/response fields.
pub fn encode_region_msg(msg: &RegionMsg) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    match msg {
        RegionMsg::Report(req) => {
            out.push(1);
            out.extend_from_slice(&req.set_id.to_le_bytes());
            out.push(req.lane);
            out.extend_from_slice(&req.slot.to_le_bytes());
            out.push(req.state as u8);
        }
        RegionMsg::Candidate(req) => {
            out.push(2);
            out.extend_from_slice(&req.set_id.to_le_bytes());
            out.push(req.lane);
            out.extend_from_slice(&req.slot.to_le_bytes());
            out.push(req.state as u8);
        }
        RegionMsg::Commit(req) => {
            out.push(3);
            out.extend_from_slice(&req.set_id.to_le_bytes());
            out.push(req.lane);
            out.extend_from_slice(&req.slot.to_le_bytes());
            out.push(req.state as u8);
        }
        RegionMsg::Rollback(req) => {
            out.push(4);
            out.extend_from_slice(&req.set_id.to_le_bytes());
            out.push(req.lane);
            out.extend_from_slice(&req.slot.to_le_bytes());
            out.push(req.state as u8);
        }
        RegionMsg::Resp(resp) => {
            out.push(5);
            out.push(resp.state as u8);
            out.extend_from_slice(&resp.slot.to_le_bytes());
            out.extend_from_slice(&resp.region_free.to_le_bytes());
        }
    }
    out
}

/// Decode a region protocol message (P7 wire).
pub fn decode_region_msg(bytes: &[u8]) -> Option<RegionMsg> {
    if bytes.is_empty() {
        return None;
    }
    let req_from = |bytes: &[u8]| -> Option<RegionReq> {
        if bytes.len() < 9 {
            return None;
        }
        let set_id = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
        let lane = bytes[5];
        let slot = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
        let state = region_state_from_u8(bytes[8])?;
        Some(RegionReq {
            set_id,
            lane,
            slot,
            state,
        })
    };
    match bytes[0] {
        1 => req_from(bytes).map(RegionMsg::Report),
        2 => req_from(bytes).map(RegionMsg::Candidate),
        3 => req_from(bytes).map(RegionMsg::Commit),
        4 => req_from(bytes).map(RegionMsg::Rollback),
        5 => {
            if bytes.len() < 8 {
                return None;
            }
            let state = region_state_from_u8(bytes[1])?;
            let slot = u16::from_le_bytes(bytes[2..4].try_into().ok()?);
            let free = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
            Some(RegionMsg::Resp(RegionResp {
                state,
                slot,
                region_free: free,
            }))
        }
        _ => None,
    }
}

fn region_state_from_u8(v: u8) -> Option<RegionState> {
    match v {
        0 => Some(RegionState::Naked),
        1 => Some(RegionState::Provisioned),
        2 => Some(RegionState::Committing),
        3 => Some(RegionState::Rollback),
        _ => None,
    }
}

/// The trap a region-full commit raises (P7: `TrapCode::RegionExhausted`).
/// Derived from the canonical IR value so the two never drift apart.
pub const REGION_EXHAUSTED_TRAP: u32 =
    ir::trap_code_u32(ir::TrapCode::RegionExhausted);

/// The number of slots a payload's set-table can route (`SET_KEY_BITS` lanes
/// × the 4-bit set-id nibble).
pub const MAX_ROUTABLE_SLOTS: u16 = (1 << (SET_KEY_BITS * 2)) as u16;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keys::region_request_to_keys;

    fn payload(set_id: u32, lanes: &[(u8, u8, u16)], slots: u16) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(SET_PAYLOAD_MAGIC);
        v.extend_from_slice(&set_id.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes()); // format_ver
        v.extend_from_slice(&slots.to_le_bytes());
        v.extend_from_slice(&[0]); // keys section lane
        v.extend_from_slice(&set_id.to_le_bytes()); // keys section set id
        v.extend_from_slice(&slots.to_le_bytes());
        v.extend_from_slice(&(lanes.len() as u16).to_le_bytes());
        for &(lane, mask, slot) in lanes {
            v.push(lane);
            v.push(mask);
            v.extend_from_slice(&slot.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_set_table_and_route_per_key_per_bit() {
        let p = payload(0x7, &[(0, 0x1, 1), (1, 0x2, 2)], 4);
        let t = parse_set_table(&p).unwrap();
        assert_eq!(t.set_id, 0x7);
        assert_eq!(t.slot_count, 4);
        // lane 0, set id 0x7 (low nibble 0x7, bit 0x1 set) -> slot 1
        assert_eq!(t.slot_for(0, 0x7).unwrap(), 1);
        // lane 1, low nibble has bit 0x2 set -> slot 2
        assert_eq!(t.slot_for(1, 0x7).unwrap(), 2);
        // lane 0 with a set id whose low nibble has no 0x1 bit -> no route
        assert!(matches!(
            t.slot_for(0, 0x4),
            Err(RegionError::LaneNotRouted { lane: 0 })
        ));
    }

    #[test]
    fn region_loader_state_machine() {
        let mut rl = RegionLoader::init(0x5, 1024);
        assert_eq!(rl.report().state, RegionState::Naked);
        let p = payload(0x5, &[(0, 0x1, 1)], 4);
        // candidate -> committing
        assert_eq!(rl.candidate(&p).unwrap(), 1);
        assert_eq!(rl.state, RegionState::Committing);
        // commit -> provisioned, installed slot 1
        let resp = rl.commit(&p).unwrap();
        assert_eq!(resp.state, RegionState::Provisioned);
        assert_eq!(rl.installed_slot, 1);
        // recompute for the same slot -> provisioned
        assert_eq!(rl.recompute_state(&p).unwrap(), RegionState::Provisioned);
        // rollback -> naked
        let rb = rl.rollback();
        assert_eq!(rb.state, RegionState::Rollback);
        assert_eq!(rl.state, RegionState::Naked);
        assert_eq!(rl.used, 0);
    }

    #[test]
    fn region_full_raises_trap_26() {
        let mut rl = RegionLoader::init(0x5, 4); // tiny region
        let p = payload(0x5, &[(0, 0x1, 1)], 4);
        assert_eq!(rl.candidate(&p), Err(RegionError::RegionFull));
        assert_eq!(REGION_EXHAUSTED_TRAP, 26);
    }

    #[test]
    fn region_msg_roundtrip() {
        let req = RegionReq {
            set_id: 0xabc,
            lane: 2,
            slot: 5,
            state: RegionState::Committing,
        };
        for msg in [
            RegionMsg::Candidate(req),
            RegionMsg::Rollback(req),
            RegionMsg::Resp(RegionResp {
                state: RegionState::Provisioned,
                slot: 5,
                region_free: 42,
            }),
        ] {
            let enc = encode_region_msg(&msg);
            assert_eq!(decode_region_msg(&enc), Some(msg));
        }
    }

    #[test]
    fn keys_to_region_request_roundtrip() {
        let req = keys_to_region_request(2, 0x5, RegionState::Naked).unwrap();
        assert_eq!(region_request_to_keys(&req), (2, 0x5));
    }

    /// G10 no-panic audit (host-input paths): every byte pattern up to a
    /// modest length must be handled by `parse_set_table`/`decode_region_msg`
    /// with an `Err`/`None`, never a panic.
    #[test]
    fn host_input_parse_paths_never_panic() {
        for len in 0..64usize {
            for mut seed in 0..=255u8 {
                let bytes: Vec<u8> = (0..len).map(|i| seed.wrapping_add(i as u8)).collect();
                let _ = parse_set_table(&bytes);
                let _ = decode_region_msg(&bytes);
                seed = seed.wrapping_mul(131);
            }
        }
        // A valid-but-hostile truncated payload must error, not panic.
        let mut payload = payload(0x5, &[(0, 0x1, 1)], 4);
        let full_len = payload.len();
        for cut in 0..full_len {
            payload.truncate(cut);
            assert!(parse_set_table(&payload).is_err());
        }
    }
}