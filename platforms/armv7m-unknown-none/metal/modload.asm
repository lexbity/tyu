@ ---------------------------------------------------------------------------
@ Module modpack section — optional runtime unit for armv7m-unknown-none
@ (linked only when the `module-loading` feature is enabled).
@
@ Embedded .lmod images live in .modpack, each prefixed with a u32 length.
@ The loader scans [__lang_modpack_start .. __lang_modpack_end).
@ ---------------------------------------------------------------------------

.syntax unified
.thumb

.section .modpack, "aw", %nobits
.global __lang_modpack_start
.global __lang_modpack_end

__lang_modpack_start:
.space 0
__lang_modpack_end:
