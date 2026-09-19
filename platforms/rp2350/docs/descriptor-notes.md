# RP2350 pack — descriptor and metal rationale (P8, post-audit)

This pack is the first real board pack. Everything in `platform.toml` is a
claim about hardware, and the toolchain trusts it — the first draft of this
pack demonstrated exactly why: it passed every structural gate while carrying
RP2040-era addresses, fabricated registers, and an SRAM size that put the boot
stack pointer past physical SRAM. This document records, for every modeling
decision, where the fact comes from and why anything excluded was excluded.

The 2026-09 audit (see the P8 addendum in `devdocs/plans/platform-layer.md`)
found the draft's tables were RP2040-derived or invented. The tables were
rebuilt from the datasheet and are now pinned row-by-row by
`crates/tyu/tests/rp2350_datasheet_facts.rs` (docs-as-tests), so this pack
cannot regress silently.

## Sources

- `devdocs/HW-datasheets/RP-008373-DS-2-rp2350-datasheet.pdf` ("DS2"):
  - address map: DS2 §2.2, Tables 13 (APB) and 14 (AHB)
  - system IRQ numbering: DS2 §3.2, Table 95
  - register lists: each peripheral's "List of registers" section
  - register access types: the per-register field tables' Type column
    (`RW`/`RO`/`WC`/`WF`), mapped to descriptor semantics as
    WC → `rw` + `write_kind = "w1c"`, WF → `wo`, RO → `ro`, RW → `rw`
  - boot image format: DS2 §5.9 (PICOBIN blocks), §5.9.5 (minimum viable image)
  - clock/tick bring-up: DS2 §8.1–8.6 (CLOCKS/PLL), §8.2 (XOSC), §8.5 (ticks)

## What is modeled

14 devices: IO_BANK0 (`GPIO/gpio0`), PADS_BANK0 (`PADS/pads_bank0`), UART0/1,
SPI0/1, I2C0/1, TIMER0/1, QMI (direct-mode surface), PIO0/1, and SIO. Every
(offset, name) row is datasheet-pinned by the facts test; IRQ columns use
Table 95 numbers (GPIO 21, UART0/1 33/34, SPI0/1 31/32, I2C0/1 36/37, PIO0
15/16, PIO1 17/18, timer alarms 0–3 / 4–7).

Notable semantic points, each of which the draft got wrong:

- IO_BANK0 puts STATUS at +8n and CTRL at +8n+4; INTR0–5 (8 pins each) are
  write-1-to-clear raw status (Type WC); enabling a GPIO interrupt for core 0
  uses PROC0_INTE0–5.
- TIMER0/1's write-1-to-clear interrupt register is `INTR` at 0x3c (there are
  no `TIMERALARM*` registers); `ARMED` at 0x20 is WC (write-1 disarms);
  `TIMERAWH`/`TIMERAWL` are at 0x24/0x28.
- SIO uses the RP2350 layout with the GPIO_HI bank interleaved
  (OUT_SET 0x18, OUT_CLR 0x20, OUT_XOR 0x28, OE 0x30, OE_SET 0x38,
  OE_CLR 0x40). The XOR aliases are `write_kind = "xor"` — modeled as w1c
  (as the draft did) would lower an RMW `bic`, which on the XOR alias
  computes `X ^ (X & ~v) = X & v`, silently the wrong value. The `xor`
  write-kind was added to the compiler registry (D-2 process: registry entry,
  classification row per backend, strategy-matrix golden, QEMU fixtures).
- PIO1 lives at 0x50300000, i.e. base_offset 0x100000 in the `pio` aperture
  (the draft's 0x10000 pointed at reserved space inside PIO0's slot).
- Spinlock reads acquire the lock (`read_kind = "effectful"`), so the R1
  phantom-read rules apply to them.

## What is deliberately not modeled (and why)

- IO_BANK0 IRQSUMMARY_*, PROC1_*, DORMANT_WAKE_*: this runtime is single-core
  and never enters dormant modes; the rows are unreachable from any word the
  pack can compile.
- UART/SPI identification registers (PERIPHID/PCELLID): read-only constants.
- QMI `M0_*`/`M1_*`/`ATRANS*`: rewriting flash timing or XIP address
  translation from a running XIP image unmaps the executing code. The
  bootrom owns that configuration; only the direct-mode surface
  (DIRECT_CSR/TX/RX) is exposed.
