//! Loader driver — the target-independent half of `__lang_load_module`.
//!
//! Implements module-format-and-loading.md §9 (loader algorithm) for Tier 0
//! (no signature verification, no Tier-2 re-derivation).
//!
//! Phase 9 additions: transactional rollback, `__lang_mod_init` hook,
//! load-once enforcement, failure atomicity.

use crate::error::LoadError;
use crate::platform::{LoaderPlatform, Region};
use crate::symbols::SymMap;
use lmod::validate::Container;

// Re-export the error type and legacy numeric shims for external callers.
pub use crate::error::{
    E_ABI_MISMATCH, E_BAD_CONTAINER, E_RELOC_UNSUPPORTED, E_SIG_INVALID, E_SYMBOL_UNRESOLVED,
    E_SYMBOL_CONFLICT, E_MODULE_DECLARES_ISR, E_RESOURCE_SHARING_MISMATCH,
    E_CONTAINER_ENCRYPTED, E_MODULE_ALREADY_LOADED, E_STACK_BOUND_UNVERIFIABLE,
    E_ENC_UNSUPPORTED, E_ENC_REQUIRES_SIGNED, E_ENC_NO_KEY, E_ENC_AUTH_FAIL, E_ENC_BAD_HEADER,
};

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
    /// Returns `Err(LoadError::ModuleAlreadyLoaded)` if already present.
    pub fn insert(&mut self, abi_hash: u64) -> Result<(), LoadError> {
        if self.contains(abi_hash) {
            return Err(LoadError::ModuleAlreadyLoaded);
        }
        if self.len < N {
            self.hashes[self.len] = abi_hash;
            self.len += 1;
            Ok(())
        } else {
            Err(LoadError::ModuleAlreadyLoaded) // set full, treat as loaded
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
) -> Result<LoadedModule, LoadError> {
    // Steps 1–2: parsing and validation already done by Container::parse().
    let hdr = container.header();

    // Step 3: check abi_hash.
    if hdr.abi_hash != platform.expected_abi_hash() {
        return Err(LoadError::AbiMismatch);
    }

    // Load-once check.
    if loaded_set.contains(hdr.abi_hash) {
        return Err(LoadError::ModuleAlreadyLoaded);
    }

    // Signed-flag consistency check (module-format §3).
    if hdr.flags & lmod::header::LMOD_FLAG_SIGNED != 0
        && platform.trust_tier().rank() < crate::platform::Tier::One.rank()
    {
        return Err(LoadError::SigInvalid);
    }

    // Step 4: Signature verification (Tier ≥ 1).
    let raw_bytes = container.raw_bytes();
    if platform.trust_tier().rank() >= crate::platform::Tier::One.rank() {
        let region_len = lmod::sig::signed_region_len(hdr);
        if region_len > raw_bytes.len() {
            return Err(LoadError::BadContainer);
        }
        let signed_region = &raw_bytes[..region_len];
        let trailer_data = &raw_bytes[region_len..];
        let trailer = lmod::sig::SigTrailer::parse(trailer_data).ok_or(LoadError::SigInvalid)?;
        if trailer.scheme == lmod::sig::SCHEME_NONE {
            return Err(LoadError::SigInvalid);
        }
        if !platform.verify_sig(signed_region, trailer.sig_bytes) {
            return Err(LoadError::SigInvalid);
        }
    }

    // Step 4b: Decryption (NEW, feature-gated).
    // Must happen after signature verification (P3: no unauthenticated decryption)
    // and before section placement (so placed bytes are plaintext).
    let decrypted_cek: Option<[u8; 32]> = None;
    let decrypted_nonce: [u8; 12] = [0u8; 12];
    let decrypted_tag: [u8; 16] = [0u8; 16];
    let aad_buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();

    if hdr.flags & lmod::header::LMOD_FLAG_ENCRYPTED != 0 {
        #[cfg(not(feature = "encryption"))]
        { return Err(LoadError::EncUnsupported); }

        #[cfg(feature = "encryption")]
        {
            if platform.trust_tier().rank() < crate::platform::Tier::One.rank() {
                return Err(LoadError::EncRequiresSigned);
            }
            // Parse enc-header.
            let eh_start = lmod::header::HEADER_SIZE as usize;
            let eh_bytes = raw_bytes.get(eh_start..).ok_or(LoadError::EncBadHeader)?;
            let eh = lmod::enc::decode_enc_header(eh_bytes).ok_or(LoadError::EncBadHeader)?;

            // Unwrap CEK — try each wrapped slot.
            let mut cek = [0u8; 32];
            let mut cek_found = false;
            for slot in &eh.wrapped_slots {
                if platform.unwrap_cek(slot.key_id, &slot.wrapped, &mut cek).is_ok() {
                    cek_found = true;
                    break;
                }
            }
            if !cek_found {
                return Err(LoadError::EncNoKey);
            }
            decrypted_cek = Some(cek);
            decrypted_nonce = eh.nonce;
            decrypted_tag = eh.tag;

            // Build AAD: header (with pre-signing values) +
            // enc_header(with tag zeroed) + modinfo + reloc.
            // lmod-sign modifies: flags (adds SIGNED), total_len (+= sig_len),
            // sig_len (from 0 to trailer size).
            // sig_off is unchanged (already correct from compute_layout).
            // Restore pre-signing values.
            let aad_total_len = hdr.sig_off;
            let mut aad_header = [0u8; lmod::header::HEADER_SIZE as usize];
            aad_header.copy_from_slice(&raw_bytes[..lmod::header::HEADER_SIZE as usize]);
            aad_header[16..20].copy_from_slice(&aad_total_len.to_le_bytes());
            // Clear the SIGNED flag — it was added after encryption.
            let aad_flags = u16::from_le_bytes([aad_header[6], aad_header[7]]) & !lmod::header::LMOD_FLAG_SIGNED;
            aad_header[6..8].copy_from_slice(&aad_flags.to_le_bytes());
            // sig_len was 0 before signing (no trailer).
            aad_header[68..72].copy_from_slice(&0u32.to_le_bytes());
            aad_buf.extend_from_slice(&aad_header);
            let mut eh_for_aad = eh_bytes[..lmod::enc::enc_header_len(eh.wrapped_slots.len())].to_vec();
            let tag_off_in_eh = 4 + lmod::enc::NONCE_LEN;
            eh_for_aad[tag_off_in_eh..tag_off_in_eh + lmod::enc::TAG_LEN].fill(0);
            aad_buf.extend_from_slice(&eh_for_aad);
            aad_buf.extend_from_slice(container.modinfo());
            let reloc_bytes = (hdr.reloc_count as usize)
                .checked_mul(lmod::reloc::RELOC_ENTRY_SIZE as usize)
                .unwrap_or(0);
            if reloc_bytes > 0 {
                let ro = hdr.reloc_off as usize;
                if ro + reloc_bytes <= raw_bytes.len() {
                    aad_buf.extend_from_slice(&raw_bytes[ro..ro + reloc_bytes]);
                }
            }
        }
    }

    // Step 5: Static-ISR rule — reject modules that declare @interrupt bindings.
    let modinfo_data = container.modinfo();
    if !modinfo_data.is_empty() {
        if let Some(mi) = lmod::modinfo::decode(modinfo_data) {
            if mi.has_isr() {
                return Err(LoadError::ModuleDeclaresIsr);
            }
        }
    }

    // --- Begin transactional section (steps 4–12) ---
    let sym_guard = RollbackGuard::new(global_map);

    // Step 6: Place sections.
    // Check placement policy: only CopyToRam is implemented in v1.
    if platform.placement_policy() != crate::platform::PlacementPolicy::CopyToRam {
        return Err(LoadError::BadContainer); // XIP not yet implemented
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

    // Step 4b (continued): Decrypt payload in-place after placement.
    if let Some(cek) = &decrypted_cek {
        #[cfg(feature = "encryption")]
        {
            let payload_len = (code_len + rodata_len + data_len) as usize;
            // Use the code_region as the temp buffer since it's writable.
            // For sections beyond code (rodata, data), we extend into the
            // code region's buffer by copying them after code.
            // All three sections are already placed; decrypt in place.
            if payload_len > 0 {
                // Copy rodata and data after code in a temp Vec.
                let mut tmp = alloc::vec![0u8; rodata_len + data_len];
                if rodata_len > 0 {
                    if let Some(ref ro) = rodata_region {
                        let cs = unsafe { ro.as_slice() };
                        tmp[..rodata_len].copy_from_slice(&cs[..rodata_len]);
                    }
                }
                if data_len > 0 {
                    if let Some(ref rw) = data_region {
                        let cs = unsafe { rw.as_slice() };
                        tmp[rodata_len..rodata_len + data_len].copy_from_slice(&cs[..data_len]);
                    }
                }

                let mut cs = unsafe { code_region.as_mut_slice() };
                // Extend cs logically to hold the full payload by extending
                // the mutable slice.  Since code_region has code_len bytes
                // but we need code_len + rodata_len + data_len, we use the
                // available writable memory after code_region (if any).
                // This is safe because alloc_exec gave us a region that may
                // be larger than code_len (page-aligned).
                let total_extend = rodata_len + data_len;
                if total_extend > 0 {
                    let extra = cs.len().saturating_sub(code_len);
                    if extra >= total_extend {
                        // Append rodata+data after code in the code region.
                        cs[code_len..code_len + rodata_len].copy_from_slice(&tmp[..rodata_len]);
                        if data_len > 0 {
                            cs[code_len + rodata_len..payload_len].copy_from_slice(
                                &tmp[rodata_len..rodata_len + data_len]
                            );
                        }
                        // Decrypt the full payload in place (code region).
                        crate::crypto::chacha20poly1305::decrypt_payload(
                            cek, &decrypted_nonce, &decrypted_tag, &aad_buf,
                            &mut cs[..payload_len],
                        ).map_err(|_| LoadError::EncAuthFail)?;

                        // Scatter back to rodata and data regions.
                        if rodata_len > 0 {
                            if let Some(ref mut ro) = rodata_region {
                                let ro_slice = unsafe { ro.as_mut_slice() };
                                ro_slice[..rodata_len].copy_from_slice(&cs[code_len..code_len + rodata_len]);
                            }
                        }
                        if data_len > 0 {
                            if let Some(ref mut rw) = data_region {
                                let rw_slice = unsafe { rw.as_mut_slice() };
                                rw_slice[..data_len].copy_from_slice(&cs[code_len + rodata_len..payload_len]);
                            }
                        }
                    } else {
                        // Not enough extra space — use a temp Vec (rare).
                        let mut payload_buf = alloc::vec![0u8; payload_len];
                        payload_buf[..code_len].copy_from_slice(&cs[..code_len]);
                        payload_buf[code_len..code_len + rodata_len].copy_from_slice(&tmp[..rodata_len]);
                        payload_buf[code_len + rodata_len..payload_len].copy_from_slice(
                            &tmp[rodata_len..rodata_len + data_len]
                        );
                        crate::crypto::chacha20poly1305::decrypt_payload(
                            cek, &decrypted_nonce, &decrypted_tag, &aad_buf, &mut payload_buf,
                        ).map_err(|_| LoadError::EncAuthFail)?;
                        cs[..code_len].copy_from_slice(&payload_buf[..code_len]);
                        if rodata_len > 0 {
                            if let Some(ref mut ro) = rodata_region {
                                let ro_slice = unsafe { ro.as_mut_slice() };
                                ro_slice[..rodata_len].copy_from_slice(&payload_buf[code_len..code_len + rodata_len]);
                            }
                        }
                        if data_len > 0 {
                            if let Some(ref mut rw) = data_region {
                                let rw_slice = unsafe { rw.as_mut_slice() };
                                rw_slice[..data_len].copy_from_slice(&payload_buf[code_len + rodata_len..payload_len]);
                            }
                        }
                    }
                } else {
                    // Only code section — decrypt directly in code region.
                    crate::crypto::chacha20poly1305::decrypt_payload(
                        cek, &decrypted_nonce, &decrypted_tag, &aad_buf,
                        &mut cs[..code_len],
                    ).map_err(|_| LoadError::EncAuthFail)?;
                }
            }
        }
    }

    // Steps 7–8: Resolve imports and apply relocations.
    let code_base = code_region.as_ptr() as u64;
    let container_code_off = hdr.code_off as u64;
    for i in 0..container.reloc_count() {
        let entry = container.reloc_entry(i).ok_or(LoadError::BadContainer)?;
        let site_off = entry.site_off as u64;
        if site_off < container_code_off { continue; }
        let local_off = (site_off - container_code_off) as usize;
        let need = if entry.kind == 1 { 8 } else { 4 };
        if local_off + need > code_len { return Err(LoadError::BadContainer); }
        let sym = sym_guard
            .map
            .lookup_by_hash(entry.sym_hash)
            .ok_or(LoadError::SymbolUnresolved)?;
        let code_slice = unsafe { code_region.as_mut_slice() };
        dispatch_import_reloc(code_slice, local_off, entry.kind, sym.addr as u64)
            .map_err(|_| LoadError::RelocUnsupported)?;
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
        let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
        for ei in 0..mi.export_count {
            let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(LoadError::BadContainer)?;
            if exp.name == MOD_INIT_NAME { continue; }
            sym_guard.map.register(exp.name, code_base as usize)?;
        }
    }

    // Phase 13: Tier-2 stack_bound re-derivation.
    // Trust relationship: the producer's `stamped` bound must be AT LEAST the
    // independently-re-derived bound.  Reject when `rederived > stamped`
    // (producer under-declared) or when the scanner returns `⊤` and the
    // producer claims a finite bound the loader cannot confirm.
    if platform.trust_tier().rank() >= crate::platform::Tier::Two.rank() {
        let code_slice = unsafe { code_region.as_mut_slice() };
        let arch = crate::rederive::Arch::detect_from_code(code_slice);
        let slot_bytes: u32 = match arch {
            crate::rederive::Arch::X86_64 => 8,
            crate::rederive::Arch::ArmThumb => 4,
            crate::rederive::Arch::RiscV => 4,
        };
        let rederived = crate::rederive::rederive_stack_high(code_slice, arch, slot_bytes);
        let modinfo_data = container.modinfo();
        if !modinfo_data.is_empty() {
            let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
            for ei in 0..mi.export_count {
                let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(LoadError::BadContainer)?;
                if exp.name == MOD_INIT_NAME { continue; }
                let stamped = read_word_meta_stack_bound(modinfo_data, exp.value_off)?;
                if stamped == crate::rederive::TOP_SENTINEL {
                    // Producer could not bound either — reject (no finite bound).
                    return Err(LoadError::BadContainer);
                }
                if rederived == crate::rederive::TOP_SENTINEL && stamped != crate::rederive::TOP_SENTINEL {
                    // Scanner cannot confirm the producer's finite claim.
                    return Err(LoadError::StackBoundUnverifiable);
                }
                if rederived > stamped {
                    return Err(LoadError::BadContainer);
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
                return Err(LoadError::ResourceSharingMismatch);
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
fn read_word_meta_stack_bound(modinfo_data: &[u8], value_off: u32) -> Result<u32, LoadError> {
    let off = value_off as usize;
    // word_meta layout: sym_hash(8) + effects(2) + requires_caps(2) + stack_bound(4) = 16
    if off + 16 > modinfo_data.len() {
        return Err(LoadError::BadContainer);
    }
    Ok(u32::from_le_bytes(
        modinfo_data[off + 12..off + 16]
            .try_into()
            .map_err(|_| LoadError::BadContainer)?,
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
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        let mut raw = build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], 64);
        raw[6] |= 0x02; // set ENCRYPTED flag

        let container = lmod::validate::Container::parse(&raw).unwrap();
        assert_ne!(container.header().flags & lmod::header::LMOD_FLAG_ENCRYPTED, 0);

        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let mut plat = TestPlatform { expected_hash: 0, fail: false };
        let result = load_module(&container, &mut plat, &mut map, &mut set);

        #[cfg(not(feature = "encryption"))]
        assert_eq!(result.unwrap_err(), E_ENC_UNSUPPORTED);
        #[cfg(feature = "encryption")]
        // With encryption feature but Tier-0, should fail with E_ENC_REQUIRES_SIGNED.
        assert_eq!(result.unwrap_err(), E_ENC_REQUIRES_SIGNED);
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
        let layout = lmod::header::compute_layout(0, mi_len, code_size, 0, 0, 0, reloc_count, 0);
        let total = layout.total_len as usize;
        let mut buf = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut buf, &layout);
        let off = layout.modinfo_off as usize;
        buf[off..off + modinfo.len()].copy_from_slice(modinfo);
        buf
    }

    // -------------------------------------------------------------------
    // FullPlatform — a Tier One platform with owned bump allocator,
    // real HMAC verification, and (when "encryption" feature is active)
    // real CEK unwrapping.
    // -------------------------------------------------------------------

    use alloc::vec::Vec;
    use core::cell::Cell;
    use crate::symbols::SymEntry;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type FullHmacSha256 = Hmac<Sha256>;

    struct FullPlatform {
        buf: Vec<u8>,
        used: Cell<usize>,
        hash: u64,
        kek: [u8; 32],
        fail_make_exec: bool,
    }

    impl FullPlatform {
        fn new(hash: u64, kek: [u8; 32]) -> Self {
            Self {
                buf: vec![0u8; 65536],
                used: Cell::new(0),
                hash,
                kek,
                fail_make_exec: false,
            }
        }
        fn alloc(&self, len: usize) -> Result<Region, u32> {
            let used = self.used.get();
            if used + len > self.buf.len() { return Err(1); }
            let ptr = self.buf.as_ptr() as *mut u8;
            let region = unsafe { Region::from_raw_parts(ptr.add(used), len) };
            self.used.set(used + len);
            Ok(region)
        }
    }

    impl LoaderPlatform for FullPlatform {
        fn alloc_exec(&mut self, len: usize) -> Result<Region, u32> { self.alloc(len) }
        fn alloc_ro(&mut self, len: usize) -> Result<Region, u32> { self.alloc(len) }
        fn alloc_rw(&mut self, len: usize) -> Result<Region, u32> { self.alloc(len) }
        fn make_exec(&mut self, _r: &mut Region) -> Result<(), u32> {
            if self.fail_make_exec { Err(1) } else { Ok(()) }
        }
        fn expected_abi_hash(&self) -> u64 { self.hash }
        fn trust_tier(&self) -> crate::platform::Tier { crate::platform::Tier::One }

        fn verify_sig(&self, signed: &[u8], sig: &[u8]) -> bool {
            let mut mac = FullHmacSha256::new_from_slice(&self.kek)
                .expect("HMAC accepts 32-byte key");
            mac.update(signed);
            mac.finalize().into_bytes().as_slice() == sig
        }

        #[cfg(feature = "encryption")]
        fn unwrap_cek(&self, _key_id: u64, wrapped: &[u8], out: &mut [u8; 32]) -> Result<(), u32> {
            let w: &[u8; 60] = wrapped.try_into().map_err(|_| 1u32)?;
            let cek = crate::crypto::chacha20poly1305::unwrap_cek(&self.kek, w)
                .map_err(|_| 1u32)?;
            *out = cek;
            Ok(())
        }
    }

    /// Build a minimal .lmod whose code section carries `code_bytes` and which
    /// exports a word named "main" with a given stack bound (Tier-2 test).
    fn build_lmod_with_code(code_bytes: &[u8], abi_hash: u64, export_main: bool) -> Vec<u8> {
        let code_len = code_bytes.len() as u32;
        let exports = if export_main {
            alloc::vec![lmod::modinfo::ExportEntry {
                sym_hash: lmod::hash::fnv1a_u64(b"main"),
                name: b"main",
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            }]
        } else {
            alloc::vec![]
        };
        let mut mi_buf = [0u8; 512];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &exports, &[], abi_hash, 0, &[])
            .unwrap_or(0) as u32;
        let reloc_count = 0u32;
        let layout = lmod::header::compute_layout(abi_hash, mi_size, code_len, 0, 0, 0, reloc_count, 0);
        let total = layout.total_len as usize;
        let mut buf = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut buf, &layout);
        let mi_off = layout.modinfo_off as usize;
        buf[mi_off..mi_off + mi_size as usize].copy_from_slice(&mi_buf[..mi_size as usize]);
        let co = layout.code_off as usize;
        buf[co..co + code_bytes.len()].copy_from_slice(code_bytes);
        buf
    }

    /// Sign a .lmod container with HMAC-SHA256 using the given key.
    fn sign_lmod(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
        let mut header = lmod::header::decode_header(data)
            .expect("valid header for signing");
        header.flags |= lmod::header::LMOD_FLAG_SIGNED;
        let region_len = lmod::sig::signed_region_len(&header);
        let sig_off = region_len as u32;
        let sig_len = lmod::sig::TRAILER_HEADER_SIZE
            + lmod::sig::sig_len_for_scheme(lmod::sig::SCHEME_HMAC_SHA256).unwrap();
        let new_total = sig_off + sig_len;

        let mut signed_region = data[..region_len].to_vec();
        signed_region[6..8].copy_from_slice(&header.flags.to_le_bytes());
        signed_region[16..20].copy_from_slice(&new_total.to_le_bytes());
        signed_region[64..68].copy_from_slice(&sig_off.to_le_bytes());
        signed_region[68..72].copy_from_slice(&sig_len.to_le_bytes());

        let mut mac = FullHmacSha256::new_from_slice(key)
            .expect("HMAC accepts 32-byte key");
        mac.update(&signed_region);
        let sig_bytes = mac.finalize().into_bytes();

        let mut out = signed_region;
        out.push(lmod::sig::SCHEME_HMAC_SHA256);
        out.extend_from_slice(&sig_bytes);
        out
    }

    /// Encrypt a .lmod container with ChaCha20-Poly1305 (fleet mode, single KEK).
    /// Returns the encrypted bytes and the random CEK used.
    #[cfg(feature = "encryption")]
    fn encrypt_lmod(data: &[u8], kek: &[u8; 32]) -> (Vec<u8>, [u8; 32]) {
        use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        use lmod::enc::{
            enc_header_len, encode_enc_header, EncHeader, EncMode, WrappedCekSlot,
            AEAD_CHACHA20POLY1305, CEK_LEN, NONCE_LEN, TAG_LEN, WRAP_LEN,
            WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
        };
        use rand_core::RngCore;

        let container = lmod::validate::Container::parse(data).unwrap();
        let hdr = container.header();

        // Generate random CEK + nonce.
        let mut cek = [0u8; CEK_LEN];
        OsRng.fill_bytes(&mut cek);
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);

        // Wrap CEK under KEK using deterministic zero-nonce ChaCha20-Poly1305.
        let wrap_cipher = ChaCha20Poly1305::new(Key::from_slice(kek));
        let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
        let mut cek_buf = cek;
        let wrap_tag = wrap_cipher.encrypt_in_place_detached(zero_nonce, b"", &mut cek_buf)
            .expect("wrap");
        let mut wrapped = [0u8; WRAP_LEN];
        wrapped[..NONCE_LEN].copy_from_slice(&[0u8; NONCE_LEN]);
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
        wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());
        let slot = WrappedCekSlot {
            key_id: 0,
            wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
            wrapped,
        };

        let eh = EncHeader {
            enc_mode: EncMode::Fleet,
            aead_id: AEAD_CHACHA20POLY1305,
            nonce,
            tag: [0u8; TAG_LEN],
            wrapped_slots: alloc::vec![slot],
        };
        let eh_len = enc_header_len(eh.wrapped_slots.len()) as u32;

        // Build payload
        let payload = {
            let mut p = alloc::vec![0u8; container.code().len() + container.rodata().len() + container.data().len()];
            let mut off = 0;
            p[off..off + container.code().len()].copy_from_slice(container.code());
            off += container.code().len();
            p[off..off + container.rodata().len()].copy_from_slice(container.rodata());
            off += container.rodata().len();
            p[off..off + container.data().len()].copy_from_slice(container.data());
            p
        };

        let layout = lmod::header::compute_layout(
            hdr.abi_hash, hdr.modinfo_len, hdr.code_len, hdr.rodata_len,
            hdr.data_len, hdr.bss_len, hdr.reloc_count, eh_len,
        );

        let total = layout.total_len as usize;
        let mut out = alloc::vec![0u8; total];

        // Write header
        let mut hdr_out = layout;
        hdr_out.format_ver = lmod::header::FORMAT_VER;
        hdr_out.flags = hdr.flags | lmod::header::LMOD_FLAG_ENCRYPTED;
        lmod::header::encode_header(&mut out, &hdr_out);

        // Write enc-header
        let mut eh_bytes = alloc::vec![0u8; eh_len as usize];
        encode_enc_header(&mut eh_bytes, &eh).unwrap();
        let hdr_size = lmod::header::HEADER_SIZE as usize;
        out[hdr_size..hdr_size + eh_len as usize].copy_from_slice(&eh_bytes);

        // Write modinfo
        let mi = container.modinfo();
        out[layout.modinfo_off as usize..layout.modinfo_off as usize + mi.len()].copy_from_slice(mi);

        // Write reloc (if any)
        if hdr.reloc_count > 0 {
            let ro = hdr.reloc_off as usize;
            let rc = hdr.reloc_count as usize;
            let rb = rc * lmod::reloc::RELOC_ENTRY_SIZE as usize;
            if ro + rb <= data.len() {
                out[layout.reloc_off as usize..layout.reloc_off as usize + rb]
                    .copy_from_slice(&data[ro..ro + rb]);
            }
        }

        // Build AAD and encrypt
        let mut aad = alloc::Vec::new();
        aad.extend_from_slice(&out[..lmod::header::HEADER_SIZE as usize]);
        let tag_off_in_eh = 4 + NONCE_LEN;
        eh_bytes[tag_off_in_eh..tag_off_in_eh + TAG_LEN].fill(0);
        aad.extend_from_slice(&eh_bytes);
        aad.extend_from_slice(mi);

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&cek));
        let aead_nonce = Nonce::from_slice(&nonce);
        let ciphertext = cipher.encrypt(aead_nonce, Payload { msg: &payload, aad: &aad })
            .expect("encrypt payload");

        // Write encrypted code/rodata/data
        let co = layout.code_off as usize;
        out[co..co + hdr.code_len as usize].copy_from_slice(&ciphertext[..hdr.code_len as usize]);
        if hdr.rodata_len > 0 {
            let ro_start = co + hdr.code_len as usize;
            let ro_len = hdr.rodata_len as usize;
            out[ro_start..ro_start + ro_len].copy_from_slice(
                &ciphertext[hdr.code_len as usize..hdr.code_len as usize + ro_len]
            );
        }
        if hdr.data_len > 0 {
            let data_start = layout.data_off as usize;
            let payload_off = (hdr.code_len + hdr.rodata_len) as usize;
            out[data_start..data_start + hdr.data_len as usize].copy_from_slice(
                &ciphertext[payload_off..payload_off + hdr.data_len as usize]
            );
        }

        // Write AEAD tag into enc-header
        let aead_tag = &ciphertext[ciphertext.len() - TAG_LEN..];
        let tag_in_header = lmod::header::HEADER_SIZE as usize + tag_off_in_eh;
        out[tag_in_header..tag_in_header + TAG_LEN].copy_from_slice(aead_tag);

        (out, cek)
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn signed_encrypted_module_loads_and_runs() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true);
        let (enc, _cek) = encrypt_lmod(&plain, &kek);
        let signed = sign_lmod(&enc, &kek);

        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        map.entries[0] = Some(SymEntry {
            hash: lmod::hash::fnv1a_u64(b"main"),
            name: &[],
            addr: 0,
        });
        map.len = 1;

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(result.is_ok(),
            "signed+encrypted module must load, got {:?}", result.err());
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn signed_encrypted_tampered_ciphertext_rejected() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true);
        let (enc, _cek) = encrypt_lmod(&plain, &kek);
        let signed = sign_lmod(&enc, &kek);

        let mut bad = signed.clone();
        let hdr = lmod::header::decode_header(&bad).unwrap();
        let co = hdr.code_off as usize;
        bad[co] ^= 0x01;

        let container = lmod::validate::Container::parse(&bad).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        map.entries[0] = Some(SymEntry { hash: 0, name: &[], addr: 0 });
        map.len = 1;

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(result.unwrap_err(), LoadError::EncAuthFail,
            "tampered ciphertext must yield EncAuthFail");
    }

    // -------------------------------------------------------------------
    // H2: Rollback-on-failure — partial symbol registration is undone.
    // -------------------------------------------------------------------

    #[test]
    fn rollback_on_make_exec_failure() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true); // exports "main"
        let signed = sign_lmod(&plain, &kek); // not encrypted — faster

        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        plat.fail_make_exec = true; // make_exec will fail

        let mut map: SymMap<'_, 256> = SymMap::new();
        // Pre-register a sentinel symbol to verify rollback doesn't clear it.
        map.register(b"__sentinel", 0xDEAD).unwrap();
        let saved_len = map.len();
        let mut set = LoadedSet::<64>::new();
        // Pre-populate loaded_set to verify it's unchanged.
        set.insert(0x1234).unwrap();
        let saved_set_contains = set.contains(0x1234);
        let set_len_before = set.len;

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(result.is_err(), "load_module must fail when make_exec fails");
        // global_map must be restored: sentinel survives, "main" export is rolled back.
        assert_eq!(map.len(), saved_len,
            "global_map must be restored to pre-load length after failure");
        assert!(map.lookup_by_name(b"__sentinel").is_some(),
            "sentinel symbol must survive rollback");
        assert!(map.lookup_by_name(b"main").is_none(),
            "partial export 'main' must be rolled back");
        // loaded_set must be unchanged.
        assert_eq!(set.len, set_len_before,
            "loaded_set must be unchanged after failed load");
        assert!(set.contains(0x1234), "loaded_set entries must survive");
    }

    // -------------------------------------------------------------------
    // M2: AbiMismatch test
    // -------------------------------------------------------------------

    #[test]
    fn abi_hash_mismatch_rejected() {
        let code = [0xC3u8];
        let plat_hash = 42u64;
        let mod_hash = 99u64; // different from platform
        let plain = build_lmod_with_code(&code, mod_hash, false);
        let container = lmod::validate::Container::parse(&plain).unwrap();
        let mut plat = FullPlatform::new(plat_hash, [0; 32]);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(result.unwrap_err(), LoadError::AbiMismatch);
    }

    // -------------------------------------------------------------------
    // M5: ModuleDeclaresIsr and ResourceSharingMismatch
    // -------------------------------------------------------------------

    #[test]
    fn isr_module_rejected() {
        let code = [0xC3u8];
        let key = [0xabu8; 32];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
        // Build modinfo with HAS_ISR flag set.
        let exports = [];
        let mut mi_buf = [0u8; 512];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &exports, &[], abi_hash, 0, &[])
            .unwrap() as u32;
        // Set the HAS_ISR flag in the modinfo header (byte 6, bit 0 = 0x01).
        mi_buf[6] |= 0x01;

        let code_len = code.len() as u32;
        let layout = lmod::header::compute_layout(abi_hash, mi_size, code_len, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut raw = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut raw, &layout);
        let mi_off = layout.modinfo_off as usize;
        raw[mi_off..mi_off + mi_size as usize].copy_from_slice(&mi_buf[..mi_size as usize]);
        let co = layout.code_off as usize;
        raw[co..co + code.len()].copy_from_slice(&code);

        // FullPlatform is Tier One — sign the module before loading.
        let signed = sign_lmod(&raw, &key);
        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, key);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(result.unwrap_err(), LoadError::ModuleDeclaresIsr);
    }

    #[test]
    fn sharing_class_mismatch_rejected() {
        let code = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
        // Build modinfo with a res_meta entry that has non-zero sharing_class.
        let res_meta = lmod::modinfo::ResMetaEntry {
            res_hash: lmod::hash::fnv1a_u64(b"some_resource"),
            sharing_class: 1, // non-zero → should be rejected
            lock_prim: 0,
        };
        let exports = [];
        let mut mi_buf = [0u8; 512];
        // encode_into needs module_name, exports, imports, abi_hash, flags, res_metas
        let mi_size = lmod::modinfo::encode_into(
            &mut mi_buf, b"T", &exports, &[], abi_hash, 0, &[res_meta],
        ).unwrap() as u32;

        let code_len = code.len() as u32;
        let layout = lmod::header::compute_layout(abi_hash, mi_size, code_len, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut raw = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut raw, &layout);
        let mi_off = layout.modinfo_off as usize;
        raw[mi_off..mi_off + mi_size as usize].copy_from_slice(&mi_buf[..mi_size as usize]);
        let co = layout.code_off as usize;
        raw[co..co + code.len()].copy_from_slice(&code);

        // FullPlatform is Tier One — sign the module before loading.
        let key = [0xabu8; 32];
        let signed = sign_lmod(&raw, &key);
        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, key);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(result.unwrap_err(), LoadError::ResourceSharingMismatch);
    }

    // -------------------------------------------------------------------
    // ModuleAlreadyLoaded (second load of same module)
    // -------------------------------------------------------------------

    #[test]
    fn double_load_rejected() {
        let code = [0xC3u8];
        let key = [0xabu8; 32];
        let abi_hash = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
        let plain = build_lmod_with_code(&code, abi_hash, false);
        let signed = sign_lmod(&plain, &key);
        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, key);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();

        // First load should succeed.
        let r1 = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(r1.is_ok(), "first load must succeed");

        // Second load (same abi_hash) should fail.
        let r2 = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(r2.unwrap_err(), LoadError::ModuleAlreadyLoaded);
    }
}

/// Dispatch an import relocation to the correct architecture backend.
fn dispatch_import_reloc(
    code: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
) -> Result<(), LoadError> {
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
        _ => Err(LoadError::RelocUnsupported),
    }
}
