//! Canonical serialization and `platform_hash` (design doc §5.3).
//!
//! The hash must be independent of TOML formatting, key order, comments, and
//! irrelevant defaults. Canonical form: schema fields emitted in the fixed
//! §5.3 order, text as length-prefixed little-endian byte strings, numerics
//! as plain little-endian, and absent optionals as their resolved defaults
//! (so "explicit default" and "omitted default" hash identically — see the
//! `defaults_resolve_explicit_and_omitted_identically` invariant test).
//!
//! `platform_hash` folds `MMIO_SEM_VER` and the descriptor schema version
//! (decision D-5) so a *format* or *semantics* evolution can never masquerade
//! as a board match.

use super::{
    AccessKind, BarrierKind, Descriptor, DESCRIPTOR_SCHEMA, MMIO_SEM_VER, ReadKind, ApertureKind,
    WriteKind,
};

/// The u64-to-hex rendering used by every downstream consumer of
/// `platform_hash` (modinfo v4 in P6). One formatter, defined here.
pub fn hash_hex(h: u64) -> String {
    format!("{:#018x}", h)
}

/// FNV-1a-64 over the canonical descriptor bytes (decision D-5).
pub fn platform_hash(desc: &Descriptor) -> u64 {
    fnv1a64(&canonical_bytes(desc))
}