- SIO INTERP* and TMDS*: application accelerators with no capability words;
  add rows together with a capability that drives them.
- DW_apb_i2c high-speed mode registers: no high-speed capability.

## Memory map and boot

- SRAM is 520 kB (0x20000000..0x20082000; DS2 Tables 11–12). The draft's
  0x84000 made `__stack_top = ORIGIN + LENGTH` point 8 KiB past physical
  SRAM — and because a linker-script symbol silently overrides the asm label
  of the same name, that value became the boot SP in the vector table. The
  first push after reset bus-faulted. Fixed to 0x82000.
- The native stack lives at the top of SRAM (`__stack_top` from the pack's
  `metal/<isa>/link.ld`); `__lang_stack_limit` sits below the DS and a 1 KiB
  guard in BSS. Both forks zero `.bss` at start: real silicon does not
  guarantee SRAM content across reset, and the DS high-water marks and the
  region allocator rely on zeroed statics.
- Boot is a PICOBIN block (DS2 §5.9) emitted by `tyu build` when
  `boot = "image_def"`: block marker, IMAGE_DEF item (EXE | Secure | Arm or
  EXE | RISC-V | RP2350), ENTRY_POINT item (initial PC/SP/SP-limit), 2BS_LAST,
  footer. The pack's `link.ld` places the block right behind the vector table
  (Arm) or at image start (RISC-V), inside the bootrom's first-4 kB scan
  window. The pack's `link.ld` is the single layout authority — the build no
  longer renders a parallel script for `boot = "image_def"` packs.
- The ARM fork installs a full 68-entry vector table (16 system exceptions +
  IRQ0–51, all routed to the trap dump) and points VTOR at it; the RISC-V
  fork points `mtvec` at `__lang_trap` (zicsr enabled locally, since the
  toolchain assembles `-march=rv32im`).
- Bring-up: XOSC (12 MHz) → resets released (UART0, TIMER0, PLL_SYS,
  PADS_BANK0, IO_BANK0) → PLL_SYS 12 MHz ×125 /5 /2 = 150 MHz → `clk_sys`
  switched to the PLL (glitchless mux polled), `clk_peri` enabled from
  `clk_sys` (UART baud divisors 81/24 = 115200), `clk_ref` switched to XOSC,
  TIMER0 tick generator set to 12 cycles = 1 µs. UART diagnostics are garbled
  until this runs; the draft emitted bootrom-clock baud constants with no
  clock setup at all.

## Glue and metal.trust

Per D-10 every asm-backed word is enumerated in `[platform.metal.trust]`;
nothing binds by silent symbol convention. The capability words
(gpio/uart/time/testio/mem region) are implemented in the metal forks for two
recorded reasons:

1. GPIO (and PADS) addressing is *parameterized* (`base + pin*8`); the source
   language's register-map model names rows, so pin-indexed access cannot be
   expressed in Tyu today. Retiring the asm needs an indexed-MMIO language
   feature first.
2. The build does not yet compile pack glue modules into images (the P7
   addendum records the same state for the reference packs); glue `.mod`
   files are the declaration surface, `.def` is the conformance surface, and
   the implementations live in metal.

Both are gaps to close in a later slice, not permanent architecture. The
region allocator is the P7 reference bump allocator ported verbatim from the
generic runtimes and enumerated like theirs.

## Concurrency and module loading

- Concurrency is declared (`[features.concurrency]`). The RISC-V fork's task
  arenas are sized for this board: 64 tasks × (2 KiB data stack + 1 KiB call
  stack) = 192 KiB of the 520 kB SRAM, instead of the QEMU-virt 64 KiB
  per-task slices. The ARM fork keeps the lm3s sizing (2 KiB slices), which
  fits a fortiori.
- Module loading is deliberately **not** declared. The on-device dynamic
  loader (the `loader-core` staticlib plus its load arena) does not fit in
  520 kB of SRAM — the QEMU runtimes only fit it because they link into
  128 MB of DRAM. `tyu build --mode=dynamic` therefore fails fast on the
  missing `dynamic_entry.asm` unit instead of linking an image that cannot
  load.

## Board claims

The `[test]` rung is `hardware` because the only meaningful evidence for this
pack is HIL. As of this writing the HIL transcript has **not** been recorded;
`docs/hil/rp2350.md` is the evidence file and says so. The QEMU execution
suites run against the runtime-class descriptors (D-14), not against this
pack — there is no RP2350 QEMU model.
