//! Descriptor compilation — the runtime-side form for backend/loader
//! consumption (design doc §5.3/§5.8, phase P3).
//!
//! `compile` converts a validated [`Descriptor`] into a [`CompiledDescriptor`]
//! (apertures + `platform_hash`), serialized with the shared `codegen-core`
//! encoder so `langc` and the loader decode the same format. The compiled
//! file is written next to `platform.toml` as `platform.desc`; the guard
//! (G14) and the sync test pin it to the source descriptor.

use std::fs;
use std::path::{Path, PathBuf};

use codegen_core::compiled_desc::{
    encode_compiled_desc, validate_compiled_desc, CompiledDescriptor, CompiledDevice,
    CompiledRegister, COMPILED_DESC_DEVICE_CAP, COMPILED_DESC_MAX_BYTES,
    COMPILED_DESC_REGISTER_CAP, COMPILED_DESC_APERTURE_CAP, REG_ACCESS_RO, REG_ACCESS_RW,
    REG_ACCESS_WO, REG_BARRIER_AFTER, REG_BARRIER_BEFORE, REG_BARRIER_BOTH, REG_BARRIER_NONE,
    REG_READ_EFFECTFUL, REG_READ_PLAIN, REG_WRITE_PLAIN, REG_WRITE_W1C, REG_WRITE_W1S,
    REG_WRITE_XOR,
};
use codegen_core::{MmioApertureKind, MmioApertureSpec};

use super::{
    AccessKind, BarrierKind, Descriptor, DescriptorError, E_DESC_INVALID, ReadKind, ApertureKind,
    WriteKind, canonical, parse::parse_descriptor, validate::validate,
};
use crate::error::TyuError;

/// The compiled descriptor file name living next to `platform.toml`.
pub const COMPILED_DESC_FILE: &str = "platform.desc";

/// Path of a pack's compiled descriptor.
pub fn descriptor_file_path(pack_root: &Path) -> PathBuf {
    pack_root.join(COMPILED_DESC_FILE)
}

