ENTRY(__lang_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x10000000, LENGTH = 0x00400000
    SRAM  (rwx): ORIGIN = 0x20000000, LENGTH = 0x00084000
}

SECTIONS
{
    .text : ALIGN(4)
    {
        *(.text*)
    } > FLASH

    .rodata : ALIGN(4)
    {
        *(.rodata*)
    } > FLASH

    .data : ALIGN(4)
    {
        *(.data*)
    } > SRAM AT > FLASH

    .bss (NOLOAD) : ALIGN(4)
    {
        *(.bss*)
        *(COMMON)
    } > SRAM

    __stack_top = ORIGIN(SRAM) + LENGTH(SRAM);
}
