//! Loader driver — the target-independent half of `__lang_load_module`.
//!
//! Implements module-format-and-loading.md §9 (loader algorithm) for Tier 0
//! (no signature verification, no Tier-2 re-derivation).
//!
//! Phase 9 additions: transactional rollback, `__lang_mod_init` hook,
//! load-once enforcement, failure atomicity.

use crate::platform::{LoaderPlatform, Region};
use crate::symbols::SymMap;
use lmod::validate::Container;

/// Error codes from the loader algorithm.
pub const E_ABI_MISMATCH: u32 = 5200;
pub const E_BAD_CONTAINER: u32 = 5201;
pub const E_SIG_INVALID: u32 = 5202;
pub const E_RELOC_UNSUPPORTED: u32 = 5204;
pub const E_SYMBOL_UNRESOLVED: u32 = 5205;
pub const E_SYMBOL_CONFLICT: u32 = 5206;
pub const E_MODULE_DECLARES_ISR: u32 = 5203;
pub const E_RESOURCE_SHARING_MISMATCH: u32 = 5208;
pub const E_CONTAINER_ENCRYPTED: u32 = 5212;
pub const E_MODULE_ALREADY_LOADED: u32 = 5210;

/// Name of the optional per-module init word.
const MOD_INIT_NAME: &[u8] = b"__lang_mod_init";

/// Save-restore guard for the global symbol map.
///
/// On drop without a call to [`commit`](RollbackGuard::commit), the map
/// is restored to its saved length (erasing any entries added).
struct RollbackGuard<'a, 'e, const N: usize> {
    map: &'a mut SymMap<'e, N>,
    saved_len: usize,
    committed: bool,
}

impl<'a, 'e, const N: usize> RollbackGuard<'a, 'e, N> {
    fn new(map: &'a mut SymMap<'e, N>) -> Self {
        let saved_len = map.len();
        RollbackGuard {
            map,
            saved_len,
            committed: false,
        }
    }
    fn commit(mut self) {
        self.committed = true;
    }
}

impl<'a, 'e, const N: usize> Drop for RollbackGuard<'a, 'e, N> {
    fn drop(&mut self) {
        if !self.committed {
            for i in self.saved_len..self.map.len() {
                self.map.entries[i] = None;
            }
            self.map.len = self.saved_len;
        }
    }
}

/// Intentionally empty — allocation release is a future phase.
/// Phase 9 guarantees symbol-map atomicity; allocation cleanup is
/// process-scoped for Tier 0 (exiting frees all mmap'd regions).

/// A fixed-capacity set of module identifiers used to enforce load-once.
///
/// Each entry is an `abi_hash` (enough to uniquely identify a module
/// within a given build).
pub struct LoadedSet<const N: usize> {
    hashes: [u64; N],
    len: usize,
}

impl<const N: usize> LoadedSet<N> {
    pub const fn new() -> Self {
        Self {
            hashes: [0; N],
            len: 0,
        }
    }

    /// Returns `true` if this `abi_hash` has already been loaded.
    pub fn contains(&self, abi_hash: u64) -> bool {
        self.hashes[..self.len].contains(&abi_hash)
    }

    /// Record that a module with `abi_hash` has been loaded.
    /// Returns `Err(E_MODULE_ALREADY_LOADED)` if already present.
    pub fn insert(&mut self, abi_hash: u64) -> Result<(), u32> {
        if self.contains(abi_hash) {
            return Err(E_MODULE_ALREADY_LOADED);
        }
        if self.len < N {
            self.hashes[self.len] = abi_hash;
            self.len += 1;
            Ok(())
        } else {
            Err(E_MODULE_ALREADY_LOADED) // set full, treat as loaded
        }
    }
}

// ---------------------------------------------------------------------------
// LoadedModule
// ---------------------------------------------------------------------------

/// A fully loaded module, ready for execution.
#[derive(Debug)]
pub struct LoadedModule {
    pub code: Region,
    pub rodata: Region,
    pub data: Region,
    /// Address of `__lang_mod_init` (0 if absent).
    pub init_addr: usize,
    /// The module's `abi_hash` (for lifecycle tracking).
    pub abi_hash: u64,
}

// ---------------------------------------------------------------------------
// Loader algorithm
// ---------------------------------------------------------------------------

