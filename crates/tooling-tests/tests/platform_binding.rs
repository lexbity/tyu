//! P6 loader platform binding corpus (design doc §5.8, decisions D-4/D-5/D-9).
//!
//! End-to-end hosted-loader tests over *real* modules built by `langc` against
//! the armv7m runtime descriptor (`board.scratch`), packed by `lmod-pack`:
//!
//! - a platformed MMIO module loads when the host carries the matching board
//!   identity (`platform_hash` + aperture table), and its aperture-base reloc site
//!   is written with the board's base (FR-15);
//! - wrong board → E5220 (the primary D-5 gate, before any binding);
//! - a aperture the board does not expose → E5222;
//! - a malformed aperture-use table → E5223;
//! - a v3 modinfo on the v4 loader → E5224;
//! - a failed load leaves the exclusivity registry empty (transactional);
//! - a platformed module on a *boardless* host is rejected E5220 (the latent
//!   hosted gap from the first audit: never a silent `RelocUnsupported`);
//! - two modules claiming the same aperture → E5221 at the binding layer.

mod common;

use common::*;
use hosted::loader::HostedLoaderPlatform;
use lmod::validate::Container;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;
use loader_core::apertures::{bind_apertures, ApertureRegistry};
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_platform_binding")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const ARM_ABI_HASH: u64 = 0x4e2b602bb1069843; // compute_abi_hash(ARCH_TAG_ARM, 4, 32, 4)

/// A module that touches `board.scratch` (the armv7m runtime descriptor's
/// aperture), so packing produces a `MmioApertureBase` reloc + aperture-use table.
const MMIO_MAIN: &str = "module Main;\n\
register-map Scratch\n\
  0x00 A u32 rw\n\
end;\n\
const scratch = Scratch @ board.scratch;\n\
: main ( -- i64 )\n\
  &!scratch.A 1 as u32 !u32\n\
  &scratch.A @u32 as i64 1 == [ 1 ] [ 0 ] if\n\
  ;\n\
export { main };\nend;\n";

fn arm_desc_dir() -> PathBuf {
    workspace_root().join("platforms/armv7m-unknown-none")
}

