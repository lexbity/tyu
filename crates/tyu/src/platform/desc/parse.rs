//! Descriptor v2 parsing — extends the existing platform.toml reading path.
//!
//! The descriptor is parsed from the *same* `platform.toml` the v1 manifest
//! reader consumes, but as an independent serde model: the v1
//! `PlatformManifest` keeps its fields, and this module reads the v2 sections
//! (`[platform] schema/family`, `[[platform.windows]]`, `[[platform.devices]]`,
//! `[platform.allocator]`, `[platform.scoped]`, `[platform.metal.trust]`) plus
//! the `[memory]` sections the allocator validation needs.
//!
//! Unknown keys inside the *new* v2 sections are hard parse errors (E3647) —
//! a typo'd `write_knd` must never be silently ignored, because that would
//! change the descriptor's hash and therefore its identity (decision D-5).
//! Unknown *kind names* (window/access/write/read/barrier) are E3646 with the
//! supported set listed (decision D-2 registry rule).

use super::{
    full_mask, AccessKind, AllocatorSpec, BarrierKind, Descriptor, DescriptorError, DeviceMap,
    E_DESC_INVALID, E_DESC_UNKNOWN_KIND, MmioWindow, MemoryModel, MemoryRegionSpec, ReadKind,
    RegisterRow, ScopedSpec, WindowKind, WriteKind,
};

