//! RISC-V dynamic loading tests.
//!
//! Constructs synthetic RV32 `.lmod` containers and verifies that the
//! loader correctly parses, relocates, and rejects mismatched modules.

mod common;

use hosted::loader::HostedLoaderPlatform;
use lmod::header::{compute_layout, encode_header};
use lmod::modinfo::{self, ExportEntry, ImportEntry};
use lmod::validate::Container;
use lmod::hash::fnv1a_u64;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;
use common::*;

/// RISC-V RV32 code with `addi s2, s2, -8` — detectable by Arch scanner.
/// Encoding: 0xFF890913 = addi s2, s2, -8, LE = [0x13, 0x09, 0x89, 0xFF]
/// ret: jalr x0, ra, 0 = 0x00008067, LE = [0x67, 0x80, 0x00, 0x00]
const RV32_CODE_WITH_DS_POP: &[u8] = &[
    0x13, 0x09, 0x89, 0xff,  // addi s2, s2, -8
    0x67, 0x80, 0x00, 0x00,  // ret
];

/// RV32 code with a 4-byte placeholder for R_RISCV_32 (kind 8) import.
const RV32_ABS32_IMPORT_CODE: &[u8] = &[
    0x00, 0x00, 0x00, 0x00,  // .word 0 (placeholder for R_RISCV_32)
    0x67, 0x80, 0x00, 0x00,  // ret
];

const RV32_ABI_HASH: u64 = 0x0445187d53048547; // compute_abi_hash(4, 32, 2)

fn make_rv32_lmod(code: &[u8], imports: &[(&str, u8)]) -> Vec<u8> {
    let import_entries: Vec<ImportEntry> = imports.iter().map(|(name, _)| {
        ImportEntry { sym_hash: fnv1a_u64(name.as_bytes()), name: name.as_bytes() }
    }).collect();
    let mut mbuf = [0u8; 1024];
    let msize = modinfo::encode_into(&mut mbuf, b"TestModule",
        &[], &import_entries, RV32_ABI_HASH, 0, &[]).unwrap();

    let layout = compute_layout(RV32_ABI_HASH, msize as u32, code.len() as u32, 0, 0, 0, imports.len() as u32);
    let mut out = vec![0u8; layout.total_len as usize];
    encode_header(&mut out, &layout);

    let mo = layout.modinfo_off as usize;
    out[mo..mo + msize].copy_from_slice(&mbuf[..msize]);
    let co = layout.code_off as usize;
    out[co..co + code.len()].copy_from_slice(code);

    let ro = layout.reloc_off as usize;
    for (i, &(name, kind)) in imports.iter().enumerate() {
        let entry_off = ro + i * 16;
        let hash = fnv1a_u64(name.as_bytes());
        out[entry_off..entry_off + 4].copy_from_slice(&layout.code_off.to_le_bytes());
        out[entry_off + 4..entry_off + 12].copy_from_slice(&hash.to_le_bytes());
        out[entry_off + 12] = kind;
    }
    out
}

#[test]
fn riscv_load_no_imports() {
    let raw = make_rv32_lmod(RV32_CODE_WITH_DS_POP, &[]);
    let container = Container::parse(&raw).unwrap();
    let mut plat = HostedLoaderPlatform::new(RV32_ABI_HASH);
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert!(result.is_ok(), "RV32 no-import load failed: {:?}", result);
}

#[test]
fn riscv_load_with_import() {
    let raw = make_rv32_lmod(RV32_ABS32_IMPORT_CODE, &[("__stack_overflow", 8)]);
    let container = Container::parse(&raw).unwrap();
    let mut plat = HostedLoaderPlatform::new(RV32_ABI_HASH);
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let loaded = load_module(&container, &mut plat, &mut global_map, &mut loaded_set)
        .expect("RV32 import load with R_RISCV_32 should succeed");

    // Verify relocation: first 4 bytes = lower 32 bits of symbol address
    let code_slice = unsafe { loaded.code.as_slice() };
    let patched = u32::from_le_bytes(code_slice[..4].try_into().unwrap()) as u64;
    let stub = unsafe { std::mem::transmute::<extern "C" fn(), u64>(common::extern_c_fn_stub) };
    assert_eq!(patched, stub & 0xFFFF_FFFF,
        "R_RISCV_32 relocation should write lower 32 bits of symbol address");
}

#[test]
fn riscv_load_abi_hash_mismatch_rejected() {
    let mut raw = make_rv32_lmod(RV32_CODE_WITH_DS_POP, &[]);
    raw[8..16].copy_from_slice(&[0u8; 8]); // corrupt abi_hash
    let container = Container::parse(&raw).unwrap();
    let mut plat = HostedLoaderPlatform::new(RV32_ABI_HASH);
    plat.reserve(4096).unwrap();

    let ds_high = allocate_runtime_page();
    let mut global_map: SymMap<'_, 256> = SymMap::new();
    register_runtime_symbols(&mut global_map, ds_high);
    let mut loaded_set = LoadedSet::<64>::new();

    let result = load_module(&container, &mut plat, &mut global_map, &mut loaded_set);
    assert_eq!(result.unwrap_err(), 5200, "expected E_ABI_MISMATCH");
}

#[test]
fn riscv_arch_detected_from_code() {
    let arch = loader_core::rederive::Arch::detect_from_code(RV32_CODE_WITH_DS_POP);
    assert_eq!(arch, loader_core::rederive::Arch::RiscV,
        "should detect RISC-V from addi s2 pattern");
}

#[test]
fn riscv_rederive_ds_pop() {
    let high = loader_core::rederive::rederive_stack_high(
        RV32_CODE_WITH_DS_POP,
        loader_core::rederive::Arch::RiscV,
        4,
    );
    // addi s2, s2, -8 = 2 slots of DS usage
    assert_eq!(high, 2, "addi s2, -8 = 2 slots (i64)");
}
