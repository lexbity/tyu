//! Flat global symbol map for the runtime loader.
//!
//! Canonical definition: abi-contract.md §6; module-format-and-loading.md §7.
//!
//! The loader maintains a flat global map of all symbols exported by loaded
//! modules.  Resolution uses `fnv1a_u64(name)` as the key.  The map rejects:
//!
//! - **Duplicate names** (`E_SYMBOL_CONFLICT 5206`): two modules export the
//!   same word name.
//! - **Hash collisions** (`E_SYMBOL_HASH_COLLISION 5207`): distinct names
//!   whose `fnv1a_u64` hashes collide (detected via retained name table).

use lmod::hash::fnv1a_u64;

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

/// Error codes matching the module-format error band `52xx`.
pub type SymError = u32;

pub const E_SYMBOL_CONFLICT: SymError = 5206;
pub const E_SYMBOL_HASH_COLLISION: SymError = 5207;

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// A single registered symbol in the global map.
#[derive(Clone, Debug)]
pub struct SymEntry<'a> {
    /// FNV-1a 64-bit hash of the symbol name.
    pub hash: u64,
    /// The symbol name (retained for collision detection and diagnostics).
    pub name: &'a [u8],
    /// Address (or offset) of the symbol in the loaded module's memory.
    pub addr: usize,
}

// ---------------------------------------------------------------------------
// Flat global symbol map
// ---------------------------------------------------------------------------

/// A fixed-capacity flat global symbol map.
///
/// Capacity is set at construction time (statically sized for no_std).
/// Lookup is O(n) — acceptable for the small symbol counts in embedded
/// firmware (< 256 symbols).
pub struct SymMap<'a, const N: usize> {
    pub(crate) entries: [Option<SymEntry<'a>>; N],
    pub(crate) len: usize,
}

impl<'a, const N: usize> SymMap<'a, N> {
    /// Create an empty symbol map.
    pub fn new() -> Self {
        // Can't be const because of lifetime parameter.
        // Use a manual init approach.
        const INIT: Option<SymEntry<'_>> = None;
        Self {
            entries: [INIT; N],
            len: 0,
        }
    }

    /// The number of registered symbols.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if no symbols have been registered.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Register a symbol.
    ///
    /// Returns `Err(E_SYMBOL_CONFLICT)` if a symbol with the same *name*
    /// already exists.  Returns `Err(E_SYMBOL_HASH_COLLISION)` if a
    /// different name produces the same hash.
    pub fn register(&mut self, name: &'a [u8], addr: usize) -> Result<(), SymError> {
        let hash = fnv1a_u64(name);
        self.register_with_hash(hash, name, addr)
    }

    /// Register a symbol with a precomputed hash.
    ///
    /// This is used for generated runtime symbol tables, where the on-device
    /// table stores `{sym_hash, addr}` and does not carry names. When `name`
    /// is empty, duplicate-name detection is skipped but duplicate hashes are
    /// still rejected.
    pub fn register_with_hash(
        &mut self,
        hash: u64,
        name: &'a [u8],
        addr: usize,
    ) -> Result<(), SymError> {
        // Check for conflicts and collisions.
        for i in 0..self.len {
            let Some(ref existing) = self.entries[i] else {
                continue;
            };
            if !name.is_empty() && !existing.name.is_empty() && existing.name == name {
                return Err(E_SYMBOL_CONFLICT);
            }
            if existing.hash == hash {
                return Err(E_SYMBOL_HASH_COLLISION);
            }
        }

        if self.len >= N {
            return Err(5209); // map full (not in spec band but practical)
        }

        self.entries[self.len] = Some(SymEntry { hash, name, addr });
        self.len += 1;
        Ok(())
    }

    /// Register one entry from a generated `.lang.symtab`.
    pub fn register_runtime_hash(&mut self, hash: u64, addr: usize) -> Result<(), SymError> {
        self.register_with_hash(hash, &[], addr)
    }

    /// Register every entry from a generated `.lang.symtab` byte range.
    ///
    /// Layout: `u32 count`, `u32 _pad`, followed by `count` repetitions of
    /// `{u64 sym_hash, u64 addr}`, all little-endian.
    pub fn register_symtab_bytes(&mut self, bytes: &[u8]) -> Result<usize, SymError> {
        if bytes.len() < 8 {
            return Err(E_SYMBOL_CONFLICT);
        }
        let count =
            u32::from_le_bytes(bytes[0..4].try_into().map_err(|_| E_SYMBOL_CONFLICT)?) as usize;
        let expected_len = 8usize
            .checked_add(count.checked_mul(16).ok_or(E_SYMBOL_CONFLICT)?)
            .ok_or(E_SYMBOL_CONFLICT)?;
        if bytes.len() < expected_len {
            return Err(E_SYMBOL_CONFLICT);
        }

        for i in 0..count {
            let off = 8 + i * 16;
            let hash = u64::from_le_bytes(
                bytes[off..off + 8]
                    .try_into()
                    .map_err(|_| E_SYMBOL_CONFLICT)?,
            );
            let addr = u64::from_le_bytes(
                bytes[off + 8..off + 16]
                    .try_into()
                    .map_err(|_| E_SYMBOL_CONFLICT)?,
            ) as usize;
            self.register_runtime_hash(hash, addr)?;
        }

        Ok(count)
    }