/// Parse a platform.toml text into a validated-at-parse descriptor.
///
/// `Ok(None)` when the manifest carries no descriptor v2 content (a legacy
/// pack). `Err` carries a single parse error (E3646/E3647) — the first
/// failure, matching serde's fail-fast contract.
pub fn parse_descriptor(text: &str) -> Result<Option<Descriptor>, DescriptorError> {
    let raw: RawManifest = toml::from_str(text).map_err(descriptor_parse_error)?;
    let Some(platform) = raw.platform else {
        return Ok(None);
    };

    let has_v2 = platform.schema.is_some()
        || !platform.windows.is_empty()
        || !platform.devices.is_empty()
        || platform.allocator.is_some()
        || platform.scoped.is_some()
        || platform.metal.is_some();
    if !has_v2 {
        return Ok(None);
    }

    let schema = platform.schema.unwrap_or(0);
    if schema == 0 {
        return Err(DescriptorError::new(
            E_DESC_INVALID,
            "missing `[platform] schema` (set schema = 2)",
        ));
    }
    if schema != super::DESCRIPTOR_SCHEMA {
        return Err(DescriptorError::new(
            E_DESC_INVALID,
            format!(
                "unsupported descriptor schema {} (this toolchain supports {})",
                schema,
                super::DESCRIPTOR_SCHEMA
            ),
        ));
    }

    let windows = platform
        .windows
        .iter()
        .map(|w| {
            Ok(MmioWindow {
                id: w.id,
                name: w.name.clone(),
                kind: parse_window_kind(&w.kind)?,
                base: w.base,
                size: w.size,
                reloc_isa: w.bind.as_deref().map(parse_reloc_isa).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, DescriptorError>>()?;

    let devices = platform
        .devices
        .iter()
        .map(|d| {
            let registers = d
                .registers
                .iter()
                .map(parse_register)
                .collect::<Result<Vec<_>, DescriptorError>>()?;
            Ok(DeviceMap {
                map: d.map.clone(),
                instance: d.instance.clone(),
                window: d.window,
                base_offset: d.base_offset,
                registers,
            })
        })
        .collect::<Result<Vec<_>, DescriptorError>>()?;

    let allocator = platform.allocator.as_ref().map(|a| AllocatorSpec {
        region: a.region.clone(),
        offset: a.offset,
        length: a.length,
        impl_path: a.r#impl.clone().unwrap_or_default(),
        policy: a.policy.clone().unwrap_or_default(),
    });

    let scoped = platform
        .scoped
        .as_ref()
        .map(|s| ScopedSpec {
            metadata_slots_max: s.metadata_slots_max,
        });

    let metal_trust = platform
        .metal
        .as_ref()
        .and_then(|m| m.trust.as_ref())
        .map(|t| t.words.clone())
        .unwrap_or_default();

    Ok(Some(Descriptor {
        schema,
        name: platform.name,
        family: platform.family.unwrap_or_default(),
        description: platform.description,
        windows,
        devices,
        allocator,
        scoped,
        metal_trust,
        memory: memory_model(&raw.memory),
    }))
}

fn parse_register(r: &RawRegister) -> Result<RegisterRow, DescriptorError> {
    let write_kind = parse_write_kind(r.write_kind.as_deref().unwrap_or("plain"))?;
    let read_kind = parse_read_kind(r.read_kind.as_deref().unwrap_or("plain"))?;
    let barrier = parse_barrier(r.barrier.as_deref().unwrap_or("none"))?;
    let access = parse_access(&r.access)?;
    Ok(RegisterRow {
        offset: r.offset,
        name: r.name.clone(),
        width: r.width,
        access,
        write_kind,
        read_kind,
        atomic_max: r.atomic_max.unwrap_or(r.width),
        mask: r.mask.unwrap_or_else(|| full_mask(r.width)),
        reset: r.reset.unwrap_or(0),
        barrier,
    })
}

fn memory_model(raw: &Option<RawMemory>) -> MemoryModel {
    let mut regions = Vec::new();
    let Some(memory) = raw else {
        return MemoryModel { regions };
    };
    let ds_region = memory.ds_region.as_deref();
    let ds_size = memory.ds_size;
    for region in [&memory.flash, &memory.sram].into_iter().flatten() {
        let this_ds_size = if ds_region == Some(region.name.as_str()) {
            ds_size
        } else {
            None
        };
        regions.push(MemoryRegionSpec {
            name: region.name.clone(),
            origin: region.origin,
            length: region.length,
            ds_size: this_ds_size,
        });
    }
    MemoryModel { regions }
}

// ---------------------------------------------------------------------------
// Kind-name mapping (decision D-2: compiler-owned, registry-validated names).
// ---------------------------------------------------------------------------

fn parse_reloc_isa(s: &str) -> Result<codegen_core::RelocIsa, DescriptorError> {
    match s {
        "arm-thumb-ldr-literal" => Ok(codegen_core::RelocIsa::ArmThumbLdrLiteral),
        "riscv-hi20-lo12" => Ok(codegen_core::RelocIsa::RiscVHi20Lo12),
        other => Err(DescriptorError::new(
            E_DESC_INVALID,
            format!(
                "unknown window bind '{other}' (supported: arm-thumb-ldr-literal, riscv-hi20-lo12)"
            ),
        )),
    }
}

fn parse_window_kind(s: &str) -> Result<WindowKind, DescriptorError> {
    match s {
        "bus" => Ok(WindowKind::Bus),
        "emulated" => Ok(WindowKind::Emulated),
        other => Err(unknown_kind("window kind", other, &["bus", "emulated"])),
    }
}

fn parse_access(s: &str) -> Result<AccessKind, DescriptorError> {
    match s {
        "ro" => Ok(AccessKind::Ro),
        "wo" => Ok(AccessKind::Wo),
        "rw" => Ok(AccessKind::Rw),
        other => Err(unknown_kind("register access", other, &["ro", "wo", "rw"])),
    }
}

fn parse_write_kind(s: &str) -> Result<WriteKind, DescriptorError> {
    match s {
        "plain" => Ok(WriteKind::Plain),
        "w1s" => Ok(WriteKind::W1s),
        "w1c" => Ok(WriteKind::W1c),
        other => Err(unknown_kind("write kind", other, &["plain", "w1s", "w1c"])),
    }
}

fn parse_read_kind(s: &str) -> Result<ReadKind, DescriptorError> {
    match s {
        "plain" => Ok(ReadKind::Plain),
        "effectful" => Ok(ReadKind::Effectful),
        other => Err(unknown_kind("read kind", other, &["plain", "effectful"])),
    }
}

fn parse_barrier(s: &str) -> Result<BarrierKind, DescriptorError> {
    match s {
        "none" => Ok(BarrierKind::None),
        "before" => Ok(BarrierKind::Before),
        "after" => Ok(BarrierKind::After),
        "both" => Ok(BarrierKind::Both),
        other => Err(unknown_kind(
            "barrier",
            other,
            &["none", "before", "after", "both"],
        )),
    }
}

fn unknown_kind(kind: &str, found: &str, supported: &[&str]) -> DescriptorError {
    DescriptorError::new(
        E_DESC_UNKNOWN_KIND,
        format!(
            "unknown {kind} '{found}' (supported: {})",
            supported.join("|")
        ),
    )
}

fn descriptor_parse_error(e: toml::de::Error) -> DescriptorError {
    let span = e
        .span()
        .map(|r| format!(" at byte {}..{}", r.start, r.end))
        .unwrap_or_default();
    DescriptorError::new(
        E_DESC_INVALID,
        format!("descriptor parse error{span}: {e}"),
    )
}

// ---------------------------------------------------------------------------
// Raw serde shapes. `deny_unknown_fields` on every *new* v2 section: a typo
// in a new key must be a hard error, never a silent hash-affecting omission.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct RawManifest {
    #[serde(default)]
    platform: Option<RawPlatformSection>,
    #[serde(default)]
    memory: Option<RawMemory>,
}

// v1 fields (`compiler_interface`, `isa`) are acknowledged so
// deny_unknown_fields does not reject them; they are never read by the
// descriptor. `RawRegion.exec` is likewise v1-only.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlatformSection {
    name: String,
    #[serde(default)]
    schema: Option<u32>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "compiler-interface")]
    compiler_interface: Option<u16>,
    /// v1 ISA rows — acknowledged so deny_unknown_fields does not reject them.
    #[serde(default)]
    isa: Vec<toml::Value>,
    #[serde(default)]
    windows: Vec<RawWindow>,
    #[serde(default)]
    devices: Vec<RawDevice>,
    #[serde(default)]
    allocator: Option<RawAllocator>,
    #[serde(default)]
    scoped: Option<RawScoped>,
    #[serde(default)]
    metal: Option<RawPlatformMetal>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWindow {
    id: u16,
    name: String,
    kind: String,
    #[serde(default)]
    base: Option<u64>,
    size: u32,
    /// Optional `bind = "arm-thumb-ldr-literal" | "riscv-hi20-lo12"` (P6).
    #[serde(default)]
    bind: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevice {
    map: String,
    instance: String,
    window: u16,
    base_offset: u32,
    #[serde(default)]
    registers: Vec<RawRegister>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegister {
    offset: u32,
    name: String,
    width: u8,
    access: String,
    #[serde(default)]
    write_kind: Option<String>,
    #[serde(default)]
    read_kind: Option<String>,
    #[serde(default)]
    atomic_max: Option<u8>,
    #[serde(default)]
    mask: Option<u64>,
    #[serde(default)]
    reset: Option<u64>,
    #[serde(default)]
    barrier: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAllocator {
    region: String,
    offset: u64,
    length: u64,
    #[serde(default)]
    r#impl: Option<String>,
    #[serde(default)]
    policy: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScoped {
    metadata_slots_max: u32,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlatformMetal {
    #[serde(default)]
    trust: Option<RawMetalTrust>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetalTrust {
    #[serde(default)]
    words: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RawMemory {
    #[serde(default)]
    flash: Option<RawRegion>,
    #[serde(default)]
    sram: Option<RawRegion>,
    #[serde(default)]
    ds_region: Option<String>,
    #[serde(default)]
    ds_size: Option<u64>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct RawRegion {
    name: String,
    origin: u64,
    length: u64,
    #[serde(default)]
    exec: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(text: &str) -> Descriptor {
        parse_descriptor(text).unwrap().expect("descriptor present")
    }

    #[test]
    fn legacy_manifest_has_no_descriptor() {
        let text = r#"
[platform]
name = "demo"
compiler-interface = 1
description = "demo pack"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
"#;
        assert!(parse_descriptor(text).unwrap().is_none());
    }

    #[test]
    fn schema_two_with_empty_tables_parses() {
        let text = r#"
[platform]
name = "demo"
schema = 2
family = "demo"
"#;
        let desc = parse_ok(text);
        assert_eq!(desc.schema, 2);
        assert_eq!(desc.name, "demo");
        assert_eq!(desc.family, "demo");
        assert!(desc.windows.is_empty());
        assert!(desc.devices.is_empty());
        assert!(desc.allocator.is_none());
        assert!(desc.metal_trust.is_empty());
    }

    #[test]
    fn defaults_resolve_explicit_and_omitted_identically() {
        let explicit = r#"
[platform]
name = "demo"
schema = 2
family = "demo"

[[platform.windows]]
id = 0
name = "mmio"
kind = "bus"
base = 0x20000000
size = 0x1000

[[platform.devices]]
map = "Scratch"
instance = "scratch"
window = 0
base_offset = 0x0
registers = [
  { offset = 0x0, name = "A", width = 32, access = "rw", write_kind = "plain", read_kind = "plain", atomic_max = 32 },
]
"#;
        let omitted = r#"
[platform]
name = "demo"
schema = 2
family = "demo"
[[platform.windows]]
id = 0
name = "mmio"
kind = "bus"
size = 0x1000
[[platform.devices]]
map = "Scratch"
instance = "scratch"
window = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "A", width = 32, access = "rw" },
]
"#;
        let a = parse_ok(explicit);
        let b = parse_ok(omitted);
        let reg_a = &a.devices[0].registers[0];
        let reg_b = &b.devices[0].registers[0];
        assert_eq!(reg_a, reg_b, "explicit defaults must equal omitted defaults");
        assert_eq!(reg_a.write_kind, WriteKind::Plain);
        assert_eq!(reg_a.atomic_max, 32);
        assert_eq!(reg_a.mask, 0xffff_ffff);
        assert_eq!(reg_a.reset, 0);
        assert_eq!(reg_a.barrier, BarrierKind::None);
    }

    #[test]
    fn unknown_write_kind_is_e3646_with_supported_set() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.devices]]
map = "G"
instance = "g"
window = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "r", width = 32, access = "rw", write_kind = "w1x" },
]
"#;
        let err = parse_descriptor(text).unwrap_err();
        assert_eq!(err.code, E_DESC_UNKNOWN_KIND);
        assert!(err.detail.contains("write kind"), "{}", err.detail);
        assert!(err.detail.contains("plain|w1s|w1c"), "{}", err.detail);
    }

    #[test]
    fn unknown_key_in_new_section_is_e3647() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.windows]]
id = 0
name = "mmio"
kind = "bus"
size = 0x1000
siz = 0x2000
"#;
        let err = parse_descriptor(text).unwrap_err();
        assert_eq!(err.code, E_DESC_INVALID);
        assert!(err.detail.contains("unknown field"), "{}", err.detail);
    }

    #[test]
    fn missing_schema_is_e3647() {
        let text = r#"
[platform]
name = "demo"
[[platform.windows]]
id = 0
name = "mmio"
kind = "bus"
size = 0x1000
"#;
        let err = parse_descriptor(text).unwrap_err();
        assert_eq!(err.code, E_DESC_INVALID);
        assert!(err.detail.contains("schema"), "{}", err.detail);
    }

    #[test]
    fn unsupported_schema_is_e3647() {
        let text = r#"
[platform]
name = "demo"
schema = 1
"#;
        let err = parse_descriptor(text).unwrap_err();
        assert_eq!(err.code, E_DESC_INVALID);
        assert!(err.detail.contains("schema 1"), "{}", err.detail);
    }

    #[test]
    fn memory_model_flags_ds_region() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[memory]
flash = { name = "FLASH", origin = 0x10000000, length = 0x400000 }
sram = { name = "SRAM", origin = 0x20000000, length = 0x84000 }
ds_region = "SRAM"
ds_size = 0x4000
"#;
        let desc = parse_ok(text);
        assert_eq!(desc.memory.regions.len(), 2);
        let sram = desc.region("SRAM").unwrap();
        assert_eq!(sram.ds_size, Some(0x4000));
        assert_eq!(desc.region("FLASH").unwrap().ds_size, None);
    }
}