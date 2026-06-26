/* ARM Cortex-M3 (lm3s6965evb) linker script.
   FLASH at 0x0 (256 KB), SRAM at 0x20000000 (64 KB). */

ENTRY(__lang_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 256K
    SRAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
}

/* Native stack top = top of SRAM */
__stack_top = ORIGIN(SRAM) + LENGTH(SRAM);

SECTIONS
{
    /* Vector table — must be at FLASH origin for Cortex-M.
       Content provided by runtime.asm .vectors section. */
    .vectors : ALIGN(4)
    {
        *(.vectors)
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
        . = ALIGN(4);
        *(.ARM.exidx*)
        *(.gnu.linkonce.armexidx.*)
    } > FLASH

    PROVIDE(__aeabi_unwind_cpp_pr0 = 0);

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
