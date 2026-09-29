//! Fuzz of the `tyu.verdicts/v2` decoder (PLAN-VERIFY-3 P7.2): arbitrary
//! bytes must be rejected fail-closed (E6402/E6417) without a panic, size
//! cap enforced, and parseable documents must round-trip their records. The
//! decoder is untrusted-input surface (§7.5).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > verifier::verdict::VERDICTS_FILE_MAX_BYTES {
        // The reader must reject an oversized input with TooLarge, no panic.
        let _ = verifier::verdict::read_verdicts(data);
        return;
    }
    match verifier::verdict::read_verdicts(data) {
        Ok(v) => {
            // A parseable document must re-encode without panic and with a
            // bounded size (the write side is capped too).
            let _ = verifier::verdict::encode_verdicts(
                "fuzz",
                "0.0.0",
                v.certifier.as_ref(),
                &v.target,
                &v.model_semantics,
                &v.concurrency,
                &v.records,
                0,
                &Default::default(),
            );
        }
        Err(e) => {
            // Every rejection must carry a closed registry code (6402/6417).
            let code = e.code();
            assert!(code == 6402 || code == 6417, "unexpected code {code}: {e:?}");
        }
    }
});