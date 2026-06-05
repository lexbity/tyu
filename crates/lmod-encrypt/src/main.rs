//! `.lmod` encryption tool.
//!
//! Encrypts the payload (code/rodata/data) using ChaCha20-Poly1305 with a
//! random CEK, wrapped under one or more KEKs.
//!
//! Usage:
//!   Fleet:  lmod-encrypt <in.lmod> <out.lmod> --mode=fleet --kek=<hex-key>
//!   Device: lmod-encrypt <in.lmod> <out.lmod> --mode=device --device-keys=<dir> --devices=<id1,id2,...>

use std::fs;
use std::path::PathBuf;
use std::process;

use chacha20poly1305::aead::{Aead, AeadInPlace, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use lmod::enc::{
    enc_header_len, encode_enc_header, EncHeader, EncMode, WrappedCekSlot,
    AEAD_CHACHA20POLY1305, CEK_LEN, NONCE_LEN, TAG_LEN, WRAP_LEN, WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
};
use lmod::header::{encode_header, compute_layout, LMOD_FLAG_ENCRYPTED, HEADER_SIZE, FORMAT_VER};
use lmod::validate::Container;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage:");
        eprintln!("  lmod-encrypt <in.lmod> <out.lmod> --mode=fleet --kek=<hex-key>");
        eprintln!("  lmod-encrypt <in.lmod> <out.lmod> --mode=device --device-keys=<dir> --devices=<id1,id2,...>");
        process::exit(1);
    }

    let mode = args.iter().find_map(|a| a.strip_prefix("--mode=")).unwrap_or("fleet");

    let data = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", args[1], e);
        process::exit(2);
    });

    let container = Container::parse(&data).unwrap_or_else(|_| {
        eprintln!("error: invalid .lmod container");
        process::exit(2);
    });

    // Generate random CEK.
    let mut cek_bytes = [0u8; CEK_LEN];
    use rand_core::RngCore;
    OsRng.fill_bytes(&mut cek_bytes);

    // Build payload: code + rodata + data.
    let code = container.code();
    let rodata = container.rodata();
    let data_sec = container.data();
    let mut payload = Vec::with_capacity(code.len() + rodata.len() + data_sec.len());
    payload.extend_from_slice(code);
    payload.extend_from_slice(rodata);
    payload.extend_from_slice(data_sec);

    // Generate random nonce for payload encryption.
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    // Build wrapped CEK slots.
    let slots = match mode {
        "fleet" => build_fleet_slots(&args, &cek_bytes),
        "device" => build_device_slots(&args, &cek_bytes),
        _ => { eprintln!("error: unknown mode '{}'", mode); process::exit(2); }
    };

    let enc_mode = match mode {
        "fleet" => EncMode::Fleet,
        "device" => EncMode::Device,
        _ => unreachable!(),
    };

    let eh = EncHeader {
        enc_mode,
        aead_id: AEAD_CHACHA20POLY1305,
        nonce,
        tag: [0u8; TAG_LEN],
        wrapped_slots: slots,
    };

    let eh_len = enc_header_len(eh.wrapped_slots.len());

    // Build encrypted container.
    let out = build_encrypted_container(&data, &container, &payload, &eh, &cek_bytes, eh_len);

    fs::write(&args[2], &out).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", args[2], e);
        process::exit(2);
    });

    eprintln!(
        "encrypted {} -> {} (mode={}, {} device slots, {} bytes payload, eh_len={})",
        args[1], args[2], mode, eh.wrapped_slots.len(), payload.len(), eh_len,
    );
}

fn build_fleet_slots(args: &[String], cek: &[u8; CEK_LEN]) -> Vec<WrappedCekSlot> {
    let kek_hex = args.iter().find_map(|a| a.strip_prefix("--kek=")).unwrap_or_else(|| {
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
    vec![wrap_cek(&kek_bytes, cek, 0)]
}

fn build_device_slots(args: &[String], cek: &[u8; CEK_LEN]) -> Vec<WrappedCekSlot> {
    let devices_str = args.iter().find_map(|a| a.strip_prefix("--devices=")).unwrap_or_else(|| {
        eprintln!("error: --devices=<id1,id2,...> is required for device mode");
        process::exit(2);
    });
    let keys_dir = args.iter().find_map(|a| a.strip_prefix("--device-keys=")).unwrap_or_else(|| {
        eprintln!("error: --device-keys=<dir> is required for device mode");
        process::exit(2);
    });
    let keys_path = PathBuf::from(keys_dir);

    let device_ids: Vec<&str> = devices_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if device_ids.is_empty() {
        eprintln!("error: no device IDs specified");
        process::exit(2);
    }

    let mut slots = Vec::with_capacity(device_ids.len());
    for (idx, dev_id) in device_ids.iter().enumerate() {
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
            eprintln!("error: device key in '{}' must be 32 bytes (64 hex chars)", key_file.display());
            process::exit(2);
        }
        // Use index as key_id for device mode (0, 1, 2, ...).
        slots.push(wrap_cek(&kek_bytes, cek, idx as u64));
    }
    slots
}

/// Wrap a CEK under a KEK using deterministic zero-nonce ChaCha20-Poly1305.
fn wrap_cek(kek: &[u8], cek: &[u8; CEK_LEN], key_id: u64) -> WrappedCekSlot {
    let wrap_cipher = ChaCha20Poly1305::new(Key::from_slice(kek));
    let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
    let mut cek_buf = *cek;
    let wrap_tag = wrap_cipher.encrypt_in_place_detached(zero_nonce, b"", &mut cek_buf).unwrap();

    let mut wrapped = [0u8; WRAP_LEN];
    wrapped[..NONCE_LEN].copy_from_slice(&[0u8; NONCE_LEN]);
    wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
    wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());

    WrappedCekSlot { key_id, wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, wrapped }
}

