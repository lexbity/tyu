# RP2040 Memory Map and Linker Script (Draft)

This document defines the RP2040-specific memory map and a minimal linker
script. It also lists differences from the generic Cortex-M assumptions.

---

## 1) RP2040 memory map (baseline)

Flash (XIP):
- 0x10000000 .. 0x10200000 (2 MB)

SRAM (total 264 KB):
- 0x20000000 .. 0x20042000 (264 KB)

Note: RP2040 physically has multiple SRAM banks. For v1, treat as one
contiguous region unless bank placement is required.

---

## 2) Minimal linker script (RP2040)

This script supports a single static image with optional module pack.

```
MEMORY
{
  FLASH (rx) : ORIGIN = 0x10000000, LENGTH = 2M
  RAM   (rwx): ORIGIN = 0x20000000, LENGTH = 264K
}

ENTRY(__lang_start)

SECTIONS
{
  .text :
  {
    KEEP(*(.vectors*))
    *(.text*)
    *(.rodata*)
  } > FLASH

  .data : AT(ADDR(.text) + SIZEOF(.text))
  {
    __data_start = .;
    *(.data*)
    __data_end = .;
  } > RAM

  .bss (NOLOAD) :
  {
    __bss_start = .;
    *(.bss*)
    *(COMMON)
    __bss_end = .;
  } > RAM

  .stack (NOLOAD) :
  {
    __lang_ds_base = .;
    . = . + 64K;
    __lang_ds_limit = .;
  } > RAM

  .modpack :
  {
    __lang_modpack_start = .;
    *(.modpack*)
    __lang_modpack_end = .;
  } > FLASH
}
```

Notes:
- .vectors is optional if you do not use a vector table in v1.
- .data is stored in FLASH and copied to RAM at startup.
- .stack size is a placeholder; choose per target profile.

---

## 3) Differences vs generic Cortex-M

Generic Cortex-M assumptions in the dynamic loading draft:
- single contiguous SRAM region
- single contiguous FLASH region
- optional vector table at image start

RP2040-specific differences:
- dual-core; v1 should assume single-core unless you enable SMP.
- SRAM is banked; placement can matter for DMA/perf, but v1 can ignore.
- XIP flash requires boot ROM setup; v1 can rely on default QEMU/XIP boot.
- No built-in semihosting in hardware; QEMU uses semihosting for tests.

---

## 4) Recommended v1 policy

- Use single-core mode.
- Treat SRAM as a contiguous region.
- Use XIP for .text/.rodata in static images.
- Use RAM for dynamically loaded module .text (simplest loader).

