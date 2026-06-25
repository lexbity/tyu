/* RISC-V RV32 (QEMU virt) linker script.
   DRAM at 0x80000000 (128 MB). */

ENTRY(__lang_start)

MEMORY
{
    DRAM (rwx) : ORIGIN = 0x80000000, LENGTH = 128M
}

/* Native stack top = start of DRAM + 64 KB */
__stack_top = ORIGIN(DRAM) + 64K;

SECTIONS
{
    .text : ALIGN(4)
    {
        /* Entry point first, at the start of DRAM (0x80000000): the `virt`
           machine resets straight here when booted with `-bios none`. */
        KEEP(*(.text.init))
        *(.text)
        *(.text.*)
        *(.rodata)
        *(.rodata.*)
    } > DRAM

    .data : ALIGN(4)
    {
        *(.data)
        *(.data.*)
    } > DRAM

    .modpack : ALIGN(4)
    {
        *(.modpack)
    } > DRAM

    .bss : ALIGN(4)
    {
        __bss_start = .;
        *(.bss)
        *(COMMON)
        __bss_end = .;
    } > DRAM
}