/// The canonical byte serialization of a descriptor (§5.3 step 1–2).
pub fn canonical_bytes(desc: &Descriptor) -> Vec<u8> {
    let mut out = Vec::new();

    // Fixed order: schema, name, family.
    encode_u32(&mut out, desc.schema);
    encode_str(&mut out, &desc.name);
    encode_str(&mut out, &desc.family);

    // Apertures sorted by id: {id, name, kind, bind, base, size}.
    let mut apertures = desc.apertures.clone();
    apertures.sort_by_key(|w| w.id);
    encode_u32(&mut out, apertures.len() as u32);
    for w in &apertures {
        encode_u16(&mut out, w.id);
        encode_str(&mut out, &w.name);
        encode_u8(&mut out, aperture_kind_disc(w.kind));
        encode_u8(&mut out, reloc_isa_disc(w.reloc_isa));
        encode_u8(&mut out, w.scratch as u8);
        encode_u64(&mut out, w.base.unwrap_or(0));
        encode_u32(&mut out, w.size);
    }

    // Devices sorted by (map, instance): {map, instance, aperture, base_offset,
    // [registers sorted by offset]}.
    let mut devices = desc.devices.clone();
    devices.sort_by(|a, b| a.map.cmp(&b.map).then_with(|| a.instance.cmp(&b.instance)));
    encode_u32(&mut out, devices.len() as u32);
    for d in &devices {
        encode_str(&mut out, &d.map);
        encode_str(&mut out, &d.instance);
        encode_u16(&mut out, d.aperture);
        encode_u32(&mut out, d.base_offset);
        let mut registers = d.registers.clone();
        registers.sort_by_key(|r| r.offset);
        encode_u32(&mut out, registers.len() as u32);
        for r in &registers {
            encode_u32(&mut out, r.offset);
            encode_str(&mut out, &r.name);
            encode_u8(&mut out, r.width);
            encode_u8(&mut out, access_disc(r.access));
            encode_u8(&mut out, write_kind_disc(r.write_kind));
            encode_u8(&mut out, read_kind_disc(r.read_kind));
            encode_u8(&mut out, r.atomic_max);
            encode_u64(&mut out, r.mask);
            encode_u64(&mut out, r.reset);
            encode_u8(&mut out, barrier_disc(r.barrier));
            encode_u16(&mut out, r.interrupt.unwrap_or(0xFFFF));
            encode_u16(&mut out, r.irq.unwrap_or(0xFFFF));
        }
    }

    // Allocator {region, offset, length, impl} (presence-flagged).
    match &desc.allocator {
        Some(a) => {
            out.push(1);
            encode_str(&mut out, &a.region);
            encode_u64(&mut out, a.offset);
            encode_u64(&mut out, a.length);
            encode_str(&mut out, &a.impl_path);
        }
        None => out.push(0),
    }

    // Scoped metadata_slots_max (presence-flagged).
    match desc.scoped {
        Some(s) => {
            out.push(1);
            encode_u32(&mut out, s.metadata_slots_max);
        }
        None => out.push(0),
    }

    // metal.trust words sorted.
    let mut words = desc.metal_trust.clone();
    words.sort();
    encode_u32(&mut out, words.len() as u32);
    for w in &words {
        encode_str(&mut out, w);
    }

    // Verification grant (static-verification.md §6.4, amended): N_isr only.
    // N_main is derived from the runtime binary's geometry and never declared,
    // so it hashes nothing here. The effective value hashes identically
    // whether declared explicitly or defaulted (isr → 32).
    encode_u32(&mut out, desc.verification.isr_stack_slots);

    // Semantics and schema versions folded in (D-5, FR-19).
    encode_u32(&mut out, MMIO_SEM_VER);
    encode_u32(&mut out, DESCRIPTOR_SCHEMA);

    out
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

fn encode_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn encode_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn encode_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn encode_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn encode_str(out: &mut Vec<u8>, s: &str) {
    encode_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------------
// Enum discriminants — permanent part of the canonical form (never renumber).
// ---------------------------------------------------------------------------

fn aperture_kind_disc(k: ApertureKind) -> u8 {
    match k {
        ApertureKind::Bus => 0,
        ApertureKind::Emulated => 1,
    }
}

fn reloc_isa_disc(r: Option<codegen_core::RelocIsa>) -> u8 {
    match r {
        None => 0,
        Some(codegen_core::RelocIsa::ArmThumbLdrLiteral) => 1,
        Some(codegen_core::RelocIsa::RiscVHi20Lo12) => 2,
    }
}

fn access_disc(a: AccessKind) -> u8 {
    match a {
        AccessKind::Ro => 0,
        AccessKind::Wo => 1,
        AccessKind::Rw => 2,
    }
}

fn write_kind_disc(w: WriteKind) -> u8 {
    match w {
        WriteKind::Plain => 0,
        WriteKind::W1s => 1,
        WriteKind::W1c => 2,
        WriteKind::Xor => 3,
    }
}

fn read_kind_disc(r: ReadKind) -> u8 {
    match r {
        ReadKind::Plain => 0,
        ReadKind::Effectful => 1,
    }
}

fn barrier_disc(b: BarrierKind) -> u8 {
    match b {
        BarrierKind::None => 0,
        BarrierKind::Before => 1,
        BarrierKind::After => 2,
        BarrierKind::Both => 3,
    }
}

// ---------------------------------------------------------------------------
// FNV-1a-64
// ---------------------------------------------------------------------------

/// Standard FNV-1a 64-bit hash over arbitrary bytes.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::desc::parse::parse_descriptor;

    fn parse(text: &str) -> Descriptor {
        parse_descriptor(text)
            .unwrap()
            .expect("descriptor present")
    }

    const DESCRIPTOR: &str = r#"
[platform]
name = "demo"
schema = 2
family = "demo-fam"

[[platform.apertures]]
id = 0
name = "apb"
kind = "bus"
base = 0x40000000
size = 0x10000

[[platform.devices]]
map = "GPIO"
instance = "gpio0"
aperture = 0
base_offset = 0xd000
registers = [
  { offset = 0x0, name = "ctrl", width = 32, access = "rw" },
  { offset = 0x4, name = "intr", width = 32, access = "rw", write_kind = "w1c" },
]

[platform.allocator]
region = "SRAM"
offset = 0x8400
length = 0x77000
impl = "glue/region"

[platform.scoped]
metadata_slots_max = 64

[platform.metal.trust]
words = ["platform.boot.enter_xip"]
"#;

    #[test]
    fn canonical_is_deterministic() {
        let a = parse(DESCRIPTOR);
        let b = parse(DESCRIPTOR);
        assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
        assert_eq!(platform_hash(&a), platform_hash(&b));
    }

    #[test]
    fn canonical_is_sorted_regardless_of_source_order() {
        // Same model, devices/registers listed in reverse order in TOML.
        let text = r#"
[platform]
name = "demo"
schema = 2
family = "demo-fam"
[[platform.apertures]]
id = 0
name = "apb"
kind = "bus"
base = 0x40000000
size = 0x10000
[[platform.devices]]
map = "GPIO"
instance = "gpio0"
aperture = 0
base_offset = 0xd000
registers = [
  { offset = 0x4, name = "intr", width = 32, access = "rw", write_kind = "w1c" },
  { offset = 0x0, name = "ctrl", width = 32, access = "rw" },
]
[platform.allocator]
region = "SRAM"
offset = 0x8400
length = 0x77000
impl = "glue/region"
[platform.scoped]
metadata_slots_max = 64
[platform.metal.trust]
words = ["platform.boot.enter_xip"]
"#;
        assert_eq!(
            canonical_bytes(&parse(DESCRIPTOR)),
            canonical_bytes(&parse(text)),
            "canonical form must be independent of source row order"
        );
    }

    #[test]
    fn hash_hex_is_0x_padded_16() {
        assert_eq!(hash_hex(0), "0x0000000000000000");
        assert_eq!(hash_hex(0x1234_5678_9abc_def0), "0x123456789abcdef0");
    }

    #[test]
    fn fnv1a_known_vector() {
        // FNV-1a-64 of empty input.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}