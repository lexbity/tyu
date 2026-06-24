# ---------------------------------------------------------------------------
# Module modpack section — optional runtime unit for riscv32-unknown-none
# (linked only when the `module-loading` feature is enabled).
#
# Embedded .lmod images live in .modpack, each prefixed with a u32 length.
# The loader scans [__lang_modpack_start .. __lang_modpack_end).
# ---------------------------------------------------------------------------

.section .modpack
.globl __lang_modpack_start
__lang_modpack_start:
.globl __lang_modpack_end
__lang_modpack_end:
