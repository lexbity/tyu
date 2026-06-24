; ---------------------------------------------------------------------------
; Module modpack section — optional runtime unit (linked only when the
; `module-loading` feature is enabled).
;
; Embedded .lmod images live in their own section, each prefixed with a
; u32 length.  The loader scans [__lang_modpack_start .. __lang_modpack_end).
;
; This unit is excluded from the link when the `module-loading` feature is
; disabled in the build profile, saving the .modpack section size (even
; when empty, the section markers cost alignment padding).
; ---------------------------------------------------------------------------

format ELF64

section '.modpack' writeable

public __lang_modpack_start
__lang_modpack_start:

public __lang_modpack_end
__lang_modpack_end:
