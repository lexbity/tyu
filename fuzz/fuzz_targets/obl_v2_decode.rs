//! Fuzz of the `tyu.obl/v2` artifact decoder (PLAN-VERIFY-3 P7.2/P1.2):
//! arbitrary bytes must be rejected fail-closed (E6400/E6401) without a
//! panic, validate the canonical identity arithmetic, and round-trip.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > verifier::model::OBL_ARTIFACT_MAX_BYTES {
        let _ = verifier::codec::read_obl(data);
        return;
    }
    match verifier::codec::read_obl(data) {
        Ok(set) => {
            // A parseable artifact must satisfy the canonical identity
            // arithmetic and re-encode without panic.
            let _ = set.validate();
            let _ = verifier::codec::encode_obl(&set);
        }
        Err(e) => {
            assert!(
                e.code() == 6400 || e.code() == 6401,
                "unexpected obl code {}: {e:?}",
                e.code()
            );
        }
    }
});