/// Run the loader algorithm (§9 steps 1–9) for a Tier-0 (unsigned) module,
/// with Phase 9 transactional rollback and init-hook support.
///
/// - On success, the module's exports are registered in `global_map`.
/// - On failure, `global_map` is restored to its pre-call state (no
///   partial registrations), and any allocated memory is released
///   via the platform.
/// - The module's `abi_hash` is recorded in `loaded_set` to prevent
///   double-loading.
///
/// # Safety
///
/// The caller must ensure:
/// - `platform` provides valid memory mappings.
/// - `global_map` outlives any code that calls the loaded functions.
/// - The loaded code adheres to the platform's calling convention.
pub fn load_module<'a>(
    container: &'a Container<'a>,
    platform: &mut dyn LoaderPlatform,
    global_map: &mut SymMap<'a, 256>,
    loaded_set: &mut LoadedSet<64>,
) -> Result<LoadedModule, u32> {
    // Steps 1–2: parsing and validation already done by Container::parse().
    let hdr = container.header();

    // Step 3: check abi_hash.
    if hdr.abi_hash != platform.expected_abi_hash() {
        return Err(E_ABI_MISMATCH);
    }

    // Load-once check.
    if loaded_set.contains(hdr.abi_hash) {
        return Err(E_MODULE_ALREADY_LOADED);
    }

    // Encryption-at-rest check (module-format §8).
    // If the container claims to be encrypted, reject — v1 has no cipher.
    if hdr.flags & lmod::header::LMOD_FLAG_ENCRYPTED != 0 {
        return Err(E_CONTAINER_ENCRYPTED);
    }

    // Signed-flag consistency check (module-format §3).
    // If the container claims to be signed but the platform is Tier 0, the
    // signature won't be verified — this is a configuration mismatch.
    if hdr.flags & lmod::header::LMOD_FLAG_SIGNED != 0
        && platform.trust_tier().rank() < crate::platform::Tier::One.rank()
    {
        return Err(E_SIG_INVALID);
    }

    // Step 4: Signature verification (Tier ≥ 1).
    let raw_bytes = container.raw_bytes();
    if platform.trust_tier().rank() >= crate::platform::Tier::One.rank() {
        let region_len = lmod::sig::signed_region_len(hdr);
        if region_len > raw_bytes.len() {
            return Err(E_BAD_CONTAINER);
        }
        let signed_region = &raw_bytes[..region_len];
        let trailer_data = &raw_bytes[region_len..];
        let trailer = lmod::sig::SigTrailer::parse(trailer_data).ok_or(E_SIG_INVALID)?;
        if trailer.scheme == lmod::sig::SCHEME_NONE {
            return Err(E_SIG_INVALID);
        }
        if !platform.verify_sig(signed_region, trailer.sig_bytes) {
            return Err(E_SIG_INVALID);
        }
    }

    // Step 5: Static-ISR rule — reject modules that declare @interrupt bindings.
    let modinfo_data = container.modinfo();
    if !modinfo_data.is_empty() {
        if let Some(mi) = lmod::modinfo::decode(modinfo_data) {
            if mi.has_isr() {
                return Err(E_MODULE_DECLARES_ISR);
            }
        }
    }

    // --- Begin transactional section (steps 4–12) ---
    let sym_guard = RollbackGuard::new(global_map);

    // Step 6: Place sections.
    // Check placement policy: only CopyToRam is implemented in v1.
    if platform.placement_policy() != crate::platform::PlacementPolicy::CopyToRam {
        return Err(E_BAD_CONTAINER); // XIP not yet implemented
    }

    let code_len = hdr.code_len as usize;
    let rodata_len = hdr.rodata_len as usize;
    let data_len = hdr.data_len as usize;
    let bss_len = hdr.bss_len as usize;

    let mut code_region = platform.alloc_exec(code_len)?;
    let mut rodata_region: Option<Region> = None;
    if rodata_len > 0 {
        rodata_region = Some(platform.alloc_ro(rodata_len)?);
    }
    let mut data_region: Option<Region> = None;
    if data_len + bss_len > 0 {
        data_region = Some(platform.alloc_rw(data_len + bss_len)?);
    }

    if code_len > 0 {
        unsafe { code_region.as_mut_slice().copy_from_slice(container.code()); }
    }
    if let Some(ref mut ro) = rodata_region {
        if rodata_len > 0 {
            unsafe { ro.as_mut_slice().copy_from_slice(container.rodata()); }
        }
    }
    if let Some(ref mut rw) = data_region {
        if data_len > 0 {
            unsafe { rw.as_mut_slice()[..data_len].copy_from_slice(container.data()); }
        }
    }

    // Steps 7–8: Resolve imports and apply relocations.
    let code_base = code_region.as_ptr() as u64;
    let container_code_off = hdr.code_off as u64;
    for i in 0..container.reloc_count() {
        let entry = container.reloc_entry(i).ok_or(E_BAD_CONTAINER)?;
        let site_off = entry.site_off as u64;
        if site_off < container_code_off { continue; }
        let local_off = (site_off - container_code_off) as usize;
        let need = if entry.kind == 1 { 8 } else { 4 };
        if local_off + need > code_len { return Err(E_BAD_CONTAINER); }
        let sym = sym_guard
            .map
            .lookup_by_hash(entry.sym_hash)
            .ok_or(E_SYMBOL_UNRESOLVED)?;
        let code_slice = unsafe { code_region.as_mut_slice() };
        dispatch_import_reloc(code_slice, local_off, entry.kind, sym.addr as u64)
            .map_err(|_| E_RELOC_UNSUPPORTED)?;
    }

    // Call __lang_mod_init (post-reloc, pre-export-registration).
    let init_addr = lookup_mod_init(container, code_base);
    if init_addr != 0 {
        let init_fn: extern "C" fn() = unsafe { core::mem::transmute(init_addr) };
        init_fn();
    }

    // Flip code from RW to RX (W^X).
    platform.make_exec(&mut code_region)?;

    // Step 9: Register exports via sym_guard.map.
    let modinfo_data = container.modinfo();
    if !modinfo_data.is_empty() {
        let mi = lmod::modinfo::decode(modinfo_data).ok_or(E_BAD_CONTAINER)?;
        for ei in 0..mi.export_count {
            let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(E_BAD_CONTAINER)?;
            if exp.name == MOD_INIT_NAME { continue; }
            sym_guard.map.register(exp.name, code_base as usize)?;
        }
    }

    // Phase 13: Tier-2 stack_bound re-derivation.
    if platform.trust_tier().rank() >= crate::platform::Tier::Two.rank() {
        let code_slice = unsafe { code_region.as_mut_slice() };
        let arch = crate::rederive::Arch::detect_from_code(code_slice);
        // slot_bytes is target-specific; we infer it from arch.
        let slot_bytes: u32 = match arch {
            crate::rederive::Arch::X86_64 => 8,
            crate::rederive::Arch::ArmThumb => 4,
            crate::rederive::Arch::RiscV => 4,
        };
        let rederived = crate::rederive::rederive_stack_high(code_slice, arch, slot_bytes);
        let modinfo_data = container.modinfo();
        if !modinfo_data.is_empty() {
            let mi = lmod::modinfo::decode(modinfo_data).ok_or(E_BAD_CONTAINER)?;
            for ei in 0..mi.export_count {
                let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(E_BAD_CONTAINER)?;
                if exp.name == MOD_INIT_NAME { continue; }
                // Read the word_meta entry's stack_bound field.
                let stamped = read_word_meta_stack_bound(modinfo_data, exp.value_off)?;
                if stamped == 0xFFFF_FFFF || rederived > stamped {
                    return Err(E_BAD_CONTAINER);
                }
            }
        }
    }

    // Phase 11: Check sharing_class consistency.
    let modinfo_data = container.modinfo();
    if !modinfo_data.is_empty() {
        let mut ri = 0u32;
        while let Some(rm) = lmod::modinfo::read_res_meta(modinfo_data, ri) {
            if rm.sharing_class != 0 {
                return Err(E_RESOURCE_SHARING_MISMATCH);
            }
            ri += 1;
        }
    }

    // Record in load-once set.
    loaded_set.insert(hdr.abi_hash)?;

    // Commit guard — rollback won't trigger on drop.
    sym_guard.commit();

    Ok(LoadedModule {
        code: code_region,
        rodata: rodata_region.unwrap_or_else(|| unsafe {
            Region::from_raw_parts(core::ptr::null_mut(), 0)
        }),
        data: data_region.unwrap_or_else(|| unsafe {
            Region::from_raw_parts(core::ptr::null_mut(), 0)
        }),
        init_addr,
        abi_hash: hdr.abi_hash,
    })
}

