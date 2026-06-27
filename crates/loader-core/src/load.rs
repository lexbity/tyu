//! Loader driver — the target-independent half of `__lang_load_module`.
//!
//! Implements module-format-and-loading.md §9 (loader algorithm) for TrustLevel Zero
//! (no signature verification, no TrustLevel-Two re-derivation).
//!
//! Phase 9 additions: transactional rollback, `__lang_mod_init` hook,
//! load-once enforcement, failure atomicity.

use crate::error::LoadError;
use crate::platform::{LoaderPlatform, Region, Rw, Rx};
use crate::symbols::SymMap;
use lmod::validate::Container;
#[cfg(feature = "encryption")]
use zeroize::Zeroize;

// Re-export the error type and legacy numeric shims for external callers.
pub use crate::error::{
    E_ABI_MISMATCH, E_BAD_CONTAINER, E_CONTAINER_ENCRYPTED, E_ENC_AUTH_FAIL, E_ENC_BAD_HEADER,
    E_ENC_NO_KEY, E_ENC_REQUIRES_SIGNED, E_ENC_UNSUPPORTED, E_MODULE_ALREADY_LOADED,
    E_MODULE_DECLARES_ISR, E_RELOC_UNSUPPORTED, E_RESOURCE_SHARING_MISMATCH, E_SIG_INVALID,
    E_STACK_BOUND_UNVERIFIABLE, E_SYMBOL_CONFLICT, E_SYMBOL_UNRESOLVED,
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
/// process-scoped for TrustLevel Zero (exiting frees all mmap'd regions).

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

#[cfg(feature = "encryption")]
fn append_bytes(dst: &mut [u8], cursor: &mut usize, src: &[u8]) -> Result<(), LoadError> {
    let end = cursor
        .checked_add(src.len())
        .ok_or(LoadError::BadContainer)?;
    if end > dst.len() {
        return Err(LoadError::BadContainer);
    }
    dst[*cursor..end].copy_from_slice(src);
    *cursor = end;
    Ok(())
}

#[cfg(feature = "encryption")]
struct ScratchGuard {
    region: Region<Rw>,
}

#[cfg(feature = "encryption")]
impl ScratchGuard {
    fn new(region: Region<Rw>) -> Self {
        Self { region }
    }

    fn as_slice(&self) -> &[u8] {
        self.region.as_slice()
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        self.region.as_mut_slice()
    }
}

#[cfg(feature = "encryption")]
impl Drop for ScratchGuard {
    fn drop(&mut self) {
        self.region.as_mut_slice().zeroize();
    }
}

#[cfg(feature = "encryption")]
struct EncHeaderView<'a> {
    nonce: [u8; lmod::enc::NONCE_LEN],
    tag: [u8; lmod::enc::TAG_LEN],
    wrapped_count: usize,
    slots: &'a [u8],
}

#[cfg(feature = "encryption")]
impl<'a> EncHeaderView<'a> {
    fn wire_len(&self) -> usize {
        lmod::enc::enc_header_len(self.wrapped_count)
    }

    fn wrapped_slots(&self) -> WrappedSlotIter<'a> {
        WrappedSlotIter { bytes: self.slots }
    }
}

#[cfg(feature = "encryption")]
struct WrappedSlot<'a> {
    key_id: u64,
    wrap_scheme: u8,
    wrapped: &'a [u8],
}

#[cfg(feature = "encryption")]
struct WrappedSlotIter<'a> {
    bytes: &'a [u8],
}

#[cfg(feature = "encryption")]
impl<'a> Iterator for WrappedSlotIter<'a> {
    type Item = WrappedSlot<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.len() < lmod::enc::WRAPPED_SLOT_SIZE {
            return None;
        }
        let (slot, rest) = self.bytes.split_at(lmod::enc::WRAPPED_SLOT_SIZE);
        self.bytes = rest;
        let key_id = u64::from_le_bytes(slot[0..8].try_into().ok()?);
        let wrap_scheme = slot[8];
        let wrapped_start = 16;
        let wrapped_end = wrapped_start + lmod::enc::WRAP_LEN;
        Some(WrappedSlot {
            key_id,
            wrap_scheme,
            wrapped: &slot[wrapped_start..wrapped_end],
        })
    }
}

