//! P6 loader binding pass (design doc §5.8, decisions D-4/D-5/D-9/D-12).
//!
//! The loader enforces board identity and binds a module's MMIO apertures
//! *before* any symbol resolution:
//!
//!   1. `modinfo_ver == MODINFO_VER` else `E5224` (v3 on a v4 loader — the
//!      first check, before any allocation; no shim, no reinterpretation).
//!   2. module `platform_hash` matches the board's compiled descriptor hash
//!      else `E5220` (D-5: board-exact). Hash `0` is the unplatformed-module
//!      sentinel (such a module cannot contain MMIO — E3640) and is allowed.
//!   3. For each aperture-use entry: the name matches a board aperture by
//!      `name_hash` and its size agrees else `E5222`; its fused access mask
//!      is a subset of the board's declared aperture capability else `E5223`;
//!      and the aperture is not already bound by a live module else `E5221`.
//!      The table itself is structurally validated (bounds, duplicate ids)
//!      else `E5223`.
//!   4. The bound apertures are reserved in a fixed-capacity registry
//!      (transactional: a failed load rolls its reservations back).
//!   5. The resolved per-aperture bases are returned for the reloc pass
//!      (`apply_import_relocations` writes them into `MmioApertureBase` sites).
//!
//! Pure byte logic, `no_std`, panic-free on host-input paths (G10 discipline).

use crate::error::LoadError;
use crate::platform::BoardAperture;
use lmod::board_table::APERTURE_CAP as BOARD_APERTURE_CAP;
use lmod::modinfo;

/// Re-exported aperture-table capacity (shared with the module/board tables).
pub use lmod::board_table::APERTURE_CAP as APERTURE_TABLE_CAP;

/// One aperture bound for a module during a load: the board-resolved base the
/// reloc pass writes into `MmioApertureBase` sites, plus the module-local id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApertureBinding {
    /// Module-local aperture id (equals the descriptor aperture id).
    pub id: u16,
    /// The board's base for this aperture, to write into reloc sites.
    pub base: u32,
}

/// Transactional exclusivity registry (decision D-9): which apertures are bound
/// to live modules. Fixed capacity matching the module/board table cap; no
/// heap in `loader-core`.
///
/// Reservations are keyed by board aperture id (the identity the loader binds
/// against) and record the owning module's `abi_hash` so they can be released
/// on unload. `mark`/`rollback` give the load path a cheap transaction
/// boundary: a failed load rolls its reservations back, leaving the registry
/// exactly as before the load began.
#[derive(Debug)]
pub struct ApertureRegistry {
    /// Bound aperture ids, sorted ascending, dense by insertion order.
    ids: [u16; BOARD_APERTURE_CAP],
    /// Owning module `abi_hash` per bound id.
    owners: [u64; BOARD_APERTURE_CAP],
    count: usize,
}

impl ApertureRegistry {
    pub const fn new() -> Self {
        Self {
            ids: [0; BOARD_APERTURE_CAP],
            owners: [0; BOARD_APERTURE_CAP],
            count: 0,
        }
    }

    /// True if `id` is currently bound to a live module.
    pub fn is_bound(&self, id: u16) -> bool {
        self.ids[..self.count].contains(&id)
    }

    /// The number of currently-bound apertures (observability).
    pub fn bound_count(&self) -> usize {
        self.count
    }

    /// A transaction mark: the registry state before a load's reservations.
    /// `rollback` restores exactly this state on failure.
    pub fn mark(&self) -> usize {
        self.count
    }

    /// Roll back to a previously captured mark (releasing any reservations
    /// made after it). No-op if `mark` is stale.
    pub fn rollback(&mut self, mark: usize) {
        if mark <= self.count {
            self.count = mark;
        }
    }

    /// Reserve `id` for module `owner`. Fails `ApertureConflict` if already bound.
    pub fn reserve(&mut self, id: u16, owner: u64) -> Result<(), LoadError> {
        if self.is_bound(id) {
            return Err(LoadError::ApertureConflict);
        }
        if self.count >= BOARD_APERTURE_CAP {
            return Err(LoadError::ApertureConflict);
        }
        self.ids[self.count] = id;
        self.owners[self.count] = owner;
        self.count += 1;
        Ok(())
    }

    /// Release every aperture bound to module `owner` (module unload). Allowed
    /// to be absent from the loaded set — idempotent.
    pub fn release_owner(&mut self, owner: u64) {
        let mut w = 0;
        while w < self.count {
            if self.owners[w] == owner {
                // Swap-remove.
                self.ids[w] = self.ids[self.count - 1];
                self.owners[w] = self.owners[self.count - 1];
                self.count -= 1;
            } else {
                w += 1;
            }
        }
    }
}