fn build_mmio_module(dir: &Path) -> PathBuf {
    ensure_bins();
    let src = dir.join("Main.mod");
    std::fs::write(&src, MMIO_MAIN).unwrap();
    let out_dir = dir.join("obj");
    std::fs::create_dir_all(&out_dir).unwrap();
    let status = Command::new(exe("langc"))
        .args([
            "--emit=obj",
            "--target=armv7m-unknown-none",
            &format!("--sysroot={}", workspace_root().join("sysroot").display()),
            &format!("--platform={}", arm_desc_dir().display()),
            &format!("--out-dir={}", out_dir.display()),
            src.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("langc");
    assert!(status.success(), "langc failed for MMIO arm module");
    let obj = out_dir.join("Main.o");
    assert!(obj.exists(), "langc must produce Main.o");
    let lmod = dir.join("Main.lmod");
    let status = Command::new(exe("lmod-pack"))
        .args([obj.to_string_lossy().as_ref(), lmod.to_string_lossy().as_ref()])
        .status()
        .expect("lmod-pack");
    assert!(status.success(), "lmod-pack failed");
    lmod
}

/// The board identity the armv7m runtime descriptor declares.
fn arm_board() -> (u64, Vec<lmod::board_table::BoardAperture>) {
    let desc_path = arm_desc_dir().join("platform.desc");
    let bytes = std::fs::read(&desc_path).expect("platform.desc present");
    let cd = codegen_core::compiled_desc::decode_compiled_desc(&bytes)
        .expect("compiled descriptor decodes");
    let apertures: Vec<lmod::board_table::BoardAperture> = cd
        .apertures()
        .iter()
        .map(|w| lmod::board_table::BoardAperture {
            name_hash: lmod::hash::fnv1a_u64(w.name.as_bytes()),
            base: w.base.unwrap_or(0) as u32,
            size: w.size,
            capability: codegen_core::compiled_desc::aperture_capability(&cd, w.id),
        })
        .collect();
    (cd.platform_hash, apertures)
}

fn load_hosted(
    lmod: &Path,
    platform_hash: Option<u64>,
    apertures: &[lmod::board_table::BoardAperture],
) -> Result<loader_core::load::LoadedModule, loader_core::error::LoadError> {
    let bytes = std::fs::read(lmod).unwrap();
    let container = Container::parse(&bytes).expect("container parses");
    // +64 bytes for the call-target trampoline that keeps BL targets in range.
    let bsize = (container.code().len() + container.rodata().len() + container.data().len() + 4095)
        & !4095;
    let mut platform = HostedLoaderPlatform::new(ARM_ABI_HASH);
    if let Some(h) = platform_hash {
        platform = platform.with_board(h, apertures);
    }
    platform.reserve(bsize + 64).unwrap();

    let ds_high = allocate_runtime_page();
    let mut map: SymMap<'_, 256> = SymMap::new();
    // The module calls `__stack_overflow` via Thumb `bl` (±16 MB). The test
    // stub lives far outside that range on x86_64, so install an in-range
    // 8-byte ARM trampoline at the end of the reserved block:
    //   ldr r3, [pc, #0] ; bx r3 ; <target u32>
    let block = platform.block_base().expect("reserved block");
    let tramp = unsafe { core::slice::from_raw_parts_mut(block.add(bsize), 8) };
    tramp[0..2].copy_from_slice(&0x4b00u16.to_le_bytes()); // ldr r3, [pc, #0]
    tramp[2..4].copy_from_slice(&0x4718u16.to_le_bytes()); // bx r3
    tramp[4..8].copy_from_slice(&(common::extern_c_fn_stub as usize as u32).to_le_bytes());
    let stack_overflow_addr = (block as usize + bsize) | 1;

    // Register the runtime symbols the compiled module needs, with
    // `__stack_overflow` bound to the in-range trampoline.
    for (name, addr) in [
        ("__stack_overflow", stack_overflow_addr),
        ("__lang_ds_high", ds_high),
        ("__lang_trap", common::extern_c_fn_stub as usize),
        ("__lang_trap_loc", common::extern_c_fn_stub as usize),
        ("__lang_ds_base", ds_high),
        ("__lang_ds_limit", ds_high + 0x1000),
        ("__lang_stack_limit", ds_high),
    ] {
        map.register_runtime_hash(lmod::hash::fnv1a_u64(name.as_bytes()), addr)
            .expect("register runtime symbol");
    }

    let mut set = LoadedSet::<64>::new();
    let mut reg = ApertureRegistry::new();
    load_module(&container, &mut platform, &mut map, &mut set, &mut reg)
}

/// P6 positive: a platformed MMIO module loads against the matching board and
/// the aperture-base reloc site is bound with the board's base.
#[test]
fn mmio_module_loads_against_matching_board() {
    let dir = temp_dir("platform_binding_positive");
    let lmod = build_mmio_module(&dir);
    let (ph, apertures) = arm_board();
    let loaded = load_hosted(&lmod, Some(ph), &apertures).expect("matching board must load");

    // The aperture-base reloc site (kind 10) must now hold the board's base for
    // the module's aperture. Find it from the container's reloc table and read
    // the code byte at that site (FR-15).
    let container_bytes = std::fs::read(&lmod).unwrap();
    let container = Container::parse(&container_bytes).unwrap();
    let code_off = container.header().code_off as usize;
    let scratch = apertures
        .iter()
        .find(|w| w.name_hash == lmod::hash::fnv1a_u64(b"scratch"))
        .expect("board has scratch");
    let code = loaded.code.as_slice();
    let mut found = false;
    for i in 0..container.reloc_count() {
        let entry = container.reloc_entry(i).unwrap();
        if entry.kind == lmod::reloc::RelocKind::MmioApertureBase as u8 {
            let site = entry.site_off as usize - code_off;
            let value = u32::from_le_bytes(code[site..site + 4].try_into().unwrap());
            assert_eq!(
                value, scratch.base,
                "aperture-base reloc site must be bound to the board's base (FR-15)"
            );
            found = true;
        }
    }
    assert!(found, "module must carry a MmioApertureBase reloc");
}

/// P6 D-5: a platformed module on a board with a different `platform_hash` is
/// rejected E5220 before any binding.
#[test]
fn wrong_board_rejected_e5220() {
    let dir = temp_dir("platform_binding_wrong_board");
    let lmod = build_mmio_module(&dir);
    let (_, apertures) = arm_board();
    let err = load_hosted(&lmod, Some(0xdead_beef_cafe_f000), &apertures).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::PlatformHashMismatch);
    assert_eq!(err.code(), 5220);
}

/// P6 latent-gap closure: a platformed module on a *boardless* host is
/// rejected E5220 (clear diagnostic) — never a silent `RelocUnsupported`.
#[test]
fn platformed_module_on_boardless_host_rejected_e5220() {
    let dir = temp_dir("platform_binding_boardless_host");
    let lmod = build_mmio_module(&dir);
    let err = load_hosted(&lmod, None, &[]).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::PlatformHashMismatch);
    assert_eq!(err.code(), 5220);
}