/// Convert a validated descriptor model into its runtime-side compiled form.
pub fn compile(desc: &Descriptor) -> Result<CompiledDescriptor, DescriptorError> {
    if desc.apertures.len() > COMPILED_DESC_APERTURE_CAP {
        return Err(DescriptorError::new(
            E_DESC_INVALID,
            format!(
                "descriptor declares {} apertures (compiled form supports {})",
                desc.apertures.len(),
                COMPILED_DESC_APERTURE_CAP
            ),
        ));
    }
    if desc.devices.len() > COMPILED_DESC_DEVICE_CAP {
        return Err(DescriptorError::new(
            E_DESC_INVALID,
            format!(
                "descriptor declares {} devices (compiled form supports {})",
                desc.devices.len(),
                COMPILED_DESC_DEVICE_CAP
            ),
        ));
    }
    let mut apertures = [MmioApertureSpec::EMPTY; COMPILED_DESC_APERTURE_CAP];
    for (i, w) in desc.apertures.iter().enumerate() {
        let name = ir::Atom::new(w.name.as_bytes()).ok_or_else(|| {
            DescriptorError::new(
                E_DESC_INVALID,
                format!("aperture [{}] name exceeds 32 bytes", w.id),
            )
        })?;
        apertures[i] = MmioApertureSpec {
            id: w.id,
            name,
            kind: match w.kind {
                ApertureKind::Bus => MmioApertureKind::Bus,
                ApertureKind::Emulated => MmioApertureKind::Emulated,
            },
            base: w.base,
            size: w.size,
            reloc_isa: w.reloc_isa,
        };
    }

    let mut devices = [CompiledDevice::EMPTY; COMPILED_DESC_DEVICE_CAP];
    for (i, d) in desc.devices.iter().enumerate() {
        if d.registers.len() > COMPILED_DESC_REGISTER_CAP {
            return Err(DescriptorError::new(
                E_DESC_INVALID,
                format!(
                    "device {} '{}' declares {} registers (compiled form supports {})",
                    d.map,
                    d.instance,
                    d.registers.len(),
                    COMPILED_DESC_REGISTER_CAP
                ),
            ));
        }
        let map = ir::Atom::new(d.map.as_bytes()).ok_or_else(|| {
            DescriptorError::new(
                E_DESC_INVALID,
                format!("device map name '{}' exceeds 32 bytes", d.map),
            )
        })?;
        let instance = ir::Atom::new(d.instance.as_bytes()).ok_or_else(|| {
            DescriptorError::new(
                E_DESC_INVALID,
                format!("device instance '{}' exceeds 32 bytes", d.instance),
            )
        })?;
        let mut registers = [CompiledRegister::EMPTY; COMPILED_DESC_REGISTER_CAP];
        for (j, r) in d.registers.iter().enumerate() {
            let name = ir::Atom::new(r.name.as_bytes()).ok_or_else(|| {
                DescriptorError::new(
                    E_DESC_INVALID,
                    format!("register '{}' name exceeds 32 bytes", r.name),
                )
            })?;
            registers[j] = CompiledRegister {
                offset: r.offset,
                name,
                width: r.width,
                access: match r.access {
                    AccessKind::Ro => REG_ACCESS_RO,
                    AccessKind::Wo => REG_ACCESS_WO,
                    AccessKind::Rw => REG_ACCESS_RW,
                },
                write_kind: match r.write_kind {
                    WriteKind::Plain => REG_WRITE_PLAIN,
                    WriteKind::W1s => REG_WRITE_W1S,
                    WriteKind::W1c => REG_WRITE_W1C,
                    WriteKind::Xor => REG_WRITE_XOR,
                },
                read_kind: match r.read_kind {
                    ReadKind::Plain => REG_READ_PLAIN,
                    ReadKind::Effectful => REG_READ_EFFECTFUL,
                },
                atomic_max: r.atomic_max,
                mask: r.mask,
                reset: r.reset,
                barrier: match r.barrier {
                    BarrierKind::None => REG_BARRIER_NONE,
                    BarrierKind::Before => REG_BARRIER_BEFORE,
                    BarrierKind::After => REG_BARRIER_AFTER,
                    BarrierKind::Both => REG_BARRIER_BOTH,
                },
                interrupt: r.interrupt.unwrap_or(0xFFFF),
                irq: r.irq.unwrap_or(0xFFFF),
            };
        }
        devices[i] = CompiledDevice {
            map,
            instance,
            aperture: d.aperture,
            base_offset: d.base_offset,
            registers,
            register_count: d.registers.len(),
        };
    }

    Ok(CompiledDescriptor {
        apertures,
        aperture_count: desc.apertures.len(),
        devices,
        device_count: desc.devices.len(),
        platform_hash: canonical::platform_hash(desc),
        verification: codegen_core::compiled_desc::VerificationGrants {
            isr_stack_slots: desc.verification.isr_stack_slots,
        },
    })
}

