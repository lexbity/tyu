ENTRY(__lang_start)

/* Explicit program headers so the linker emits a PT_NOTE segment for the
   PVH ELF note.  QEMU 10+ finds the 32-bit entry point via that note. */
PHDRS {
    note   PT_NOTE  FLAGS(4);       /* readable — not loaded at runtime */
    text   PT_LOAD  FLAGS(5);       /* readable + executable */
    rodata PT_LOAD  FLAGS(4);       /* readable */
    rwdata PT_LOAD  FLAGS(6);       /* readable + writable */
}

SECTIONS {
    /* PVH ELF note — placed first so it appears early in the file.
       Assigned to the PT_NOTE phdr; not mapped into virtual memory. */
    .note.Xen  0 : { *(.note.Xen) }  :note

    /* Load everything else starting at 1 MB (standard multiboot placement). */
    . = 0x100000;

    .text.boot  : { *(.text.boot) }  :text
    .text       : { *(.text) }       :text
    .rodata     : { *(.rodata) }     :rodata
    .lang.symtab : {
        PROVIDE(__lang_symtab_start = .);
        *(.lang.symtab)
        PROVIDE(__lang_symtab_end = .);
    } :rodata
    .lang.keys : {
        PROVIDE(__lang_keys_start = .);
        *(.lang.keys)
        PROVIDE(__lang_keys_end = .);
    } :rodata
    .data       : { *(.data) }       :rwdata
    .modpack    : {
        PROVIDE(__lang_modpack_start = .);
        *(.modpack)
        PROVIDE(__lang_modpack_end = .);
    } :rwdata
    .bss        : {
        *(.bss)
        *(COMMON)
    }                                :rwdata
    . = ALIGN(16);
    .loadheap (NOLOAD) : {
        PROVIDE(__lang_loadheap_start = .);
        . = . + 0x100000;
        PROVIDE(__lang_loadheap_end = .);
    }                                :rwdata
}
