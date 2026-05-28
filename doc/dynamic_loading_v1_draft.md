# Dynamic Loading v1 Draft (ELF-based)

This draft defines a minimal dynamic loading model that works for:
- Cortex-M bare metal (QEMU)
- x86_64 hosted on Linux

The compiler emits relocatable ELF modules. The runtime loader resolves
imports, applies relocations, and runs module init.

The static image is still valid and considered the trivial case. The
dynamic model is a superset.

---

## 1) Artifacts

### 1.1 Base runtime image
Type: ET_EXEC (or ET_DYN if you want PIC runtime)

Contents:
- runtime + loader + sysroot core
- optional module pack (embedded modules for tests)

Required symbols:
- __lang_start
- __lang_trap (and optional __lang_trap_loc)
- __lang_ds_base
- __lang_ds_limit
- __lang_modpack_start (optional)
- __lang_modpack_end (optional)

### 1.2 Module image
Type: ET_REL (relocatable) preferred for v1.

Sections:
- .text, .rodata, .data, .bss
- .symtab, .strtab
- .rel.* or .rela.* (relocations)
- .lang.modinfo (custom metadata, required)

Each module is a single language compilation unit:
  module Name; ... end;

---

## 2) Module metadata (.lang.modinfo)

Binary blob, little endian, placed in a dedicated section.

Layout (fixed v1):
struct LangModInfo {
  u32 magic;        // 0x4c4d4f44 "LMOD"
  u16 version;      // 1
  u16 flags;        // 0 for now
  u32 name_len;     // bytes
  u32 export_count;
  u32 import_count;
  u32 abi_hash;     // build-ABI guard (see below)
  // followed by:
  // name bytes (not null-terminated)
  // export symbol hashes (u32 * export_count)
  // import symbol hashes (u32 * import_count)
};

Symbol hash is the same hash used by codegen symbol names (see below).

abi_hash is a stable hash of:
- compiler version that affects ABI
- target profile
- sysroot version
- runtime ABI version

---

## 3) Symbol naming

Each word symbol is exported as:
  w_<hash>

hash is fnv1a_u32 over the UTF-8 bytes of the word name.

Example:
  : main ( -- i64 ) ... ;
  symbol => w_6d61696e

The loader uses the hashes from .lang.modinfo to resolve imports.

---

## 4) Loader responsibilities (all targets)

1) Acquire module bytes:
   - semihosting file
   - embedded .modpack
   - host FS (x86_64)
2) Parse ELF headers and section table.
3) Locate .text/.rodata/.data/.bss, .symtab/.strtab, .rel/.rela, .lang.modinfo.
4) Allocate/load:
   - .text, .rodata (RX/RO)
   - .data, .bss (RW)
5) Resolve imports using .lang.modinfo import list.
6) Apply relocations (target-specific subset below).
7) Optional: call __lang_mod_init if present.

---

## 5) Relocation subset (v1)

Keep the subset small and explicit. Reject anything else.

### 5.1 Cortex-M (ARM Thumb)
Recommended subset:
- R_ARM_ABS32
- R_ARM_THM_CALL
- R_ARM_THM_JUMP24
- R_ARM_REL32 (only if PC-relative data is used)

Notes:
- Function pointers must have LSB=1 (Thumb bit).
- The loader must set the bit when resolving function symbols.

### 5.2 x86_64 (hosted)
Recommended subset:
- R_X86_64_64
- R_X86_64_PC32
- R_X86_64_PLT32

Notes:
- W^X: apply relocations while pages are RW, then flip to RX.

---

## 6) Memory layout for Cortex-M (RP2040-like)

Assumed memory map:
- FLASH: 0x10000000..0x10200000 (2MB)
- SRAM : 0x20000000..0x20042000 (264KB)

Runtime alloc policy:
- .text/.rodata default to FLASH (XIP) for base image.
- Modules can be:
  - XIP from FLASH if stored in .modpack
  - Copied to RAM if loaded via semihosting

Data stack:
- __lang_ds_base and __lang_ds_limit in SRAM.

Magic test location:
- __lang_test_status at a fixed SRAM address (e.g., 0x20000000).
- 0 = running, 1 = pass, other = fail code.
- QEMU runner polls this address.

---

## 6.1 Cortex-M loader ABI (v1)

This section defines the loader-facing ABI used by dynamic loading on Cortex-M.

Required symbols (exported by runtime):
- __lang_load_module(ptr,len) -> i32
  - Returns 0 on success, nonzero error code on failure.
- __lang_resolve_symbol(hash:u32) -> ptr
  - Returns resolved address or 0 if not found.

Module init:
- Optional symbol: __lang_mod_init
  - If present, called after relocations are applied.

Thumb address rule:
- All function pointers must have bit0 set to 1 (Thumb state).

Test status:
- __lang_test_status is a fixed address in SRAM.
- Loader should not overwrite this location.

---

## 7) Module acquisition

### 7.1 Semihosting (default for tests)
The loader opens module files by name:
  <module>.lmod

File contains an ET_REL ELF module with .lang.modinfo.

### 7.2 Embedded modpack
The base image may embed module blobs in a section:
  .modpack

The loader scans __lang_modpack_start..__lang_modpack_end, each blob
preceded by a u32 length.

---

## 8) Static linking (trivial case)

Static mode links runtime + module(s) into a single ET_EXEC.
No loader is required. Symbols resolve at link time.

Dynamic mode is a strict superset: same ABI and symbol hashes.

---

## 9) x86_64 hosted loader

Implement the same loader logic, but use mmap:
- map text as RX
- map rodata as RO
- map data/bss as RW
- then apply relocations, then enforce W^X.

Module files can be read from the host FS.
The loader can coexist with the existing static runtime.

---

## 10) Open items

- Precise abi_hash definition and versioning policy.
- Whether module init is mandatory or optional.
- If position-independent modules are required (ET_DYN) or optional.
- Static-first targets (ESP32): see doc/esp32_memory_map_linker.md for
  v1 static layout and test signaling.