fn build_encrypted_container(
    original_bytes: &[u8],
    container: &Container,
    payload: &[u8],
    eh: &EncHeader,
    cek_bytes: &[u8; CEK_LEN],
    eh_len: usize,
) -> Vec<u8> {
    let hdr = container.header();
    let eh_len_u32 = eh_len as u32;
    let cek = Key::from_slice(cek_bytes);

    let layout = compute_layout(
        hdr.abi_hash, hdr.modinfo_len, hdr.code_len, hdr.rodata_len,
        hdr.data_len, hdr.bss_len, hdr.reloc_count, eh_len_u32,
    );

    let total = layout.total_len as usize;
    let mut out = vec![0u8; total];

    let mut header_out = layout;
    header_out.format_ver = FORMAT_VER;
    header_out.flags = hdr.flags | LMOD_FLAG_ENCRYPTED;
    encode_header(&mut out, &header_out);

    let mut eh_bytes = vec![0u8; eh_len];
    encode_enc_header(&mut eh_bytes, eh).unwrap();
    out[HEADER_SIZE as usize..HEADER_SIZE as usize + eh_len].copy_from_slice(&eh_bytes);

    let mi = container.modinfo();
    out[layout.modinfo_off as usize..layout.modinfo_off as usize + mi.len()].copy_from_slice(mi);

    let orig_hdr = container.header();
    let ro = orig_hdr.reloc_off as usize;
    let reloc_count = orig_hdr.reloc_count as usize;
    let reloc_bytes = reloc_count * lmod::reloc::RELOC_ENTRY_SIZE as usize;
    if reloc_bytes > 0 && ro + reloc_bytes <= original_bytes.len() {
        out[layout.reloc_off as usize..layout.reloc_off as usize + reloc_bytes]
            .copy_from_slice(&original_bytes[ro..ro + reloc_bytes]);

        // Adjust reloc site_off values from old code_off to new code_off.
        // The enc-header shifts the code section forward; reloc table entries
        // contain absolute file offsets that must be adjusted by the delta.
        let old_code_off = orig_hdr.code_off;
        let new_code_off = layout.code_off;
        if new_code_off != old_code_off {
            let delta = new_code_off.wrapping_sub(old_code_off);
            let reloc_start = layout.reloc_off as usize;
            for i in 0..reloc_count {
                let entry_off = reloc_start + i * lmod::reloc::RELOC_ENTRY_SIZE as usize;
                let site_off = u32::from_le_bytes(
                    out[entry_off..entry_off + 4].try_into().unwrap()
                );
                let adjusted = site_off.wrapping_add(delta);
                out[entry_off..entry_off + 4].copy_from_slice(&adjusted.to_le_bytes());
            }
        }
    }

    // Build AAD.
    let mut aad = Vec::new();
    aad.extend_from_slice(&out[..HEADER_SIZE as usize]);
    let tag_off_in_eh = 4 + NONCE_LEN;
    eh_bytes[tag_off_in_eh..tag_off_in_eh + TAG_LEN].fill(0);
    aad.extend_from_slice(&eh_bytes);
    aad.extend_from_slice(mi);
    if reloc_bytes > 0 {
        aad.extend_from_slice(&out[layout.reloc_off as usize..layout.reloc_off as usize + reloc_bytes]);
    }

    // Encrypt payload.
    let cipher = ChaCha20Poly1305::new(cek);
    let aead_nonce = Nonce::from_slice(&eh.nonce);
    let ciphertext = cipher.encrypt(aead_nonce, Payload { msg: payload, aad: &aad })
        .unwrap_or_else(|_| { eprintln!("error: payload encryption failed"); process::exit(2); });

    // Write encrypted code.
    let co = layout.code_off as usize;
    out[co..co + hdr.code_len as usize].copy_from_slice(&ciphertext[..hdr.code_len as usize]);

    if hdr.rodata_len > 0 {
        let ro_start = co + hdr.code_len as usize;
        let ro_len = hdr.rodata_len as usize;
        out[ro_start..ro_start + ro_len].copy_from_slice(
            &ciphertext[hdr.code_len as usize..hdr.code_len as usize + ro_len]
        );
    }

    if hdr.data_len > 0 {
        let data_start = layout.data_off as usize;
        let payload_off = (hdr.code_len + hdr.rodata_len) as usize;
        out[data_start..data_start + hdr.data_len as usize].copy_from_slice(
            &ciphertext[payload_off..payload_off + hdr.data_len as usize]
        );
    }

    // Write AEAD tag into enc-header.
    let aead_tag = &ciphertext[ciphertext.len() - TAG_LEN..];
    let tag_off_in_output = HEADER_SIZE as usize + tag_off_in_eh;
    out[tag_off_in_output..tag_off_in_output + TAG_LEN].copy_from_slice(aead_tag);

    out
}
