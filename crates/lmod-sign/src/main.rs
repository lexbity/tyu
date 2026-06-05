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
    let mut header = lmod::header::decode_header(&data).unwrap_or_else(|| {
        eprintln!("error: invalid .lmod header");
        process::exit(2);
    });

    // Compute the final header values BEFORE signing, so the HMAC is
    // computed over the exact bytes that the loader will verify.
    header.flags |= lmod::header::LMOD_FLAG_SIGNED;
    let region_len = signed_region_len(&header);
    let sig_off = region_len as u32;
    let sig_len = TRAILER_HEADER_SIZE + sig_len_for_scheme(SCHEME_HMAC_SHA256).unwrap();
    let new_total = sig_off + sig_len;

    let mut signed_region = data[..region_len].to_vec();
    // Patch the header fields that will differ between the encrypted container
    // and the signed output: flags, total_len, sig_off, sig_len.
    signed_region[6..8].copy_from_slice(&header.flags.to_le_bytes());
    signed_region[16..20].copy_from_slice(&new_total.to_le_bytes());
    signed_region[64..68].copy_from_slice(&sig_off.to_le_bytes());
    signed_region[68..72].copy_from_slice(&sig_len.to_le_bytes());

    // Compute HMAC-SHA256 over the corrected signed region.
    let key_bytes = hex::decode(key_hex).unwrap_or_else(|e| {
        eprintln!("error: invalid key hex: {}", e);
        process::exit(2);
    });

    let mut mac = HmacSha256::new_from_slice(&key_bytes).expect("HMAC accepts any key length");
    mac.update(&signed_region);
    let result = mac.finalize();
    let sig_bytes = result.into_bytes();

    // Build output: signed region + trailer.
    let mut out = signed_region;
    out.push(SCHEME_HMAC_SHA256);
    out.extend_from_slice(&sig_bytes);

    // Update header offsets.
    let sig_off = region_len as u32;
    let sig_len = TRAILER_HEADER_SIZE + sig_len_for_scheme(SCHEME_HMAC_SHA256).unwrap();
    let new_total = sig_off + sig_len;

    // Update total_len and sig fields in the output header.
    out[16..20].copy_from_slice(&new_total.to_le_bytes());
    out[64..68].copy_from_slice(&sig_off.to_le_bytes());
    out[68..72].copy_from_slice(&sig_len.to_le_bytes());

    fs::write(&args[2], &out).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", args[2], e);
        process::exit(2);
    });

    eprintln!(
        "signed {} -> {} (scheme=HMAC-SHA256, sig_off={}, sig_len={}, region={} bytes)",
        args[1], args[2], sig_off, sig_len, region_len,
    );
}
