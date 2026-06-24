//! `.lmod` signing tool — binary entry point.
//!
//! Usage:
//!   lmod-sign <input.lmod> <output.lmod> --key=<hex-key>

use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] == "--help" || args[1] == "-h" {
        eprintln!("Usage: lmod-sign <input.lmod> <output.lmod> --key=<hex-key>");
        process::exit(1);
    }

    let key_hex = args
        .iter()
        .find_map(|a| a.strip_prefix("--key="))
        .unwrap_or_else(|| {
            eprintln!("error: --key=<hex-key> is required");
            process::exit(2);
        });

    let data = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", args[1], e);
        process::exit(2);
    });

    let key_bytes = hex::decode(key_hex).unwrap_or_else(|e| {
        eprintln!("error: invalid key hex: {}", e);
        process::exit(2);
    });
    if key_bytes.len() != 32 {
        eprintln!("error: key must be 32 bytes (64 hex chars)");
        process::exit(2);
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);

    let out = match lmod_sign::sign(&data, &key) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(2);
        }
    };

    fs::write(&args[2], &out).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", args[2], e);
        process::exit(2);
    });

    eprintln!(
        "signed {} -> {} (scheme=HMAC-SHA256, {} bytes)",
        args[1],
        args[2],
        out.len(),
    );
}
