/* RP2350 (Hazard3 RV32IM) linker script — board pack fork (P8).
   XIP FLASH at 0x10000000 (4 MB), SRAM at 0x20000000 (520 kB: banks 0-7
   striped + non-striped banks 8-9, DS2 Tables 11-12). */

ENTRY(__lang_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x10000000, LENGTH = 0x00400000
    SRAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 0x00082000
}

/* Native stack top = top of SRAM (above the loader arena). The image_def
   block's ENTRY_POINT item carries the initial PC/SP (DS2 5.9.3.4). */
__stack_top = ORIGIN(SRAM) + LENGTH(SRAM);

SECTIONS
{
    /* PICOBIN image-definition block (emitted by `tyu build`). The bootrom
       scans the first 4 kB of flash for it (DS2 5.9.5), so it leads the
       image; entry comes from the block's ENTRY_POINT item. */
    .image_def : ALIGN(4)
    {
        KEEP(*(.image_def))
    } > FLASH

    .text : ALIGN(4)
    {
        *(.text)
        *(.text.*)
        *(.rodata)
        *(.rodata.*)
        . = ALIGN(8);
        __lang_symtab_start = .;
        *(.lang.symtab)
        __lang_symtab_end = .;
        . = ALIGN(8);
        __lang_keys_start = .;
        *(.lang.keys)
        __lang_keys_end = .;
    } > FLASH

    .data : ALIGN(4)
    {
        *(.data)
        *(.data.*)
    } > SRAM AT > FLASH

    .modpack : ALIGN(4)
    {
        __lang_modpack_start = .;
        *(.modpack)
        __lang_modpack_end = .;
    } > FLASH

    .bss : ALIGN(4)
    {
        __bss_start = .;
        *(.bss)
        *(COMMON)
        __bss_end = .;
    } > SRAM

    .loadheap (NOLOAD) : ALIGN(16)
    {
        __lang_loadheap_start = .;
        . = . + 2K;
        __lang_loadheap_end = .;
    } > SRAM
}
