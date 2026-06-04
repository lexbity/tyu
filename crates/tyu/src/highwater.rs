//! Stack high-water cross-check.
//!
//! After a test image runs, compares the runtime-measured peak data-stack depth
//! (the `H` marker) against the conservatively re-derived bound from the ELF
//! code bytes.  The re-derived bound must be >= the measured value.

use std::path::Path;

/// Check that the runtime-measured high-water does not exceed the re-derived
/// conservative bound.
///
/// Returns `Ok(())` if the check passes or if the re-derived bound is `⊤`
/// (unverifiable).  Returns `Err` with a message if the check fails.
pub fn check_high_water(measured: u32, image_path: &Path) -> Result<(), String> {
    let elf_bytes = std::fs::read(image_path)
        .map_err(|e| format!("reading ELF '{}': {}", image_path.display(), e))?;

    // Determine slot_bytes from the ELF class: ELF64 => 8, ELF32 => 4.
    if elf_bytes.len() < 5 {
        return Err("ELF too short".into());
    }
    let slot_bytes: u8 = match elf_bytes[4] {
        2 => 8, // ELF64
        1 => 4, // ELF32
        _ => return Err("unknown ELF class".into()),
    };

    let rederived = harness_core::rederive_elf_high(&elf_bytes, slot_bytes);
    if rederived == harness_core::TOP_SENTINEL {
        return Ok(());
    }
    if measured > rederived {
        return Err(format!(
            "high-water mismatch: runtime measured {} slots but re-derived \
             conservative bound is {} slots (measured > re-derived => analysis unsound)",
            measured, rederived,
        ));
    }
    Ok(())
}