impl Default for ApertureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse and validate the module's aperture-use table against the board, and
/// reserve each bound aperture in `registry` (transactional).
///
/// Returns the per-aperture binding table (`id -> base`, indexed by module
/// aperture id; ids not in the table carry base 0) on success. On failure the
/// registry is left exactly as it was — the caller rolls back via
/// [`ApertureRegistry::rollback`] using the mark taken before this call.
pub fn bind_apertures(
    modinfo_data: &[u8],
    board_hash: Option<u64>,
    board: &[BoardAperture],
    registry: &mut ApertureRegistry,
    owner: u64,
) -> Result<[u32; BOARD_APERTURE_CAP], LoadError> {
    if modinfo_data.is_empty() {
        // No modinfo: a module with no board claims binds nothing. An empty
        // modinfo cannot carry apertures or a hash.
        return Ok([0; BOARD_APERTURE_CAP]);
    }

    // 1. Version gate (D-12) — before anything structural is trusted.
    let hdr = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
    if hdr.modinfo_ver != modinfo::MODINFO_VER {
        return Err(LoadError::ModinfoVersionUnsupported);
    }

    // 2. Board identity (D-5).
    let module_hash = hdr.platform_hash;
    if module_hash != 0 {
        match board_hash {
            Some(board) if board == module_hash => {}
            _ => return Err(LoadError::PlatformHashMismatch),
        }
    }

    let aperture_count = hdr.aperture_count;
    if aperture_count == 0 {
        return Ok([0; BOARD_APERTURE_CAP]);
    }
    if aperture_count as usize > BOARD_APERTURE_CAP {
        return Err(LoadError::ApertureTableMalformed);
    }

    // 3. Structural validation of the aperture-use table (E5223): every entry
    //    must decode within bounds and ids must be unique.
    let mut seen = [0u16; BOARD_APERTURE_CAP];
    let mut seen_count = 0usize;
    for i in 0..aperture_count {
        let entry = lmod::modinfo::read_aperture_use(modinfo_data, i)
            .ok_or(LoadError::ApertureTableMalformed)?;
        for s in &seen[..seen_count] {
            if *s == entry.aperture_id {
                return Err(LoadError::ApertureTableMalformed);
            }
        }
        if seen_count < BOARD_APERTURE_CAP {
            seen[seen_count] = entry.aperture_id;
            seen_count += 1;
        }
    }

    // 4. Per-entry resolution + reservation.
    let mut bases = [0u32; BOARD_APERTURE_CAP];
    for i in 0..aperture_count {
        let entry = lmod::modinfo::read_aperture_use(modinfo_data, i)
            .ok_or(LoadError::ApertureTableMalformed)?;
        let id = entry.aperture_id as usize;
        if id >= board.len() {
            return Err(LoadError::ApertureUnresolved);
        }
        let board_aperture = &board[id];
        // The board aperture's id must equal the module's claimed id.
        if board_aperture.name_hash != entry.name_hash {
            return Err(LoadError::ApertureUnresolved);
        }
        if board_aperture.size != entry.size {
            return Err(LoadError::ApertureUnresolved);
        }
        if entry.access_mask & !board_aperture.capability != 0 {
            return Err(LoadError::ApertureTableMalformed);
        }
        // Exclusivity (D-9).
        registry.reserve(entry.aperture_id, owner)?;
        bases[entry.aperture_id as usize] = board_aperture.base;
    }
    Ok(bases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lmod::board_table::{BoardTable, BoardAperture, APERTURE_CAP as BOARD_APERTURE_CAP};
    use lmod::hash::fnv1a_u64;
    use lmod::modinfo::{encode_into, ApertureUseEntry, ACCESS_READ, ACCESS_WRITE};

    fn board() -> BoardTable {
        let mut t = BoardTable::empty();
        t.platform_hash = 0xabc;
        t.aperture_count = 2;
        t.apertures[0] = BoardAperture {
            name_hash: fnv1a_u64(b"apb"),
            base: 0x4000_0000,
            size: 0x1_0000,
            capability: ACCESS_READ | ACCESS_WRITE,
        };
        t.apertures[1] = BoardAperture {
            name_hash: fnv1a_u64(b"scratch"),
            base: 0x2000_0000,
            size: 0x1000,
            capability: ACCESS_READ | ACCESS_WRITE,
        };
        t
    }

    fn modinfo_with(platform_hash: u64, apertures: &[ApertureUseEntry]) -> alloc::vec::Vec<u8> {
        let mut buf = [0u8; 1024];
        let n = encode_into(&mut buf, b"T", &[], &[], 0, 0, &[], platform_hash, apertures).unwrap();
        buf[..n].to_vec()
    }

    #[test]
    fn matching_module_binds_all_apertures() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"apb"),
                size: 0x1_0000,
                aperture_id: 0,
                access_mask: ACCESS_READ | ACCESS_WRITE,
            }],
        );
        let mut reg = ApertureRegistry::new();
        let bases = bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap();
        assert_eq!(bases[0], 0x4000_0000);
        assert_eq!(reg.bound_count(), 1);
        assert!(reg.is_bound(0));
    }

    #[test]
    fn v3_module_rejected_e5224() {
        let mut mi = modinfo_with(0xabc, &[]);
        // Force the version field to 3 (a v3 artifact).
        mi[4..6].copy_from_slice(&3u16.to_le_bytes());
        let b = board();
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::ModinfoVersionUnsupported
        );
    }

    #[test]
    fn hash_mismatch_rejected_e5220() {
        let b = board();
        let mi = modinfo_with(0xdef, &[]);
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::PlatformHashMismatch
        );
    }

    #[test]
    fn unplatformed_module_loads_anywhere() {
        let b = board();
        let mi = modinfo_with(0, &[]);
        let mut reg = ApertureRegistry::new();
        let bases = bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap();
        assert_eq!(bases, [0; BOARD_APERTURE_CAP]);
        assert_eq!(reg.bound_count(), 0);
    }

    #[test]
    fn unexposed_aperture_rejected_e5222() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"ghost"),
                size: 0x1000,
                aperture_id: 0,
                access_mask: ACCESS_READ,
            }],
        );
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::ApertureUnresolved
        );
        assert_eq!(reg.bound_count(), 0);
    }

    #[test]
    fn size_drift_rejected_e5222() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"apb"),
                size: 0x2000, // board says 0x10000
                aperture_id: 0,
                access_mask: ACCESS_READ,
            }],
        );
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::ApertureUnresolved
        );
    }

    #[test]
    fn access_beyond_capability_rejected_e5223() {
        let b = board(); // no w1c capability declared
        let mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"apb"),
                size: 0x1_0000,
                aperture_id: 0,
                access_mask: lmod::modinfo::ACCESS_W1C,
            }],
        );
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::ApertureTableMalformed
        );
    }

    #[test]
    fn duplicate_aperture_id_rejected_e5223() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"apb"),
                    size: 0x1_0000,
                    aperture_id: 0,
                    access_mask: ACCESS_READ,
                },
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"apb"),
                    size: 0x1_0000,
                    aperture_id: 0,
                    access_mask: ACCESS_WRITE,
                },
            ],
        );
        let mut reg = ApertureRegistry::new();
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap_err(),
            LoadError::ApertureTableMalformed
        );
    }

    #[test]
    fn exclusivity_conflict_e5221() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"apb"),
                size: 0x1_0000,
                aperture_id: 0,
                access_mask: ACCESS_READ,
            }],
        );
        let mut reg = ApertureRegistry::new();
        bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap();
        // Second module, same aperture.
        assert_eq!(
            bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 2).unwrap_err(),
            LoadError::ApertureConflict
        );
    }

    #[test]
    fn failed_load_rolls_back_reservations() {
        let b = board();
        let ok_mi = modinfo_with(
            0xabc,
            &[ApertureUseEntry {
                name_hash: fnv1a_u64(b"apb"),
                size: 0x1_0000,
                aperture_id: 0,
                access_mask: ACCESS_READ,
            }],
        );
        // A module with aperture 0 ok then aperture 1 failing (size drift).
        let bad_mi = modinfo_with(
            0xabc,
            &[
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"apb"),
                    size: 0x1_0000,
                    aperture_id: 0,
                    access_mask: ACCESS_READ,
                },
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"scratch"),
                    size: 0x9999, // drift
                    aperture_id: 1,
                    access_mask: ACCESS_READ,
                },
            ],
        );
        let mut reg = ApertureRegistry::new();
        let mark = reg.mark();
        let err = bind_apertures(&bad_mi, Some(0xabc), b.apertures(), &mut reg, 7).unwrap_err();
        assert_eq!(err, LoadError::ApertureUnresolved);
        reg.rollback(mark);
        assert_eq!(reg.bound_count(), 0, "failed load must leave no reservations");
        // A subsequent conflicting load now succeeds (transactionality proof).
        bind_apertures(&ok_mi, Some(0xabc), b.apertures(), &mut reg, 1).unwrap();
        assert!(reg.is_bound(0));
    }

    #[test]
    fn release_owner_frees_apertures() {
        let b = board();
        let mi = modinfo_with(
            0xabc,
            &[
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"apb"),
                    size: 0x1_0000,
                    aperture_id: 0,
                    access_mask: ACCESS_READ,
                },
                ApertureUseEntry {
                    name_hash: fnv1a_u64(b"scratch"),
                    size: 0x1000,
                    aperture_id: 1,
                    access_mask: ACCESS_READ,
                },
            ],
        );
        let mut reg = ApertureRegistry::new();
        bind_apertures(&mi, Some(0xabc), b.apertures(), &mut reg, 9).unwrap();
        assert_eq!(reg.bound_count(), 2);
        reg.release_owner(9);
        assert_eq!(reg.bound_count(), 0);
        assert!(!reg.is_bound(0));
        assert!(!reg.is_bound(1));
    }
}