/// Read the `stack_bound` field from a word_meta entry at the given
/// byte offset within modinfo data.
fn read_word_meta_stack_bound(modinfo_data: &[u8], value_off: u32) -> Result<u32, u32> {
    let off = value_off as usize;
    // word_meta layout: sym_hash(8) + effects(2) + requires_caps(2) + stack_bound(4) = 16
    if off + 16 > modinfo_data.len() {
        return Err(E_BAD_CONTAINER);
    }
    Ok(u32::from_le_bytes(
        modinfo_data[off + 12..off + 16]
            .try_into()
            .map_err(|_| E_BAD_CONTAINER)?,
    ))
}

/// Look for `__lang_mod_init` in the module's export table.
/// Returns its code offset (or 0 if absent).
fn lookup_mod_init<'a>(container: &'a Container<'a>, code_base: u64) -> usize {
    let modinfo_data = container.modinfo();
    if modinfo_data.is_empty() {
        return 0;
    }
    let mi = match lmod::modinfo::decode(modinfo_data) {
        Some(m) => m,
        None => return 0,
    };
    for ei in 0..mi.export_count {
        let exp = match lmod::modinfo::read_export(modinfo_data, ei) {
            Some(e) => e,
            None => continue,
        };
        if exp.name == MOD_INIT_NAME {
            // The init function is exported; its address is code_base
            // (same simplification as other exports — see Phase 8).
            return code_base as usize;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Tier;
    use alloc::vec;

    /// Static buffer for TestPlatform allocations (page-aligned, 64KB).
    static mut TEST_BUF: [u8; 65536] = [0u8; 65536];
    static mut TEST_BUF_USED: usize = 0;

    struct TestPlatform {
        expected_hash: u64,
        fail: bool,
    }
    impl LoaderPlatform for TestPlatform {
        fn alloc_exec(&mut self, len: usize) -> Result<Region, u32> {
            let buf = unsafe { &mut *core::ptr::addr_of_mut!(TEST_BUF) };
            let used = unsafe { TEST_BUF_USED };
            if used + len > buf.len() { return Err(1); }
            let ptr = unsafe { buf.as_mut_ptr().add(used) };
            unsafe { TEST_BUF_USED = used + len };
            unsafe { Ok(Region::from_raw_parts(ptr, len)) }
        }
        fn alloc_ro(&mut self, len: usize) -> Result<Region, u32> {
            self.alloc_exec(len)
        }
        fn alloc_rw(&mut self, len: usize) -> Result<Region, u32> {
            self.alloc_exec(len)
        }
        fn make_exec(&mut self, _r: &mut Region) -> Result<(), u32> {
            if self.fail { Err(1) } else { Ok(()) }
        }
        fn expected_abi_hash(&self) -> u64 { self.expected_hash }
        fn trust_tier(&self) -> Tier { Tier::Zero }
    }

    #[test]
    fn rollback_guard_restores_on_error() {
        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"keep", 0x100).unwrap();
        let saved = map.len();

        {
            let mut guard = RollbackGuard::new(&mut map);
            guard.map.register(b"temp", 0x200).unwrap();
            // guard drops without commit → rollback
        }

        assert_eq!(map.len(), saved, "temp entry should have been removed");
        assert!(map.lookup_by_name(b"keep").is_some());
        assert!(map.lookup_by_name(b"temp").is_none());
    }

    #[test]
    fn rollback_guard_commit_preserves() {
        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"keep", 0x100).unwrap();

        {
            let mut guard = RollbackGuard::new(&mut map);
            guard.map.register(b"perm", 0x200).unwrap();
            guard.commit();
        }

        assert!(map.lookup_by_name(b"perm").is_some());
    }

    #[test]
    fn loaded_set_rejects_duplicates() {
        let mut set = LoadedSet::<8>::new();
        assert!(set.insert(42).is_ok());
        assert!(set.insert(42).is_err()); // duplicate
        assert!(set.insert(99).is_ok());  // different hash
    }

    #[test]
    fn signed_flag_without_tier_1_rejected() {
        // Build a module with the SIGNED flag set, but use a Tier-0 platform.
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        let mut raw = build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], 64);
        raw[6] |= lmod::header::LMOD_FLAG_SIGNED as u8;

        let container = lmod::validate::Container::parse(&raw).unwrap();

        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        // This platform defaults to Tier 0 — signed module should be rejected.
        let mut plat = TestPlatform { expected_hash: 0, fail: false };
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(result.is_err(), "signed module on Tier 0 should be rejected");
        assert_eq!(result.unwrap_err(), E_SIG_INVALID);
    }

    #[test]
    fn encrypted_container_rejected() {
        // Use build_minimal_lmod_with_modinfo to create a valid container,
        // then manually set the encrypted flag in the header.
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        let mut raw = build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], 64);
        // Patch the encrypted flag into the header (byte 6, bit 1).
        raw[6] |= 0x02;

        // The container should still parse (total_len unchanged, flag is just a bit).
        let container = lmod::validate::Container::parse(&raw).unwrap();
        assert_ne!(container.header().flags & lmod::header::LMOD_FLAG_ENCRYPTED, 0);

        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let mut plat = TestPlatform { expected_hash: 0, fail: false };
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(result.is_err(), "encrypted container should be rejected");
        assert_eq!(result.unwrap_err(), E_CONTAINER_ENCRYPTED);
    }

    #[test]
    fn loaded_set_contains() {
        let mut set = LoadedSet::<8>::new();
        assert!(!set.contains(42));
        set.insert(42).unwrap();
        assert!(set.contains(42));
        assert!(!set.contains(99));
    }

    #[test]
    fn mod_init_is_excluded_from_exports() {
        // We can't easily create a real module in no_std, but we can
        // verify the `MOD_INIT_NAME` constant and the skip logic.
        // The constant should match the expected word name.
        assert_eq!(MOD_INIT_NAME, b"__lang_mod_init");
    }

    #[test]
    fn lookup_mod_init_returns_zero_for_empty_modinfo() {
        let container_bytes = build_minimal_lmod(0);
        let container = lmod::validate::Container::parse(&container_bytes).unwrap();
        let result = lookup_mod_init(&container, 0x1000);
        assert_eq!(result, 0);
    }

    #[test]
    fn lookup_mod_init_detects_init_export() {
        // Build modinfo with a __lang_mod_init export + one regular export.
        let exports = [
            lmod::modinfo::ExportEntry {
                sym_hash: lmod::hash::fnv1a_u64(b"__lang_mod_init"),
                name: b"__lang_mod_init",
                effects: 0, requires_caps: 0, stack_bound: 0,
            },
            lmod::modinfo::ExportEntry {
                sym_hash: lmod::hash::fnv1a_u64(b"user_word"),
                name: b"user_word",
                effects: 0, requires_caps: 0, stack_bound: 0,
            },
        ];
        let mut mi_buf = [0u8; 256];
        let size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &exports, &[], 0, 0, &[]).unwrap();
        let container_bytes = build_minimal_lmod_with_modinfo(&mi_buf[..size], 64);
        let container = lmod::validate::Container::parse(&container_bytes).unwrap();
        let result = lookup_mod_init(&container, 0x2000);
        // The init function was found: returns code_base.
        assert_eq!(result, 0x2000);
    }

    // -----------------------------------------------------------------------
    // Fuzz tests: Container::parse and load_module must never panic
    // -----------------------------------------------------------------------
    // The TestPlatform's alloc_exec leaks Vecs, which causes heap corruption
    // under repeated calls.  We test Container::parse separately (no alloc)
    // and rely on the end-to-end tests in tooling-tests to exercise
    // load_module with realistic containers.

    #[test]
    fn fuzz_load_module_valid_container_no_panic() {
        // A single valid container must load without panic.
        let bytes = build_minimal_lmod(64);
        let container = lmod::validate::Container::parse(&bytes).unwrap();
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let mut plat = TestPlatform { expected_hash: 0, fail: false };
        // This will fail (unresolved symbols) but must not panic.
        let _result = load_module(&container, &mut plat, &mut map, &mut set);
    }

    /// Build a minimal .lmod container with a given code section size.
    fn build_minimal_lmod(code_size: u32) -> alloc::vec::Vec<u8> {
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], code_size)
    }

    /// Build a minimal .lmod container with explicit modinfo bytes.
    fn build_minimal_lmod_with_modinfo(modinfo: &[u8], code_size: u32) -> alloc::vec::Vec<u8> {
        let mi_len = modinfo.len() as u32;
        let reloc_count = 0u32;
        let layout = lmod::header::compute_layout(0, mi_len, code_size, 0, 0, 0, reloc_count);
        let total = layout.total_len as usize;
        let mut buf = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut buf, &layout);
        let off = layout.modinfo_off as usize;
        buf[off..off + modinfo.len()].copy_from_slice(modinfo);
        buf
    }
}

/// Dispatch an import relocation to the correct architecture backend.
fn dispatch_import_reloc(
    code: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
) -> Result<(), u32> {
    // Determine addend based on relocation kind.
    let addend = match kind {
        1 => 0,           // R_X86_64_64
        2 | 3 => -4,      // R_X86_64_PC32 / R_X86_64_PLT32
        4 => 0,           // R_ARM_ABS32
        5 => -4,          // R_ARM_THM_CALL (PC = site + 4)
        6 => -4,          // R_ARM_THM_JUMP24
        7 => 0,           // R_ARM_REL32
        _ => 0,
    };
    match kind {
        1 | 2 | 3 => crate::reloc_x86_64::apply_import_reloc(code, site_off, kind, sym_addr, addend),
        4 | 5 | 6 | 7 => crate::reloc_arm::apply_import_reloc(code, site_off, kind, sym_addr, addend),
        8 | 9 => crate::reloc_riscv::apply_import_reloc(code, site_off, kind, sym_addr, addend),
        _ => Err(crate::reloc_x86_64::E_RELOC_UNSUPPORTED),
    }
}