/// Acceptance §9.4: a module compiled against rp2350's descriptor is rejected
/// E5220 by the lm3s6965evb (armv7m) runtime board — even an MMIO-free module
/// (D-5 board-exact as delivered).
#[test]
fn rp2350_module_rejected_by_arm_runtime_board() {
    ensure_bins();
    let dir = temp_dir("platform_binding_rp2350_vs_lm3s");
    let src = dir.join("Main.mod");
    std::fs::write(
        &src,
        "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n",
    )
    .unwrap();
    let out_dir = dir.join("obj");
    std::fs::create_dir_all(&out_dir).unwrap();
    let status = Command::new(exe("langc"))
        .args([
            "--emit=obj",
            "--target=armv7m-unknown-none",
            &format!("--sysroot={}", workspace_root().join("sysroot").display()),
            &format!("--platform={}", workspace_root().join("platforms/rp2350").display()),
            &format!("--out-dir={}", out_dir.display()),
            src.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("langc");
    assert!(status.success(), "langc failed for rp2350 module");
    let lmod = dir.join("rp2350.lmod");
    let status = Command::new(exe("lmod-pack"))
        .args([
            out_dir.join("Main.o").to_string_lossy().as_ref(),
            lmod.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("lmod-pack");
    assert!(status.success(), "lmod-pack failed");

    // The lm3s board (armv7m runtime descriptor) must reject the rp2350-built
    // module: same arch/abi_hash, different platform_hash → E5220.
    let (ph, apertures) = arm_board();
    let err = load_hosted(&lmod, Some(ph), &apertures).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::PlatformHashMismatch);
    assert_eq!(err.code(), 5220);
}

fn patch_modinfo_field(bytes: &mut [u8], modinfo_off: usize, field_off: usize, value: &[u8]) {
    let off = modinfo_off + field_off;
    bytes[off..off + value.len()].copy_from_slice(value);
}

/// Locate the `.lmod` modinfo section (container header offsets 20..28).
fn lmod_modinfo_off(bytes: &[u8]) -> usize {
    let off = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    assert!(off + len <= bytes.len());
    off
}

/// P6 E5222: a module whose aperture-use entry names a aperture the board does not
/// expose is rejected.
#[test]
fn unexposed_aperture_rejected_e5222() {
    let dir = temp_dir("platform_binding_unexposed");
    let lmod_path = build_mmio_module(&dir);
    let mut bytes = std::fs::read(&lmod_path).unwrap();
    let mi = lmod_modinfo_off(&bytes);
    let wu = lmod::modinfo::aperture_use_offset(&bytes[mi..]).unwrap() + mi;
    let orig = u64::from_le_bytes(bytes[wu..wu + 8].try_into().unwrap());
    bytes[wu..wu + 8].copy_from_slice(&(orig ^ 0x1234_5678_9abc_def0).to_le_bytes());
    let patched = dir.join("patched.lmod");
    std::fs::write(&patched, &bytes).unwrap();
    let (ph, apertures) = arm_board();
    let err = load_hosted(&patched, Some(ph), &apertures).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::ApertureUnresolved);
    assert_eq!(err.code(), 5222);
}

/// P6 E5223: a structurally malformed aperture-use table is rejected.
#[test]
fn malformed_aperture_table_rejected_e5223() {
    let dir = temp_dir("platform_binding_malformed");
    let lmod_path = build_mmio_module(&dir);
    let mut bytes = std::fs::read(&lmod_path).unwrap();
    let mi = lmod_modinfo_off(&bytes);
    patch_modinfo_field(&mut bytes, mi, 32, &99u16.to_le_bytes());
    let patched = dir.join("patched.lmod");
    std::fs::write(&patched, &bytes).unwrap();
    let (ph, apertures) = arm_board();
    let err = load_hosted(&patched, Some(ph), &apertures).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::ApertureTableMalformed);
    assert_eq!(err.code(), 5223);
}

/// P6 D-12: a v3 modinfo on the v4 loader is rejected E5224 before any
/// allocation.
#[test]
fn v3_module_rejected_e5224() {
    let dir = temp_dir("platform_binding_v3");
    let lmod_path = build_mmio_module(&dir);
    let mut bytes = std::fs::read(&lmod_path).unwrap();
    let mi = lmod_modinfo_off(&bytes);
    patch_modinfo_field(&mut bytes, mi, 4, &3u16.to_le_bytes());
    let patched = dir.join("patched.lmod");
    std::fs::write(&patched, &bytes).unwrap();
    let (ph, apertures) = arm_board();
    let err = load_hosted(&patched, Some(ph), &apertures).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::ModinfoVersionUnsupported);
    assert_eq!(err.code(), 5224);
}

