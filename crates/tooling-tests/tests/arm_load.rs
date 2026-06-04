//! ARM dynamic loading tests.
//!
//! Constructs synthetic ARM Thumb `.lmod` containers and verifies that the
//! loader correctly parses, relocates, and rejects mismatched modules.

mod common;

use hosted::loader::HostedLoaderPlatform;
use lmod::header::{compute_layout, encode_header, LmodHeader};
use lmod::modinfo::{self, ExportEntry, ImportEntry};
use lmod::validate::Container;
use lmod::hash::fnv1a_u64;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;
use common::*;

/// ARM Thumb code containing `subs r4, r4, #4` — detectable by Arch scanner.
/// Encoding: 0001 1110 0100 0100 = 0x1E44, LE = [0x44, 0x1E].
/// The 00011 prefix at bits 15:11 is what the scanner looks for.
const ARM_CODE_WITH_SUBS_R4: &[u8] = &[
    0x44, 0x1e,              // subs r4, r4, #4
    0x70, 0x47,              // bx lr
];

/// ARM Thumb code with an import reference via absolute address.
/// Uses R_ARM_ABS32: a 4-byte placeholder at offset 0 that the loader
/// patches with the resolved symbol's address.  (THM_CALL's ±16 MB range
/// is too tight for cross-region host addresses on x86_64.)
const ARM_ABS32_IMPORT_CODE: &[u8] = &[
    0x00, 0x00, 0x00, 0x00,  // .word 0 (placeholder for R_ARM_ABS32)
    0x70, 0x47,              // bx lr
];

const ARM_ABI_HASH: u64 = 0x0445187d53048547; // compute_abi_hash(4, 32, 2)

fn make_arm_lmod(code: &[u8], imports: &[(&str, u8)]) -> Vec<u8> {
    let modinfo_bytes = make_arm_modinfo(imports);
    let modinfo_len = modinfo_bytes.len() as u32;
    let code_len = code.len() as u32;
    let reloc_count = imports.len() as u32;

    let layout = compute_layout(
        ARM_ABI_HASH, modinfo_len, code_len, 0, 0, 0, reloc_count,
    );
    let total = layout.total_len as usize;
    let mut out = vec![0u8; total];

    encode_header(&mut out, &layout);

    let mo = layout.modinfo_off as usize;
    out[mo..mo + modinfo_bytes.len()].copy_from_slice(&modinfo_bytes);

    let co = layout.code_off as usize;
    out[co..co + code.len()].copy_from_slice(code);

    // Write reloc entries (each 16 bytes: site_off(4) + sym_hash(8) + kind(1) + pad(3))
    let ro = layout.reloc_off as usize;
    for (i, &(name, kind)) in imports.iter().enumerate() {
        let entry_off = ro + i * 16;
        let hash = fnv1a_u64(name.as_bytes());
        // Reloc site: within code section
        out[entry_off..entry_off + 4].copy_from_slice(&layout.code_off.to_le_bytes());
        out[entry_off + 4..entry_off + 12].copy_from_slice(&hash.to_le_bytes());
        out[entry_off + 12] = kind;
    }

    out
}

fn make_arm_modinfo(imports: &[(&str, u8)]) -> Vec<u8> {
    let export_entries: [ExportEntry; 0] = [];
    let import_entries: Vec<ImportEntry> = imports.iter().map(|(name, _)| {
        ImportEntry { sym_hash: fnv1a_u64(name.as_bytes()), name: name.as_bytes() }
    }).collect();

    let mut buf = [0u8; 1024];
    let size = modinfo::encode_into(
        &mut buf, b"TestModule",
        &export_entries, &import_entries,
        ARM_ABI_HASH, 0, &[],
    ).unwrap();
    buf[..size].to_vec()
}

fn make_arm_platform() -> HostedLoaderPlatform {
    HostedLoaderPlatform::new(ARM_ABI_HASH)
}

/// Test that an ARM `.lmod` with a simple (no-import) function can be loaded.
#[test]
fn arm_load_no_imports() {
    let raw = make_arm_lmod(ARM_CODE_WITH_SUBS_R4, &[]);
    let container = Container::parse(&raw).unwrap();
    let mut plat = make_arm_platform();
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert!(result.is_ok(), "ARM no-import load failed: {:?}", result);
}

/// Test that an ARM `.lmod` with an import is loaded and the relocation applied.
#[test]
fn arm_load_with_import() {
    let raw = make_arm_lmod(ARM_ABS32_IMPORT_CODE, &[("__stack_overflow", 4)]);
    let container = Container::parse(&raw).unwrap();
    let mut plat = make_arm_platform();
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let loaded = load_module(&container, &mut plat, &mut global_map, &mut loaded_set)
        .expect("ARM import load with ABS32 should succeed");

    // Verify that the relocation was applied: the first 4 bytes of the loaded
    // code section should now contain the lower 32 bits of the symbol address.
    // ARM is a 32-bit architecture, so ABS32 writes a 32-bit absolute address.
    let code_slice = unsafe { loaded.code.as_slice() };
    let patched = u32::from_le_bytes(code_slice[..4].try_into().unwrap()) as u64;

    let stub = unsafe { std::mem::transmute::<extern "C" fn(), u64>(common::extern_c_fn_stub) };
    assert_eq!(patched, stub & 0xFFFF_FFFF,
        "ABS32 relocation should write the lower 32 bits of the symbol address");
}

/// Test that an abi_hash mismatch is correctly rejected.
#[test]
fn arm_load_abi_hash_mismatch_rejected() {
    let mut raw = make_arm_lmod(ARM_CODE_WITH_SUBS_R4, &[]);
    // Corrupt abi_hash in the header (offset 8 in the .lmod header)
    raw[8..16].copy_from_slice(&[0u8; 8]);
    let container = Container::parse(&raw).unwrap();
    let mut plat = make_arm_platform(); // still expects ARM_ABI_HASH
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert_eq!(result.unwrap_err(), 5200, "expected E_ABI_MISMATCH");
}

/// Test that ARM architecture detection works via code bytes.
#[test]
fn arm_arch_detected_from_thumb_code() {
    let arch = loader_core::rederive::Arch::detect_from_code(ARM_CODE_WITH_SUBS_R4);
    assert_eq!(arch, loader_core::rederive::Arch::ArmThumb,
        "should detect ARM Thumb from subs r4 pattern");
}

/// Test that ARM rederive works for the code with a DS push.
#[test]
fn arm_rederive_subs_r4() {
    let high = loader_core::rederive::rederive_stack_high(
        ARM_CODE_WITH_SUBS_R4,
        loader_core::rederive::Arch::ArmThumb,
        4,
    );
    // subs r4, r4, #4 → 1 slot of DS usage
    assert_eq!(high, 1, "subs r4, #4 = 1 slot");
}