/// Load, validate, compile, and (if stale) write the compiled descriptor for
/// a platform pack.
///
/// `manifest_path` locates the descriptor TOML (a pack may use the
/// `<name>.platform.toml` layout, so the pack root alone is insufficient);
/// `pack_root` is where `platform.desc` is written (matching langc's
/// `--platform=<dir>` → `<dir>/platform.desc` contract). Every build/test path
/// that selects a platform calls this before invoking `langc`.
pub fn ensure_compiled_descriptor(
    manifest_path: &Path,
    pack_root: &Path,
) -> Result<CompiledDescriptor, TyuError> {
    let text = fs::read_to_string(manifest_path).map_err(|e| {
        TyuError::Platform(format!("reading '{}': {}", manifest_path.display(), e))
    })?;
    let desc = parse_descriptor(&text)
        .map_err(|e| TyuError::Platform(format!("E{} descriptor: {}", e.code, e.detail)))?
        .ok_or_else(|| {
            TyuError::Platform(format!(
                "'{}' carries no descriptor v2 (set schema = 2)",
                manifest_path.display()
            ))
        })?;
    let violations = validate(&desc, Some(pack_root));
    if !violations.is_empty() {
        let first = &violations[0];
        return Err(TyuError::Platform(format!(
            "E{} descriptor for '{}': {}",
            first.code,
            pack_root.display(),
            first.detail
        )));
    }
    let compiled = compile(&desc)
        .map_err(|e| TyuError::Platform(format!("E{} descriptor: {}", e.code, e.detail)))?;

    // Write the compiled form if absent or stale (deterministic bytes).
    let desc_path = descriptor_file_path(pack_root);
    let mut buf = [0u8; COMPILED_DESC_MAX_BYTES];
    let n = encode_compiled_desc(&compiled, &mut buf)
        .map_err(|e| TyuError::Platform(format!("encoding compiled descriptor: {}", e)))?;
    let fresh = &buf[..n];
    match fs::read(&desc_path) {
        Ok(existing) if existing == fresh => {}
        _ => {
            fs::write(&desc_path, fresh)
                .map_err(|e| TyuError::Platform(format!("writing '{}': {}", desc_path.display(), e)))?;
        }
    }
    Ok(compiled)
}

/// Decode + validate a compiled descriptor from bytes (used by the sync test
/// and exposed for langc-side parity checks).
pub fn decode_and_validate(bytes: &[u8]) -> Result<CompiledDescriptor, TyuError> {
    let cd = codegen_core::compiled_desc::decode_compiled_desc(bytes)
        .map_err(|e| TyuError::Platform(format!("decoding compiled descriptor: {}", e)))?;
    validate_compiled_desc(&cd)
        .map_err(|e| TyuError::Platform(format!("compiled descriptor invalid: {}", e)))?;
    Ok(cd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_matches_parse_roundtrip() {
        let text = r#"
[platform]
name = "rp2350"
schema = 2
family = "rp2350"
[[platform.apertures]]
id = 0
name = "apb"
kind = "bus"
base = 0x40000000
size = 0x10000
[[platform.apertures]]
id = 1
name = "mmio"
kind = "bus"
base = 0x20000000
size = 0x1000
"#;
        let desc = parse_descriptor(text).unwrap().unwrap();
        assert!(validate(&desc, None).is_empty());
        let cd = compile(&desc).unwrap();
        assert_eq!(cd.aperture_count, 2);
        assert_eq!(cd.apertures()[0].name.as_bytes(), b"apb");
        assert_eq!(cd.apertures()[0].base, Some(0x40000000));
        assert_eq!(cd.apertures()[1].size, 0x1000);
        // Encode/decode roundtrip through the shared codegen-core format.
        let mut buf = [0u8; COMPILED_DESC_MAX_BYTES];
        let n = encode_compiled_desc(&cd, &mut buf).unwrap();
        let decoded = decode_and_validate(&buf[..n]).unwrap();
        assert_eq!(decoded, cd);
    }

    #[test]
    fn compile_rejects_over_capacity_apertures() {
        let mut text = String::from("[platform]\nname = \"big\"\nschema = 2\nfamily = \"big\"\n");
        for i in 0..9 {
            text.push_str(&format!(
                "[[platform.apertures]]\nid = {i}\nname = \"w{i}\"\nkind = \"bus\"\nbase = 0x{:x}\nsize = 0x1000\n",
                0x40000000 + i * 0x10000
            ));
        }
        let desc = parse_descriptor(&text).unwrap().unwrap();
        let err = compile(&desc).unwrap_err();
        assert_eq!(err.code, E_DESC_INVALID);
        assert!(err.detail.contains("9 apertures"));
    }
}