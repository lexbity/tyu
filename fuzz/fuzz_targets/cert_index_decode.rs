//! Fuzz the `tyu.cert/v1` index decoder (`cert_index_decode`, P11.3).
//!
//! The certification package's `cert.json` parser is hostile input: it is
//! read by `tyu cert verify|show|diff` and by the E6503 pre-ship deploy
//! check. The parser must be total (no panic/abort on any byte string) and
//! fail-closed. This target feeds the parser raw bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tyu::cert::parse_index(data);
});