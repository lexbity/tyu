//! ARM Thumb window-base reloc site (P6).
//!
//! Site pattern: `ldr rN, [pc, #imm]` over a literal-pool word. The reloc
//! site is that word: the assembler materialises `ldr rN, =__lang_window_N_base`
//! as a pc-relative load from a pool entry carrying an `R_ARM_ABS32`
//! relocation against the extern symbol. At pack time the word is bound to
//! the window base (a plain 32-bit little-endian address); at load time the
//! loader re-derives the same base from its descriptor (`check_window_base`).
//!
//! `apply_base`/`read_site_base` from the shared `mod` write/read that word
//! unchanged — the lone literal holds the *address*, not an addend.

/// The shared apply/read semantics are exactly the 32-bit LE word; this
/// module exists to name the ARM pattern and pin its invariants.
pub fn apply(site: &mut [u8], site_off: usize, base: u32) -> Option<()> {
    super::RelocKind::apply_base(site, site_off, base)
}

pub fn read_site(site: &[u8], site_off: usize) -> Option<u32> {
    super::RelocKind::read_site_base(site, site_off)
}

#[cfg(test)]
mod tests {
    use super::{apply, read_site};
    use super::super::WINDOW_BASE_SITE_SIZE;

    #[test]
    fn arm_site_roundtrip() {
        let mut site = [0u8; 8];
        apply(&mut site, 2, 0x20000000).unwrap();
        assert_eq!(read_site(&site, 2), Some(0x20000000));
        assert_eq!(WINDOW_BASE_SITE_SIZE, 4);
    }

    #[test]
    fn arm_site_out_of_range_is_none() {
        let mut site = [0u8; 4];
        assert!(apply(&mut site, 2, 0).is_none());
        assert_eq!(read_site(&site, 2), None);
    }
}