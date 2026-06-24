//! `.lmod` encryption tool — binary entry point.
//!
//! Usage:
//!   Fleet:  lmod-encrypt <in.lmod> <out.lmod> --mode=fleet --kek=<hex-key>
//!   Device: lmod-encrypt <in.lmod> <out.lmod> --mode=device --device-keys=<dir> --devices=<id1,id2,...>

use std::fs;
use std::path::PathBuf;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage:");
        eprintln!("  lmod-encrypt <in.lmod> <out.lmod> --mode=fleet --kek=<hex-key>");
        eprintln!("  lmod-encrypt <in.lmod> <out.lmod> --mode=device --device-keys=<dir> --devices=<id1,id2,...>");
        process::exit(1);
    }

    let data = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", args[1], e);
        process::exit(2);
    });

    let mode = args
        .iter()
        .find_map(|a| a.strip_prefix("--mode="))
        .unwrap_or("fleet");

    let result = match mode {
        "fleet" => {
            let kek_hex = args
                .iter()
                .find_map(|a| a.strip_prefix("--kek="))
                .unwrap_or_else(|| {
                    eprintln!("error: --kek=<hex-key> is required for fleet mode");
                    process::exit(2);
                });
            let kek_bytes = hex::decode(kek_hex).unwrap_or_else(|e| {
                eprintln!("error: invalid KEK hex: {}", e);
                process::exit(2);
            });
            if kek_bytes.len() != 32 {
                eprintln!("error: KEK must be 32 bytes (64 hex chars)");
                process::exit(2);
            }
            let mut kek = [0u8; 32];
            kek.copy_from_slice(&kek_bytes);
            lmod_encrypt::encrypt_fleet(&data, &kek)
        }
        "device" => {
            let devices_str = args
                .iter()
                .find_map(|a| a.strip_prefix("--devices="))
                .unwrap_or_else(|| {
                    eprintln!("error: --devices=<id1,id2,...> is required for device mode");
                    process::exit(2);
                });
            let keys_dir = args
                .iter()
                .find_map(|a| a.strip_prefix("--device-keys="))
                .unwrap_or_else(|| {
                    eprintln!("error: --device-keys=<dir> is required for device mode");
                    process::exit(2);
                });
            let keys_path = PathBuf::from(keys_dir);

            let device_ids: Vec<&str> = devices_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if device_ids.is_empty() {
                eprintln!("error: no device IDs specified");
                process::exit(2);
            }

            let mut device_keys: Vec<(String, [u8; 32])> = Vec::with_capacity(device_ids.len());
            for dev_id in &device_ids {
                let key_file = keys_path.join(format!("{}.key", dev_id));
                let hex_content = fs::read_to_string(&key_file).unwrap_or_else(|e| {
                    eprintln!("error: reading device key '{}': {}", key_file.display(), e);
                    process::exit(2);
                });
                let kek_bytes = hex::decode(hex_content.trim()).unwrap_or_else(|e| {
                    eprintln!("error: invalid hex in '{}': {}", key_file.display(), e);
                    process::exit(2);
                });
                if kek_bytes.len() != 32 {
                    eprintln!(
                        "error: device key in '{}' must be 32 bytes (64 hex chars)",
                        key_file.display()
                    );
                    process::exit(2);
                }
                let mut kek = [0u8; 32];
                kek.copy_from_slice(&kek_bytes);
                device_keys.push((dev_id.to_string(), kek));
            }
            lmod_encrypt::encrypt_device(&data, &device_keys)
        }
        _ => {
            eprintln!("error: unknown mode '{}'", mode);
            process::exit(2);
        }
    };

    let out = match result {
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
        "encrypted {} -> {} (mode={}, {} bytes)",
        args[1],
        args[2],
        mode,
        out.len()
    );
}