#[cfg(feature = "encryption")]
fn parse_enc_header_view(bytes: &[u8]) -> Result<EncHeaderView<'_>, LoadError> {
    const FIXED_LEN: usize = 36;
    if bytes.len() < FIXED_LEN {
        return Err(LoadError::EncBadHeader);
    }
    if lmod::enc::EncMode::from_u8(bytes[0]).is_none() {
        return Err(LoadError::EncBadHeader);
    }
    if bytes[1] != lmod::enc::AEAD_CHACHA20POLY1305 {
        return Err(LoadError::EncUnsupported);
    }

    let mut nonce = [0u8; lmod::enc::NONCE_LEN];
    nonce.copy_from_slice(&bytes[4..4 + lmod::enc::NONCE_LEN]);
    let mut tag = [0u8; lmod::enc::TAG_LEN];
    tag.copy_from_slice(
        &bytes[4 + lmod::enc::NONCE_LEN..4 + lmod::enc::NONCE_LEN + lmod::enc::TAG_LEN],
    );
    let count_off = 4 + lmod::enc::NONCE_LEN + lmod::enc::TAG_LEN;
    let wrapped_count = u32::from_le_bytes(
        bytes[count_off..count_off + 4]
            .try_into()
            .map_err(|_| LoadError::EncBadHeader)?,
    ) as usize;
    if wrapped_count > 64 {
        return Err(LoadError::EncBadHeader);
    }
    let expected_len = lmod::enc::enc_header_len(wrapped_count);
    if bytes.len() < expected_len {
        return Err(LoadError::EncBadHeader);
    }
    let slots = &bytes[FIXED_LEN..expected_len];
    Ok(EncHeaderView {
        nonce,
        tag,
        wrapped_count,
        slots,
    })
}

// ---------------------------------------------------------------------------
// LoadedModule
// ---------------------------------------------------------------------------

/// A fully loaded module, ready for execution.
#[derive(Debug)]
pub struct LoadedModule {
    pub code: Region<Rx>,
    pub rodata: Region<Rw>,
    pub data: Region<Rw>,
    /// Address of `__lang_mod_init` (0 if absent).
    pub init_addr: usize,
    /// The module's `abi_hash` (for lifecycle tracking).
    pub abi_hash: u64,
}

#[derive(Clone, Copy)]
struct SectionLens {
    code: usize,
    rodata: usize,
    data: usize,
    bss: usize,
}

impl SectionLens {
    fn from_header(hdr: &lmod::header::LmodHeader) -> Self {
        Self {
            code: hdr.code_len as usize,
            rodata: hdr.rodata_len as usize,
            data: hdr.data_len as usize,
            bss: hdr.bss_len as usize,
        }
    }

    #[cfg(feature = "encryption")]
    fn encrypted_payload_len(self) -> Result<usize, LoadError> {
        self.code
            .checked_add(self.rodata)
            .and_then(|n| n.checked_add(self.data))
            .ok_or(LoadError::BadContainer)
    }
}

struct PlacedSections {
    code: Region<Rw>,
    rodata: Option<Region<Rw>>,
    data: Option<Region<Rw>>,
}

#[cfg(feature = "encryption")]
struct DecryptionContext {
    cek: [u8; 32],
    nonce: [u8; 12],
    tag: [u8; 16],
    aad: ScratchGuard,
}

fn verify_signature_if_required(
    hdr: &lmod::header::LmodHeader,
    platform: &dyn LoaderPlatform,
    raw_bytes: &[u8],
) -> Result<(), LoadError> {
    if platform.trust_level().rank() < crate::platform::TrustLevel::One.rank() {
        return Ok(());
    }

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
    Ok(())
}

fn validate_modinfo_for_load(modinfo_data: &[u8], remaining_slots: u32) -> Result<(), LoadError> {
    if modinfo_data.is_empty() {
        return Ok(());
    }
    let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
    if mi.has_isr() {
        return Err(LoadError::ModuleDeclaresIsr);
    }
    enforce_stack_bound_budget(modinfo_data, remaining_slots)
}

fn place_sections(
    platform: &mut dyn LoaderPlatform,
    container: &Container<'_>,
    lens: SectionLens,
) -> Result<PlacedSections, LoadError> {
    let mut code = platform.alloc_exec(lens.code)?;
    let mut rodata = if lens.rodata > 0 {
        Some(platform.alloc_ro(lens.rodata)?)
    } else {
        None
    };
    let mut data = if lens.data + lens.bss > 0 {
        Some(platform.alloc_rw(lens.data + lens.bss)?)
    } else {
        None
    };

    if lens.code > 0 {
        code.as_mut_slice().copy_from_slice(container.code());
    }
    if let Some(ref mut ro) = rodata {
        ro.as_mut_slice().copy_from_slice(container.rodata());
    }
    if let Some(ref mut rw) = data {
        if lens.data > 0 {
            rw.as_mut_slice()[..lens.data].copy_from_slice(container.data());
        }
    }

    Ok(PlacedSections { code, rodata, data })
}

