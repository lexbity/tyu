//! Byte-level fuzz of the platform descriptor reader (Phase P1).
//!
//! Exercises parse → validate → canonical/hash on arbitrary bytes (lossily
//! converted to text). The validator runs with `pack_root = None` so the
//! disk-backed `metal.trust` rule is skipped. Added to the nightly tier, not
//! the PR tier.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(Some(desc)) = tyu::platform::desc::parse::parse_descriptor(&text) {
        let _ = tyu::platform::desc::validate::validate(&desc, None);
        let _ = tyu::platform::desc::canonical::platform_hash(&desc);
    }
});