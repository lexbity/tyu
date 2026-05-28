# ESP32 (ESP32-WROOM-32) Memory Map and Linker Script (Draft)

This document defines the ESP32 classic (ESP32-WROOM-32) memory map and a
minimal static linker script for v1 bring-up under QEMU. It also lists
differences from generic Cortex-M assumptions.

Source notes:
- Module: ESP32-WROOM-32 datasheet (4 MB external flash).
- SoC: ESP32 datasheet (memory map).

---

## 1) ESP32 classic memory map (baseline)

External SPI flash (mapped):
- DROM (data, read-only): 0x3F40_0000 .. 0x3F7F_FFFF
- IROM (code, execute):  0x400C_2000 .. 0x40BF_FFFF

Internal SRAM (IRAM/DRAM):
- IRAM (instr-capable):  0x4007_0000 .. 0x400B_FFFF
- DRAM (data):           0x3FFA_E000 .. 0x3FFD_FFFF
- DRAM (data):           0x3FFE_0000 .. 0x3FFF_FFFF

RTC memory (optional, v1 ignore):
- RTC FAST: 0x3FF8_0000 .. 0x3FF8_1FFF
- RTC SLOW: 0x5000_0000 .. 0x5000_1FFF

ROM:
- ROM0: 0x4000_0000 .. 0x4005_FFFF
- ROM1: 0x3FF9_0000 .. 0x3FF9_FFFF

---

## 2) Minimal static linker script (ESP32 classic)

This script targets a single static ELF image for QEMU testing. It places
code in IROM, rodata in DROM, and data/stack in DRAM.

```
MEMORY
{
  IROM (rx)  : ORIGIN = 0x400C2000, LENGTH = 0x033E0000
  DROM (r)   : ORIGIN = 0x3F400000, LENGTH = 0x00400000
  IRAM (rx)  : ORIGIN = 0x40070000, LENGTH = 0x00050000
  DRAM0 (rwx): ORIGIN = 0x3FFAE000, LENGTH = 0x00032000
  DRAM1 (rwx): ORIGIN = 0x3FFE0000, LENGTH = 0x00020000
}

ENTRY(__lang_start)

SECTIONS
{
  .text :
  {
    *(.text*)
  } > IROM

  .rodata :
  {
    *(.rodata*)
  } > DROM

  .data : AT(ADDR(.rodata) + SIZEOF(.rodata))
  {
    __data_start = .;
    *(.data*)
    __data_end = .;
  } > DRAM0

  .bss (NOLOAD) :
  {
    __bss_start = .;
    *(.bss*)
    *(COMMON)
    __bss_end = .;
  } > DRAM0

  .stack (NOLOAD) :
  {
    __lang_ds_base = .;
    . = . + 32K;
    __lang_ds_limit = .;
  } > DRAM1

  .test_status (NOLOAD) :
  {
    __lang_test_status = .;
    LONG(0);
  } > DRAM1
}
```

Notes:
- The IROM/DROM lengths above are placeholders; adjust based on QEMU and
  flash size limits (ESP32-WROOM-32 has 4 MB flash).
- You may choose IRAM for .text if QEMU or the loader cannot XIP from IROM.
- The test status word lives in DRAM1 and is polled by QEMU:
  - 0 = running, 1 = pass, other = fail code.

---

## 3) Differences vs Cortex-M targets

- Xtensa ISA (not ARM Thumb).
- Separate IROM/DROM memory-mapped flash regions.
- No vector table model like Cortex-M; boot ROM hands off to entry.
- Dual-core CPU exists, but v1 assumes single-core.

---

## 4) Recommended v1 policy

- Use static images (ET_EXEC) for QEMU tests.
- Place code in IROM and rodata in DROM if QEMU supports XIP.
- Use DRAM for data, bss, and the data stack.
- Use a single magic test status word in DRAM1.

