//! `.lmod` signing tool.
//!
//! Appends a signature/MAC trailer to an `.lmod` container.
//!
//! Usage:
//!   lmod-sign <input.lmod> <output.lmod> [--key=<hex-key>]
//!
//! Default key (for testing): `ab` x 32 (32-byte key, all 0xab).
//! In production the key is managed by a hardware security module / key
//! provisioning system.

use hmac::{Hmac, Mac};
use lmod::header::LmodHeader;
use lmod::sig::{sig_len_for_scheme, signed_region_len, SCHEME_HMAC_SHA256, TRAILER_HEADER_SIZE};
use sha2::Sha256;
use std::fs;
use std::process;

type HmacSha256 = Hmac<Sha256>;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] == "--help" || args[1] == "-h" {
        eprintln!("Usage: lmod-sign <input.lmod> <output.lmod> [--key=<hex-key>]");
        process::exit(1);
    }

    let key_hex = args
        .iter()
        .find_map(|a| a.strip_prefix("--key="))
        .unwrap_or("abababababababababababababababababababababababababababababababab");

    let data = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", args[1], e);
        process::exit(2);
    });

    // Parse header to find the signed region.
    let header = lmod::header::decode_header(&data).unwrap_or_else(|| {
        eprintln!("error: invalid .lmod header");
        process::exit(2);
    });

    let region_len = signed_region_len(&header);
    let signed_region = &data[..region_len];

    // Compute HMAC-SHA256.
    let key_bytes = hex::decode(key_hex).unwrap_or_else(|e| {
        eprintln!("error: invalid key hex: {}", e);
        process::exit(2);
    });

    let mut mac = HmacSha256::new_from_slice(&key_bytes).expect("HMAC accepts any key length");
    mac.update(signed_region);
    let result = mac.finalize();
    let sig_bytes = result.into_bytes();

    // Build output: signed region + trailer.
    let mut out = signed_region.to_vec();
    out.push(SCHEME_HMAC_SHA256);
    out.extend_from_slice(&sig_bytes);

    // Update header: set sig_off and sig_len.
    let sig_off = region_len as u32;
    let sig_len = TRAILER_HEADER_SIZE + sig_len_for_scheme(SCHEME_HMAC_SHA256).unwrap();
    let old_total = header.total_len;
    let new_total = sig_off + sig_len;

    // Patch header fields in the output.
    out[64..68].copy_from_slice(&sig_off.to_le_bytes());
    out[68..72].copy_from_slice(&sig_len.to_le_bytes());
    // Update total_len.
    out[16..20].copy_from_slice(&new_total.to_le_bytes());
    // Set the SIGNED flag in the header flags (byte 6, bit 0).
    out[6] |= lmod::header::LMOD_FLAG_SIGNED as u8;

    fs::write(&args[2], &out).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", args[2], e);
        process::exit(2);
    });

    eprintln!(
        "signed {} -> {} (scheme=HMAC-SHA256, sig_off={}, sig_len={}, region={} bytes)",
        args[1], args[2], sig_off, sig_len, region_len,
    );
}