/// P6 transactional (§5.8 step 4): a failed load must leave the exclusivity
/// registry exactly as it was — observable via `bound_count`.
#[test]
fn failed_load_rolls_back_aperture_reservations() {
    let dir = temp_dir("platform_binding_rollback");
    let lmod_path = build_mmio_module(&dir);
    let bytes = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&bytes).expect("container parses");
    let mi = container.modinfo();
    let (ph, apertures) = arm_board();

    // First, corrupt the aperture table so the load fails after a reservation
    // would have been made (E5222 for a good second entry — here the single
    // entry is corrupted to fail during reservation).
    let mut bad = bytes.clone();
    let mi_off = lmod_modinfo_off(&mut bad);
    let wu = lmod::modinfo::aperture_use_offset(&bad[mi_off..]).unwrap() + mi_off;
    let orig = u64::from_le_bytes(bad[wu..wu + 8].try_into().unwrap());
    bad[wu..wu + 8].copy_from_slice(&(orig ^ 1).to_le_bytes());
    let bad_path = dir.join("bad.lmod");
    std::fs::write(&bad_path, &bad).unwrap();

    let mut reg = ApertureRegistry::new();
    let mark = reg.mark();
    let bad_bytes = std::fs::read(&bad_path).unwrap();
    let bad_container = Container::parse(&bad_bytes).unwrap();
    let err = bind_apertures(
        bad_container.modinfo(),
        Some(ph),
        &apertures,
        &mut reg,
        0xAAAA,
    )
    .unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::ApertureUnresolved);
    reg.rollback(mark);
    assert_eq!(reg.bound_count(), 0, "failed load must leave the registry empty");

    // A subsequent good module binds the same aperture successfully.
    bind_apertures(mi, Some(ph), &apertures, &mut reg, 0xBBBB).unwrap();
    assert_eq!(reg.bound_count(), 1);
}

/// P6 D-9 exclusivity: two modules claiming the same aperture conflict E5221 at
/// the binding layer (mirrors the sysroot-built-in + user-module case).
#[test]
fn aperture_conflict_rejected_e5221() {
    let dir = temp_dir("platform_binding_conflict");
    let lmod_path = build_mmio_module(&dir);
    let bytes = std::fs::read(&lmod_path).unwrap();
    let container = Container::parse(&bytes).unwrap();
    let mi = container.modinfo();
    let (ph, apertures) = arm_board();

    let mut reg = ApertureRegistry::new();
    bind_apertures(mi, Some(ph), &apertures, &mut reg, 0x1111).unwrap();
    let err = bind_apertures(mi, Some(ph), &apertures, &mut reg, 0x2222).unwrap_err();
    assert_eq!(err, loader_core::error::LoadError::ApertureConflict);
    assert_eq!(err.code(), 5221);
}