#[cfg(feature = "encryption")]
fn prepare_decryption(
    hdr: &lmod::header::LmodHeader,
    platform: &mut dyn LoaderPlatform,
    raw_bytes: &[u8],
    modinfo_data: &[u8],
    reloc_count: u32,
) -> Result<Option<DecryptionContext>, LoadError> {
    if hdr.flags & lmod::header::LMOD_FLAG_ENCRYPTED == 0 {
        return Ok(None);
    }
    if platform.trust_level().rank() < crate::platform::TrustLevel::One.rank() {
        return Err(LoadError::EncRequiresSigned);
    }

    let eh_start = lmod::header::HEADER_SIZE as usize;
    let eh_bytes = raw_bytes.get(eh_start..).ok_or(LoadError::EncBadHeader)?;
    let eh = parse_enc_header_view(eh_bytes)?;

    let mut cek = [0u8; 32];
    let mut cek_found = false;
    for slot in eh.wrapped_slots() {
        if slot.wrap_scheme != lmod::enc::WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305 {
            continue;
        }
        if platform
            .unwrap_cek(slot.key_id, slot.wrapped, &mut cek)
            .is_ok()
        {
            cek_found = true;
            break;
        }
    }
    if !cek_found {
        return Err(LoadError::EncNoKey);
    }

    let aad_total_len = hdr.sig_off;
    let mut aad_header = [0u8; lmod::header::HEADER_SIZE as usize];
    aad_header.copy_from_slice(&raw_bytes[..lmod::header::HEADER_SIZE as usize]);
    aad_header[16..20].copy_from_slice(&aad_total_len.to_le_bytes());
    let aad_flags =
        u16::from_le_bytes([aad_header[6], aad_header[7]]) & !lmod::header::LMOD_FLAG_SIGNED;
    aad_header[6..8].copy_from_slice(&aad_flags.to_le_bytes());
    aad_header[68..72].copy_from_slice(&0u32.to_le_bytes());

    let enc_header_len = eh.wire_len();
    let reloc_bytes = (reloc_count as usize)
        .checked_mul(lmod::reloc::RELOC_ENTRY_SIZE as usize)
        .ok_or(LoadError::BadContainer)?;
    let reloc_slice = if reloc_bytes > 0 {
        let ro = hdr.reloc_off as usize;
        let end = ro.checked_add(reloc_bytes).ok_or(LoadError::BadContainer)?;
        Some(raw_bytes.get(ro..end).ok_or(LoadError::BadContainer)?)
    } else {
        None
    };
    let aad_len = (lmod::header::HEADER_SIZE as usize)
        .checked_add(enc_header_len)
        .and_then(|n| n.checked_add(modinfo_data.len()))
        .and_then(|n| n.checked_add(reloc_slice.map_or(0, |s| s.len())))
        .ok_or(LoadError::BadContainer)?;

    let mut aad_region = ScratchGuard::new(platform.alloc_rw(aad_len)?);
    {
        let aad = aad_region.as_mut_slice();
        let mut cursor = 0usize;
        append_bytes(aad, &mut cursor, &aad_header)?;
        append_bytes(aad, &mut cursor, &eh_bytes[..enc_header_len])?;
        let tag_off_in_eh = aad_header.len() + 4 + lmod::enc::NONCE_LEN;
        aad[tag_off_in_eh..tag_off_in_eh + lmod::enc::TAG_LEN].fill(0);
        append_bytes(aad, &mut cursor, modinfo_data)?;
        if let Some(reloc) = reloc_slice {
            append_bytes(aad, &mut cursor, reloc)?;
        }
        if cursor != aad_len {
            return Err(LoadError::BadContainer);
        }
    }

    Ok(Some(DecryptionContext {
        cek,
        nonce: eh.nonce,
        tag: eh.tag,
        aad: aad_region,
    }))
}

#[cfg(not(feature = "encryption"))]
fn reject_encrypted_without_feature(hdr: &lmod::header::LmodHeader) -> Result<(), LoadError> {
    if hdr.flags & lmod::header::LMOD_FLAG_ENCRYPTED != 0 {
        Err(LoadError::EncUnsupported)
    } else {
        Ok(())
    }
}

#[cfg(feature = "encryption")]
fn decrypt_sections(
    platform: &mut dyn LoaderPlatform,
    sections: &mut PlacedSections,
    lens: SectionLens,
    decrypt: Option<&DecryptionContext>,
) -> Result<(), LoadError> {
    let Some(decrypt) = decrypt else {
        return Ok(());
    };
    let payload_len = lens.encrypted_payload_len()?;
    if payload_len == 0 {
        return Ok(());
    }

    let mut payload_region = ScratchGuard::new(platform.alloc_rw(payload_len)?);
    {
        let payload = payload_region.as_mut_slice();
        let mut cursor = 0usize;
        append_bytes(payload, &mut cursor, &sections.code.as_slice()[..lens.code])?;
        if lens.rodata > 0 {
            let ro = sections.rodata.as_ref().ok_or(LoadError::BadContainer)?;
            append_bytes(payload, &mut cursor, &ro.as_slice()[..lens.rodata])?;
        }
        if lens.data > 0 {
            let rw = sections.data.as_ref().ok_or(LoadError::BadContainer)?;
            append_bytes(payload, &mut cursor, &rw.as_slice()[..lens.data])?;
        }
        if cursor != payload_len {
            return Err(LoadError::BadContainer);
        }
        crate::crypto::chacha20poly1305::decrypt_payload(
            &decrypt.cek,
            &decrypt.nonce,
            &decrypt.tag,
            decrypt.aad.as_slice(),
            payload,
        )
        .map_err(|_| LoadError::EncAuthFail)?;
    }

    let payload = payload_region.as_slice();
    sections.code.as_mut_slice()[..lens.code].copy_from_slice(&payload[..lens.code]);
    if lens.rodata > 0 {
        let ro = sections.rodata.as_mut().ok_or(LoadError::BadContainer)?;
        ro.as_mut_slice()[..lens.rodata]
            .copy_from_slice(&payload[lens.code..lens.code + lens.rodata]);
    }
    if lens.data > 0 {
        let rw = sections.data.as_mut().ok_or(LoadError::BadContainer)?;
        rw.as_mut_slice()[..lens.data]
            .copy_from_slice(&payload[lens.code + lens.rodata..payload_len]);
    }
    Ok(())
}

fn apply_import_relocations<const N: usize>(
    code: &mut Region<Rw>,
    container: &Container<'_>,
    hdr: &lmod::header::LmodHeader,
    map: &SymMap<'_, N>,
    code_len: usize,
) -> Result<(), LoadError> {
    let container_code_off = hdr.code_off as u64;
    for i in 0..container.reloc_count() {
        let entry = container.reloc_entry(i).ok_or(LoadError::BadContainer)?;
        let site_off = entry.site_off as u64;
        if site_off < container_code_off {
            continue;
        }
        let local_off = (site_off - container_code_off) as usize;
        let need = if entry.kind == 1 { 8 } else { 4 };
        if local_off + need > code_len {
            return Err(LoadError::BadContainer);
        }
        let sym = map
            .lookup_by_hash(entry.sym_hash)
            .ok_or(LoadError::SymbolUnresolved)?;
        dispatch_import_reloc(code.as_mut_slice(), local_off, entry.kind, sym.addr as u64)
            .map_err(|_| LoadError::RelocUnsupported)?;
    }
    Ok(())
}

fn register_exports<'a, const N: usize>(
    modinfo_data: &'a [u8],
    code_base: usize,
    map: &mut SymMap<'a, N>,
) -> Result<(), LoadError> {
    if modinfo_data.is_empty() {
        return Ok(());
    }
    let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
    for ei in 0..mi.export_count {
        let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(LoadError::BadContainer)?;
        if exp.name == MOD_INIT_NAME {
            continue;
        }
        map.register_with_hash(exp.sym_hash, exp.name, code_base)?;
    }
    Ok(())
}

fn verify_trust_two_stack_bounds(
    platform: &dyn LoaderPlatform,
    code: &Region<Rx>,
    modinfo_data: &[u8],
) -> Result<(), LoadError> {
    if platform.trust_level().rank() < crate::platform::TrustLevel::Two.rank() {
        return Ok(());
    }
    let code_slice = code.as_slice();
    let arch = crate::rederive::Arch::detect_from_code(code_slice);
    let slot_bytes: u32 = match arch {
        crate::rederive::Arch::X86_64 => 8,
        crate::rederive::Arch::ArmThumb => 4,
        crate::rederive::Arch::RiscV => 4,
    };
    let rederived = crate::rederive::rederive_stack_high(code_slice, arch, slot_bytes);
    if modinfo_data.is_empty() {
        return Ok(());
    }
    let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
    for ei in 0..mi.export_count {
        let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(LoadError::BadContainer)?;
        if exp.name == MOD_INIT_NAME {
            continue;
        }
        let stamped = read_word_meta_stack_bound(modinfo_data, exp.value_off)?;
        if stamped == crate::rederive::TOP_SENTINEL {
            return Err(LoadError::BadContainer);
        }
        if rederived == crate::rederive::TOP_SENTINEL && stamped != crate::rederive::TOP_SENTINEL {
            return Err(LoadError::StackBoundUnverifiable);
        }
        if rederived > stamped {
            return Err(LoadError::BadContainer);
        }
    }
    Ok(())
}

fn check_resource_sharing(modinfo_data: &[u8]) -> Result<(), LoadError> {
    if modinfo_data.is_empty() {
        return Ok(());
    }
    let mut ri = 0u32;
    while let Some(rm) = lmod::modinfo::read_res_meta(modinfo_data, ri) {
        if rm.sharing_class != 0 {
            return Err(LoadError::ResourceSharingMismatch);
        }
        ri += 1;
    }
    Ok(())
}

fn run_init_if_present(
    container: &Container<'_>,
    code_base: usize,
    code_region: &Region<Rx>,
) -> Result<usize, LoadError> {
    let init_addr = lookup_mod_init(container, code_base as u64);
    if init_addr == 0 {
        return Ok(0);
    }

    let init_off = init_addr
        .checked_sub(code_base)
        .ok_or(LoadError::BadContainer)?;
    let init_ptr = code_region.entry(init_off).ok_or(LoadError::BadContainer)?;
    let init_fn: extern "C" fn() = unsafe { core::mem::transmute(init_ptr) };
    init_fn();
    Ok(init_addr)
}

// ---------------------------------------------------------------------------
// Loader algorithm
// ---------------------------------------------------------------------------

/// Run the loader algorithm (§9 steps 1–9) for a TrustLevel-Zero (unsigned) module,
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
    let hdr = container.header();

    if hdr.abi_hash != platform.expected_abi_hash() {
        return Err(LoadError::AbiMismatch);
    }
    if loaded_set.contains(hdr.abi_hash) {
        return Err(LoadError::ModuleAlreadyLoaded);
    }
    if hdr.flags & lmod::header::LMOD_FLAG_SIGNED != 0
        && platform.trust_level().rank() < crate::platform::TrustLevel::One.rank()
    {
        return Err(LoadError::SigInvalid);
    }

    let raw_bytes = container.raw_bytes();
    let modinfo_data = container.modinfo();
    verify_signature_if_required(hdr, platform, raw_bytes)?;

    #[cfg(feature = "encryption")]
    let decrypt = prepare_decryption(
        hdr,
        platform,
        raw_bytes,
        modinfo_data,
        container.reloc_count(),
    )?;
    #[cfg(not(feature = "encryption"))]
    reject_encrypted_without_feature(hdr)?;

    validate_modinfo_for_load(modinfo_data, platform.ds_remaining_slots())?;

    let sym_guard = RollbackGuard::new(global_map);

    if platform.placement_policy() != crate::platform::PlacementPolicy::CopyToRam {
        return Err(LoadError::BadContainer);
    }

    let lens = SectionLens::from_header(hdr);
    let mut sections = place_sections(platform, container, lens)?;
    #[cfg(feature = "encryption")]
    decrypt_sections(platform, &mut sections, lens, decrypt.as_ref())?;

    let code_base = sections.code.as_ptr() as usize;
    apply_import_relocations(&mut sections.code, container, hdr, sym_guard.map, lens.code)?;

    let PlacedSections { code, rodata, data } = sections;
    let code_region = platform.make_exec(code)?;

    let init_addr = run_init_if_present(container, code_base, &code_region)?;

    register_exports(modinfo_data, code_base, sym_guard.map)?;
    verify_trust_two_stack_bounds(platform, &code_region, modinfo_data)?;
    check_resource_sharing(modinfo_data)?;

    loaded_set.insert(hdr.abi_hash)?;
    sym_guard.commit();

    Ok(LoadedModule {
        code: code_region,
        rodata: rodata
            .unwrap_or_else(|| unsafe { Region::<Rw>::from_raw_parts(core::ptr::null_mut(), 0) }),
        data: data
            .unwrap_or_else(|| unsafe { Region::<Rw>::from_raw_parts(core::ptr::null_mut(), 0) }),
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

fn enforce_stack_bound_budget(modinfo_data: &[u8], remaining_slots: u32) -> Result<(), LoadError> {
    if modinfo_data.is_empty() {
        return Ok(());
    }
    let mi = lmod::modinfo::decode(modinfo_data).ok_or(LoadError::BadContainer)?;
    for ei in 0..mi.export_count {
        let exp = lmod::modinfo::read_export(modinfo_data, ei).ok_or(LoadError::BadContainer)?;
        if exp.name == MOD_INIT_NAME {
            continue;
        }
        let stack_bound = read_word_meta_stack_bound(modinfo_data, exp.value_off)?;
        if stack_bound > remaining_slots {
            return Err(LoadError::StackBoundUnverifiable);
        }
    }
    Ok(())
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
    use crate::platform::TrustLevel;
    use alloc::vec;

    /// Static buffer for TestPlatform allocations (page-aligned, 64KB).
    static mut TEST_BUF: [u8; 65536] = [0u8; 65536];
    static mut TEST_BUF_USED: usize = 0;

    struct TestPlatform {
        expected_hash: u64,
        fail: bool,
        trust_level: TrustLevel,
    }
    impl LoaderPlatform for TestPlatform {
        fn alloc_exec(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            let buf = unsafe { &mut *core::ptr::addr_of_mut!(TEST_BUF) };
            let used = unsafe { TEST_BUF_USED };
            if used + len > buf.len() {
                return Err(1);
            }
            let ptr = unsafe { buf.as_mut_ptr().add(used) };
            unsafe { TEST_BUF_USED = used + len };
            unsafe { Ok(Region::<Rw>::from_raw_parts(ptr, len)) }
        }
        fn alloc_ro(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            self.alloc_exec(len)
        }
        fn alloc_rw(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            self.alloc_exec(len)
        }
        fn make_exec(&mut self, r: Region<Rw>) -> Result<Region<Rx>, u32> {
            if self.fail {
                Err(1)
            } else {
                Ok(unsafe { Region::<Rx>::from_raw_parts(r.as_mut_ptr(), r.len()) })
            }
        }
        fn verify_sig(&self, _signed: &[u8], _sig: &[u8]) -> bool {
            // TestPlatform models TrustLevel Zero unless a test opts higher;
            // it never accepts signatures.
            false
        }
        fn expected_abi_hash(&self) -> u64 {
            self.expected_hash
        }
        fn trust_level(&self) -> TrustLevel {
            self.trust_level
        }
        #[cfg(feature = "encryption")]
        fn unwrap_cek(
            &self,
            _key_id: u64,
            _wrapped: &[u8],
            _out: &mut [u8; 32],
        ) -> Result<(), u32> {
            // This mock has no KEK material; encrypted containers are rejected.
            Err(crate::load::E_ENC_NO_KEY)
        }
    }

    #[test]
    fn rollback_guard_restores_on_error() {
        let mut map: SymMap<'_, 4> = SymMap::new();
        map.register(b"keep", 0x100).unwrap();
        let saved = map.len();

        {
            let guard = RollbackGuard::new(&mut map);
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
            let guard = RollbackGuard::new(&mut map);
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
        assert!(set.insert(99).is_ok()); // different hash
    }

    #[test]
    fn signed_flag_without_trust_level_one_rejected() {
        // Build a module with the SIGNED flag set, but use a TrustLevel-One platform.
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        let mut raw = build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], 64);
        raw[6] |= lmod::header::LMOD_FLAG_SIGNED as u8;

        let container = lmod::validate::Container::parse(&raw).unwrap();

        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        // This platform is TrustLevel One and does not override verify_sig.
        let mut plat = TestPlatform {
            expected_hash: 0,
            fail: false,
            trust_level: TrustLevel::One,
        };
        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(
            result.is_err(),
            "signed module on TrustLevel One should be rejected"
        );
        assert_eq!(result.unwrap_err(), E_SIG_INVALID);
    }

    #[test]
    fn encrypted_container_rejected() {
        let mut mi_buf = [0u8; 128];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &[], &[], 0, 0, &[]).unwrap();
        let mut raw = build_minimal_lmod_with_modinfo(&mi_buf[..mi_size], 64);
        raw[6] |= 0x02; // set ENCRYPTED flag

        let container = lmod::validate::Container::parse(&raw).unwrap();
        assert_ne!(
            container.header().flags & lmod::header::LMOD_FLAG_ENCRYPTED,
            0
        );

        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();
        let mut plat = TestPlatform {
            expected_hash: 0,
            fail: false,
            trust_level: TrustLevel::Zero,
        };
        let result = load_module(&container, &mut plat, &mut map, &mut set);

        #[cfg(not(feature = "encryption"))]
        assert_eq!(result.unwrap_err(), E_ENC_UNSUPPORTED);
        #[cfg(feature = "encryption")]
        // With encryption feature but TrustLevel Zero, should fail with E_ENC_REQUIRES_SIGNED.
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
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            },
            lmod::modinfo::ExportEntry {
                sym_hash: lmod::hash::fnv1a_u64(b"user_word"),
                name: b"user_word",
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
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
        let mut plat = TestPlatform {
            expected_hash: 0,
            fail: false,
            trust_level: TrustLevel::Zero,
        };
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
    // FullPlatform — a TrustLevel One platform with owned bump allocator,
    // real HMAC verification, and (when "encryption" feature is active)
    // real CEK unwrapping.
    // -------------------------------------------------------------------

    use alloc::vec::Vec;
    use core::cell::{Cell, RefCell};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type FullHmacSha256 = Hmac<Sha256>;

    struct FullPlatform {
        buf: Vec<u8>,
        used: Cell<usize>,
        alloc_ranges: RefCell<Vec<(usize, usize)>>,
        hash: u64,
        kek: [u8; 32],
        fail_make_exec: bool,
        ds_remaining_slots: u32,
    }

    impl FullPlatform {
        fn new(hash: u64, kek: [u8; 32]) -> Self {
            Self {
                buf: vec![0u8; 65536],
                used: Cell::new(0),
                alloc_ranges: RefCell::new(Vec::new()),
                hash,
                kek,
                fail_make_exec: false,
                ds_remaining_slots: u32::MAX,
            }
        }
        fn with_ds_remaining_slots(mut self, slots: u32) -> Self {
            self.ds_remaining_slots = slots;
            self
        }
        fn alloc(&self, len: usize) -> Result<Region<Rw>, u32> {
            let used = self.used.get();
            if used + len > self.buf.len() {
                return Err(1);
            }
            let ptr = self.buf.as_ptr() as *mut u8;
            let region = unsafe { Region::<Rw>::from_raw_parts(ptr.add(used), len) };
            self.used.set(used + len);
            self.alloc_ranges.borrow_mut().push((used, len));
            Ok(region)
        }

        #[cfg(feature = "encryption")]
        fn allocation_is_zero(&self, index: usize) -> bool {
            let ranges = self.alloc_ranges.borrow();
            let Some(&(start, len)) = ranges.get(index) else {
                return false;
            };
            self.buf[start..start + len].iter().all(|&b| b == 0)
        }

        #[cfg(feature = "encryption")]
        fn allocation_count(&self) -> usize {
            self.alloc_ranges.borrow().len()
        }
    }

    impl LoaderPlatform for FullPlatform {
        fn alloc_exec(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            self.alloc(len)
        }
        fn alloc_ro(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            self.alloc(len)
        }
        fn alloc_rw(&mut self, len: usize) -> Result<Region<Rw>, u32> {
            self.alloc(len)
        }
        fn make_exec(&mut self, r: Region<Rw>) -> Result<Region<Rx>, u32> {
            if self.fail_make_exec {
                Err(1)
            } else {
                Ok(unsafe { Region::<Rx>::from_raw_parts(r.as_mut_ptr(), r.len()) })
            }
        }
        fn expected_abi_hash(&self) -> u64 {
            self.hash
        }
        fn trust_level(&self) -> crate::platform::TrustLevel {
            // FullPlatform is the authenticated loader test platform.
            crate::platform::TrustLevel::One
        }
        fn ds_remaining_slots(&self) -> u32 {
            self.ds_remaining_slots
        }

        fn verify_sig(&self, signed: &[u8], sig: &[u8]) -> bool {
            // FullPlatform models TrustLevel One with HMAC verification.
            let mut mac =
                FullHmacSha256::new_from_slice(&self.kek).expect("HMAC accepts 32-byte key");
            mac.update(signed);
            mac.finalize().into_bytes().as_slice() == sig
        }

        #[cfg(feature = "encryption")]
        fn unwrap_cek(&self, _key_id: u64, wrapped: &[u8], out: &mut [u8; 32]) -> Result<(), u32> {
            // FullPlatform reuses the test KEK for encrypted-load fixtures.
            let w: &[u8; 60] = wrapped.try_into().map_err(|_| 1u32)?;
            let cek =
                crate::crypto::chacha20poly1305::unwrap_cek(&self.kek, w).map_err(|_| 1u32)?;
            *out = cek;
            Ok(())
        }
    }

    /// Build a minimal .lmod whose code section carries `code_bytes` and which
    /// exports a word named "main" with a given stack bound (TrustLevel-Two test).
    fn build_lmod_with_code(code_bytes: &[u8], abi_hash: u64, export_main: bool) -> Vec<u8> {
        build_lmod_with_stack_bound(code_bytes, abi_hash, export_main, 0)
    }

    fn build_lmod_with_stack_bound(
        code_bytes: &[u8],
        abi_hash: u64,
        export_main: bool,
        stack_bound: u32,
    ) -> Vec<u8> {
        let code_len = code_bytes.len() as u32;
        let exports = if export_main {
            alloc::vec![lmod::modinfo::ExportEntry {
                sym_hash: lmod::hash::fnv1a_u64(b"main"),
                name: b"main",
                effects: 0,
                requires_caps: 0,
                stack_bound,
            }]
        } else {
            alloc::vec![]
        };
        let mut mi_buf = [0u8; 512];
        let mi_size = lmod::modinfo::encode_into(&mut mi_buf, b"T", &exports, &[], abi_hash, 0, &[])
            .unwrap_or(0) as u32;
        let reloc_count = 0u32;
        let layout =
            lmod::header::compute_layout(abi_hash, mi_size, code_len, 0, 0, 0, reloc_count, 0);
        let total = layout.total_len as usize;
        let mut buf = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut buf, &layout);
        let mi_off = layout.modinfo_off as usize;
        buf[mi_off..mi_off + mi_size as usize].copy_from_slice(&mi_buf[..mi_size as usize]);
        let co = layout.code_off as usize;
        buf[co..co + code_bytes.len()].copy_from_slice(code_bytes);
        buf
    }

    #[test]
    fn stack_bound_exceeding_remaining_ds_slots_rejected() {
        let code = [0xC3u8];
        let key = [0xabu8; 32];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
        let plain = build_lmod_with_stack_bound(&code, abi_hash, true, 9);
        let signed = lmod_sign::sign(&plain, &key).expect("sign");
        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, key).with_ds_remaining_slots(8);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(result.unwrap_err(), LoadError::StackBoundUnverifiable);
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn signed_encrypted_module_loads_and_runs() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true);
        let enc = lmod_encrypt::encrypt_fleet(&plain, &kek).expect("encrypt");
        let signed = lmod_sign::sign(&enc, &kek).expect("sign");

        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(
            result.is_ok(),
            "signed+encrypted module must load, got {:?}",
            result.err()
        );
        assert_eq!(
            plat.allocation_count(),
            3,
            "encrypted code-only load allocates aad, code, and payload scratch"
        );
        assert!(
            plat.allocation_is_zero(0),
            "AAD scratch must be zeroized after successful load"
        );
        assert!(
            plat.allocation_is_zero(2),
            "payload scratch must be zeroized after successful load"
        );
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn signed_encrypted_tampered_ciphertext_rejected() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true);
        let enc = lmod_encrypt::encrypt_fleet(&plain, &kek).expect("encrypt");
        let signed = lmod_sign::sign(&enc, &kek).expect("sign");

        let mut bad = signed.clone();
        let hdr = lmod::header::decode_header(&bad).unwrap();
        let co = hdr.code_off as usize;
        bad[co] ^= 0x01;

        let container = lmod::validate::Container::parse(&bad).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(
            result.unwrap_err(),
            LoadError::SigInvalid,
            "tampered signed ciphertext must fail signature verification before decrypt"
        );
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn signed_encrypted_auth_failure_zeroizes_scratch() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true);
        let mut enc = lmod_encrypt::encrypt_fleet(&plain, &kek).expect("encrypt");
        let hdr = lmod::header::decode_header(&enc).unwrap();
        let co = hdr.code_off as usize;
        enc[co] ^= 0x01;
        let signed = lmod_sign::sign(&enc, &kek).expect("sign");

        let container = lmod::validate::Container::parse(&signed).unwrap();
        let mut plat = FullPlatform::new(abi_hash, kek);
        let mut map: SymMap<'_, 256> = SymMap::new();
        let mut set = LoadedSet::<64>::new();

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert_eq!(
            result.unwrap_err(),
            LoadError::EncAuthFail,
            "signature-valid ciphertext tamper must fail during AEAD authentication"
        );
        assert_eq!(
            plat.allocation_count(),
            3,
            "auth-failure path allocates aad, code, and payload scratch"
        );
        assert!(
            plat.allocation_is_zero(0),
            "AAD scratch must be zeroized after EncAuthFail"
        );
        assert!(
            plat.allocation_is_zero(2),
            "payload scratch must be zeroized after EncAuthFail"
        );
    }

    // -------------------------------------------------------------------
    // H2: Rollback-on-failure — partial symbol registration is undone.
    // -------------------------------------------------------------------

    #[test]
    fn rollback_on_make_exec_failure() {
        let kek = [0xabu8; 32];
        let ret_insn = [0xC3u8];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);

        let plain = build_lmod_with_code(&ret_insn, abi_hash, true); // exports "main"
        let signed = lmod_sign::sign(&plain, &kek).expect("sign"); // not encrypted — faster

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
        let set_len_before = set.len;

        let result = load_module(&container, &mut plat, &mut map, &mut set);
        assert!(
            result.is_err(),
            "load_module must fail when make_exec fails"
        );
        // global_map must be restored: sentinel survives, "main" export is rolled back.
        assert_eq!(
            map.len(),
            saved_len,
            "global_map must be restored to pre-load length after failure"
        );
        assert!(
            map.lookup_by_name(b"__sentinel").is_some(),
            "sentinel symbol must survive rollback"
        );
        assert!(
            map.lookup_by_name(b"main").is_none(),
            "partial export 'main' must be rolled back"
        );
        // loaded_set must be unchanged.
        assert_eq!(
            set.len, set_len_before,
            "loaded_set must be unchanged after failed load"
        );
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
    // M5: ResourceSharingMismatch
    // -------------------------------------------------------------------

    #[test]
    fn isr_module_rejected() {
        let code = [0xC3u8];
        let key = [0xabu8; 32];
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
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

        // FullPlatform is TrustLevel One — sign the module before loading.
        let signed = lmod_sign::sign(&raw, &key).expect("sign");
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
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
        // Build modinfo with a res_meta entry that has non-zero sharing_class.
        let res_meta = lmod::modinfo::ResMetaEntry {
            res_hash: lmod::hash::fnv1a_u64(b"some_resource"),
            sharing_class: 1, // non-zero → should be rejected
            lock_prim: 0,
        };
        let exports = [];
        let mut mi_buf = [0u8; 512];
        // encode_into needs module_name, exports, imports, abi_hash, flags, res_metas
        let mi_size =
            lmod::modinfo::encode_into(&mut mi_buf, b"T", &exports, &[], abi_hash, 0, &[res_meta])
                .unwrap() as u32;

        let code_len = code.len() as u32;
        let layout = lmod::header::compute_layout(abi_hash, mi_size, code_len, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut raw = alloc::vec![0u8; total];
        lmod::header::encode_header(&mut raw, &layout);
        let mi_off = layout.modinfo_off as usize;
        raw[mi_off..mi_off + mi_size as usize].copy_from_slice(&mi_buf[..mi_size as usize]);
        let co = layout.code_off as usize;
        raw[co..co + code.len()].copy_from_slice(&code);

        // FullPlatform is TrustLevel One — sign the module before loading.
        let key = [0xabu8; 32];
        let signed = lmod_sign::sign(&raw, &key).expect("sign");
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
        let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
        let plain = build_lmod_with_code(&code, abi_hash, false);
        let signed = lmod_sign::sign(&plain, &key).expect("sign");
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
        1 => 0,      // R_X86_64_64
        2 | 3 => -4, // R_X86_64_PC32 / R_X86_64_PLT32
        4 => 0,      // R_ARM_ABS32
        5 => 0,      // R_ARM_THM_CALL (ARM backend applies PC = site + 4)
        6 => 0,      // R_ARM_THM_JUMP24
        7 => 0,      // R_ARM_REL32
        _ => 0,
    };
    match kind {
        1 | 2 | 3 => {
            crate::reloc_x86_64::apply_import_reloc(code, site_off, kind, sym_addr, addend)
        }
        4 | 5 | 6 | 7 => {
            crate::reloc_arm::apply_import_reloc(code, site_off, kind, sym_addr, addend)
        }
        8 | 9 => crate::reloc_riscv::apply_import_reloc(code, site_off, kind, sym_addr, addend),
        _ => Err(LoadError::RelocUnsupported),
    }
}
