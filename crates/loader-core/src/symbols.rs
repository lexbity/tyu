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

        // Check for conflicts and collisions.
        for i in 0..self.len {
            let Some(ref existing) = self.entries[i] else {
                continue;
            };
            if existing.name == name {
                return Err(E_SYMBOL_CONFLICT);
            }
            if existing.hash == hash {
                return Err(E_SYMBOL_HASH_COLLISION);
            }
        }

        if self.len >= N {
            return Err(5209); // map full (not in spec band but practical)
        }

        self.entries[self.len] = Some(SymEntry {
            hash,
            name,
            addr,
        });
        self.len += 1;
        Ok(())
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
            self.register(exp.name, addr)?;
        }
        Ok(count as usize)
    }

    /// Look up a symbol by name.  Returns `None` if not found.
    pub fn lookup_by_name(&self, name: &[u8]) -> Option<&SymEntry<'a>> {
        let hash = fnv1a_u64(name);
        self.lookup_by_hash(hash)
            .filter(|e| e.name == name)
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
        let size = lmod::modinfo::encode_into(&mut buf, b"MyMod", &exports, &[], 42, 0, &[]).unwrap();
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
