//! RISC-V aperture-base reloc site (P6).
//!
//! Site pattern: `auipc rN, hi20` + `lw rN, lo12(rN)` over a literal-pool
//! word. The reloc site is that word: a `R_RISCV_32` relocation against the
//! extern `__lang_aperture_N_base`, loaded pc-relatively (`R_RISCV_LO12_I`)
//! so it survives module relocation into RAM. At pack time the word is bound
//! to the aperture base; at load time the loader re-derives the same base from
//! its descriptor (`check_aperture_base`).
//!
//! `apply_base`/`read_site_base` from the shared `mod` write/read that word
//! unchanged — the hi20/lo12 pair addresses the literal; the literal itself
//! holds the *address*, not an addend.

/// The shared apply/read semantics are exactly the 32-bit LE word; this
/// module exists to name the RISC-V pattern and pin its invariants.
pub fn apply(site: &mut [u8], site_off: usize, base: u32) -> Option<()> {
    super::RelocKind::apply_base(site, site_off, base)
}

pub fn read_site(site: &[u8], site_off: usize) -> Option<u32> {
    super::RelocKind::read_site_base(site, site_off)
}

#[cfg(test)]
mod tests {
    use super::{apply, read_site};

    #[test]
    fn riscv_site_roundtrip() {
        let mut site = [0u8; 8];
        apply(&mut site, 4, 0x80000000).unwrap();
        assert_eq!(read_site(&site, 4), Some(0x80000000));
    }

    #[test]
    fn riscv_site_preserves_neighbours() {
        let mut site = [0xAAu8; 8];
        apply(&mut site, 1, 0xDEAD_BEEF).unwrap();
        assert_eq!(read_site(&site, 1), Some(0xDEAD_BEEF));
        assert_eq!(site[0], 0xAA); // untouched before the site
        assert_eq!(site[5], 0xAA); // untouched after the site
    }
}