    /// Register all exported symbols from a `.lmod` container's modinfo.
    ///
    /// Each export is registered with its `value_off` as the address
    /// (interpretation depends on the loader — typically an offset from
    /// the module's load base).
    ///
    /// Returns the number of symbols registered, or the first error.
    pub fn register_from_modinfo(
        &mut self,
        modinfo_data: &'a [u8],
        base_addr: usize,
    ) -> Result<usize, SymError> {
        let mi = lmod::modinfo::decode(modinfo_data).ok_or(E_SYMBOL_CONFLICT)?;
        let count = mi.export_count;
        for i in 0..count {
            let exp = lmod::modinfo::read_export(modinfo_data, i).ok_or(E_SYMBOL_CONFLICT)?;
            let addr = base_addr.wrapping_add(exp.value_off as usize);
            self.register_with_hash(exp.sym_hash, exp.name, addr)?;
        }
        Ok(count as usize)
    }

    /// Look up a symbol by name.  Returns `None` if not found.
    pub fn lookup_by_name(&self, name: &[u8]) -> Option<&SymEntry<'a>> {
        let hash = fnv1a_u64(name);
        self.lookup_by_hash(hash).filter(|e| e.name == name)
    }

    /// Look up a symbol by its FNV-1a 64-bit hash.
    ///
    /// Because hash collisions are possible (though extremely unlikely at
    /// u64 width), this returns the *first* matching entry.  The caller
    /// should verify the name via `SymEntry.name` if exactness is critical.
    pub fn lookup_by_hash(&self, hash: u64) -> Option<&SymEntry<'a>> {
        for i in 0..self.len {
            if let Some(ref entry) = self.entries[i] {
                if entry.hash == hash {
                    return Some(entry);
                }
            }
        }
        None
    }

    /// Iterate over all registered entries.
    pub fn iter(&self) -> impl Iterator<Item = &SymEntry<'a>> {
        self.entries[..self.len].iter().filter_map(|e| e.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn empty_map() {
        let map: SymMap<'_, 8> = SymMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn register_and_lookup() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        map.register(b"main", 0x1000).unwrap();
        map.register(b"helper", 0x2000).unwrap();

        assert_eq!(map.len(), 2);

        let main = map.lookup_by_name(b"main").unwrap();
        assert_eq!(main.addr, 0x1000);

        let hash = fnv1a_u64(b"main");
        let by_hash = map.lookup_by_hash(hash).unwrap();
        assert_eq!(by_hash.name, b"main");
    }

    #[test]
    fn lookup_missing_returns_none() {
        let map: SymMap<'_, 8> = SymMap::new();
        assert!(map.lookup_by_name(b"nonexistent").is_none());
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        map.register(b"main", 0x1000).unwrap();
        let err = map.register(b"main", 0x2000).unwrap_err();
        assert_eq!(err, E_SYMBOL_CONFLICT);
    }

    #[test]
    fn prehashed_runtime_entry_resolves_by_hash() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        let hash = 0xaccb_676a_903a_06d9;
        map.register_runtime_hash(hash, 0x1234).unwrap();

        let entry = map.lookup_by_hash(hash).unwrap();
        assert_eq!(entry.addr, 0x1234);
        assert_eq!(entry.name, b"");
    }

    #[test]
    fn prehashed_duplicate_hash_rejected() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        let hash = 0xaccb_676a_903a_06d9;
        map.register_runtime_hash(hash, 0x1234).unwrap();
        let err = map.register_runtime_hash(hash, 0x5678).unwrap_err();
        assert_eq!(err, E_SYMBOL_HASH_COLLISION);
    }

    #[test]
    fn register_symtab_bytes_populates_runtime_hashes() {
        let hash_a = 0x1111_2222_3333_4444u64;
        let addr_a = 0x5555_6666_7777_8888u64;
        let hash_b = 0x9999_aaaa_bbbb_ccccu64;
        let addr_b = 0xdddd_eeee_ffff_0000u64;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&hash_a.to_le_bytes());
        bytes.extend_from_slice(&addr_a.to_le_bytes());
        bytes.extend_from_slice(&hash_b.to_le_bytes());
        bytes.extend_from_slice(&addr_b.to_le_bytes());

        let mut map: SymMap<'_, 8> = SymMap::new();
        let count = map.register_symtab_bytes(&bytes).unwrap();

        assert_eq!(count, 2);
        assert_eq!(map.lookup_by_hash(hash_a).unwrap().addr, addr_a as usize);
        assert_eq!(map.lookup_by_hash(hash_b).unwrap().addr, addr_b as usize);
    }

    #[test]
    fn forged_hash_entry_triggers_collision_error() {
        // Inject a forged entry whose hash matches fnv1a_u64(b"gamma")
        // but whose name is different.  Registering b"gamma" must
        // detect the hash collision.
        let mut map: SymMap<'_, 8> = SymMap::new();
        let h = fnv1a_u64(b"gamma");
        map.entries[0] = Some(SymEntry {
            hash: h,
            name: b"not-gamma",
            addr: 0x100,
        });
        map.len = 1;

        let err = map.register(b"gamma", 0x200).unwrap_err();
        assert_eq!(err, E_SYMBOL_HASH_COLLISION);
    }

    #[test]
    fn hash_collision_constant_is_defined() {
        assert_eq!(E_SYMBOL_HASH_COLLISION, 5207);
    }

    #[test]
    fn symmap_capacity_exceeded() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        map.register(b"a", 0).unwrap();
        map.register(b"b", 1).unwrap();
        map.register(b"c", 2).unwrap();
        map.register(b"d", 3).unwrap();
        map.register(b"e", 4).unwrap();
        map.register(b"f", 5).unwrap();
        map.register(b"g", 6).unwrap();
        map.register(b"h", 7).unwrap();
        let err = map.register(b"overflow", 999).unwrap_err();
        assert_eq!(err, 5209);
    }

    #[test]
    fn symmap_capacity_exact_fit() {
        let mut map: SymMap<'_, 8> = SymMap::new();
        map.register(b"a", 0).unwrap();
        map.register(b"b", 1).unwrap();
        map.register(b"c", 2).unwrap();
        map.register(b"d", 3).unwrap();
        map.register(b"e", 4).unwrap();
        map.register(b"f", 5).unwrap();
        map.register(b"g", 6).unwrap();
        map.register(b"h", 7).unwrap();
        assert_eq!(map.len(), 8);
    }

    #[test]
    fn distinct_names_no_collision() {
        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"abc", 0x100).unwrap();
        assert_ne!(fnv1a_u64(b"abc"), fnv1a_u64(b"xyz"));
        assert!(map.register(b"xyz", 0x200).is_ok());
    }

    #[test]
    fn iterate_entries() {
        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"a", 1).unwrap();
        map.register(b"b", 2).unwrap();
        map.register(b"c", 3).unwrap();

        let names: Vec<&[u8]> = map.iter().map(|e| e.name).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&&b"a"[..]));
        assert!(names.contains(&&b"b"[..]));
        assert!(names.contains(&&b"c"[..]));
    }

    #[test]
    fn map_full_error() {
        let mut map: SymMap<'_, 2> = SymMap::new();
        map.register(b"a", 1).unwrap();
        map.register(b"b", 2).unwrap();
        let err = map.register(b"c", 3).unwrap_err();
        assert_eq!(err, 5209);
    }

    #[test]
    fn register_from_modinfo_populates_map() {
        let exports = [
            lmod::modinfo::ExportEntry {
                sym_hash: fnv1a_u64(b"foo"),
                name: b"foo",
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            },
            lmod::modinfo::ExportEntry {
                sym_hash: fnv1a_u64(b"bar"),
                name: b"bar",
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            },
        ];
        let mut buf = [0u8; 768];
        let size =
            lmod::modinfo::encode_into(&mut buf, b"MyMod", &exports, &[], 42, 0, &[]).unwrap();
        let modinfo = &buf[..size];

        let mut map: SymMap<'_, 8> = SymMap::new();
        let n = map.register_from_modinfo(modinfo, 0x1000).unwrap();
        assert_eq!(n, 2);

        // value_off is a byte offset into the modinfo data; when added to
        // base_addr it gives the address of the word_meta entry.
        assert!(map.lookup_by_name(b"foo").is_some());
        assert!(map.lookup_by_name(b"bar").is_some());

        // Each entry gets a distinct value_off (different word_meta slots).
        let foo = map.lookup_by_name(b"foo").unwrap();
        let bar = map.lookup_by_name(b"bar").unwrap();
        assert_ne!(foo.addr, bar.addr);
    }

    #[test]
    fn register_from_modinfo_detects_duplicate() {
        let exports = [lmod::modinfo::ExportEntry {
            sym_hash: fnv1a_u64(b"dup"),
            name: b"dup",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }];
        let mut buf = [0u8; 256];
        let size = lmod::modinfo::encode_into(&mut buf, b"M", &exports, &[], 0, 0, &[]).unwrap();

        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"dup", 0x100).unwrap();
        // Registering the same modinfo should detect the duplicate.
        let err = map.register_from_modinfo(&buf[..size], 0x200).unwrap_err();
        assert_eq!(err, E_SYMBOL_CONFLICT);
    }
}
