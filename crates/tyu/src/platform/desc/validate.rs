//! Descriptor v2 whole-model validation (§5.2 validation rules).
//!
//! Every rule is a hard error; the validator collects *all* violations (one
//! `DescriptorError` per violation) so a board author fixes a full list in one
//! pass. `langc` (later phases) and `tyu platform lint` share this one
//! implementation (FR-2).
//!
//! The `metal.trust.words` rule needs the pack on disk; pass `pack_root` from
//! the lint path. Pure model validation (fuzz, unit tests) passes `None` and
//! skips the disk-backed rule.

use super::{
    AccessKind, Descriptor, DescriptorError, DeviceMap, E_DESC_INVALID, RegisterRow, ApertureKind,
    WriteKind, full_mask,
};
use std::path::Path;

/// Validate a descriptor as a whole model. Returns one error per violation.
pub fn validate(desc: &Descriptor, pack_root: Option<&Path>) -> Vec<DescriptorError> {
    let mut errors = Vec::new();

    validate_apertures(desc, &mut errors);
    validate_devices(desc, &mut errors);
    validate_verification(desc, &mut errors);
    validate_allocator(desc, pack_root, &mut errors);
    validate_metal_trust(desc, pack_root, &mut errors);

    errors
}

fn err(detail: impl Into<String>) -> DescriptorError {
    DescriptorError::new(E_DESC_INVALID, detail)
}

fn validate_apertures(desc: &Descriptor, errors: &mut Vec<DescriptorError>) {
    // Rule: aperture ids unique and densely numbered from 0.
    let mut ids: Vec<u16> = desc.apertures.iter().map(|w| w.id).collect();
    ids.sort_unstable();
    for (index, &id) in ids.iter().enumerate() {
        if id as usize != index {
            errors.push(err(format!(
                "aperture ids must be unique and densely numbered from 0 (found id {id} at index {index})"
            )));
            break;
        }
    }

    // Rule: aperture size > 0.
    for w in &desc.apertures {
        if w.size == 0 {
            errors.push(err(format!(
                "aperture [{}] '{}': size must be > 0",
                w.id, w.name
            )));
        }
    }

    // Rule: aperture names are the stable identity a module binds on — they
    // must be unique within a descriptor.
    let mut names: Vec<&str> = desc.apertures.iter().map(|w| w.name.as_str()).collect();
    names.sort_unstable();
    for pair in names.windows(2) {
        if pair[0] == pair[1] {
            errors.push(err(format!("aperture names must be unique (duplicate '{}')", pair[0])));
        }
    }

    // Rule (P6 binding): an emulated aperture's base is a link-time symbol with
    // runtime-dynamic addressing — it has no binding-time relocation. A bind
    // on an emulated aperture is a contradiction.
    for w in &desc.apertures {
        if w.kind == ApertureKind::Emulated && w.reloc_isa.is_some() {
            errors.push(err(format!(
                "aperture [{}] '{}': an emulated aperture must not declare a bind (its addressing is runtime-dynamic)",
                w.id, w.name
            )));
        }
    }

    // Rule (P8): a bus aperture must not alias a [memory] region unless it is
    // declared `scratch = true` (the D-14 QEMU/HIL test fiction). A silent
    // aperture/SRAM overlap on a real board is exactly the "module writes
    // whatever occupies those addresses" hazard symbolic MMIO exists to
    // prevent. Emulated apertures (link-time base) carry no board address.
    for w in &desc.apertures {
        if w.kind != ApertureKind::Bus || w.scratch {
            continue;
        }
        let Some(base) = w.base else { continue };
        let end = base.saturating_add(w.size as u64);
        for r in &desc.memory.regions {
            let r_end = r.origin.saturating_add(r.length);
            if base < r_end && r.origin < end {
                errors.push(err(format!(
                    "aperture [{}] '{}' at 0x{:x}..0x{:x} overlaps memory region '{}' — declare `scratch = true` if this is the deliberate QEMU/HIL test fiction",
                    w.id, w.name, base, end, r.name
                )));
            }
        }
    }

    // Rule: two apertures with absolute bases must not overlap address space.
    // Emulated apertures (link-time base) are excluded — their placement is a
    // link decision, not a board fact.
    let mut ranged: Vec<(u64, u64)> = desc
        .apertures
        .iter()
        .filter_map(|w| w.base.map(|b| (b, b.saturating_add(w.size as u64))))
        .collect();
    ranged.sort_unstable();
    for pair in ranged.windows(2) {
        if pair[0].1 > pair[1].0 {
            errors.push(err(format!(
                "apertures at base 0x{:x} and 0x{:x} overlap",
                pair[0].0, pair[1].0
            )));
        }
    }
}

fn validate_devices(desc: &Descriptor, errors: &mut Vec<DescriptorError>) {
    // Rule: device (map, instance) identity unique (canonical §5.3 sorts by it).
    let mut keys: Vec<(&str, &str)> = desc
        .devices
        .iter()
        .map(|d| (d.map.as_str(), d.instance.as_str()))
        .collect();
    keys.sort_unstable();
    for pair in keys.windows(2) {
        if pair[0] == pair[1] {
            errors.push(err(format!(
                "device (map, instance) must be unique (duplicate '{}' '{}')",
                pair[0].0, pair[0].1
            )));
        }
    }

    for d in &desc.devices {
        let Some(aperture) = desc.aperture(d.aperture) else {
            errors.push(err(format!(
                "device {} '{}': references unknown aperture id {}",
                d.map, d.instance, d.aperture
            )));
            continue;
        };

        // Rule: register offsets unique within a map.
        let mut offsets: Vec<u32> = d.registers.iter().map(|r| r.offset).collect();
        offsets.sort_unstable();
        for pair in offsets.windows(2) {
            if pair[0] == pair[1] {
                errors.push(err(format!(
                    "device {} '{}': duplicate register offset 0x{:x}",
                    d.map, d.instance, pair[0]
                )));
            }
        }

        for r in &d.registers {
            validate_register(d, r, errors);
        }

        // Rule: base_offset + map extent ≤ aperture.size.
        let extent = d.extent();
        if extent > 0 && d.base_offset.saturating_add(extent) > aperture.size {
            errors.push(err(format!(
                "device {} '{}': base_offset 0x{:x} + extent 0x{:x} > aperture [{}] '{}' size 0x{:x}",
                d.map, d.instance, d.base_offset, extent, aperture.id, aperture.name, aperture.size
            )));
        }
    }
}

fn validate_register(d: &DeviceMap, r: &RegisterRow, errors: &mut Vec<DescriptorError>) {
    // Rule: width one of 8|16|32|64.
    if !matches!(r.width, 8 | 16 | 32 | 64) {
        errors.push(err(format!(
            "device {} '{}' register '{}': width {} not in {{8,16,32,64}}",
            d.map, d.instance, r.name, r.width
        )));
    }

    // Rule: register offsets word-aligned to their width.
    let bytes = r.width_bytes();
    if bytes > 0 && r.offset % bytes != 0 {
        errors.push(err(format!(
            "device {} '{}' register '{}': offset 0x{:x} not aligned to width {}",
            d.map, d.instance, r.name, r.offset, r.width
        )));
    }

    // Rule: atomic_max ≤ width.
    if r.atomic_max > r.width {
        errors.push(err(format!(
            "device {} '{}' register '{}': atomic_max {} > width {}",
            d.map, d.instance, r.name, r.atomic_max, r.width
        )));
    }
    if !matches!(r.atomic_max, 8 | 16 | 32 | 64) {
        errors.push(err(format!(
            "device {} '{}' register '{}': atomic_max {} not in {{8,16,32,64}}",
            d.map, d.instance, r.name, r.atomic_max
        )));
    }

    // Rule (P8): datasheet IRQ numbers fit an NVIC IRQ (0..=64 on Cortex-M).
    for (label, irq) in [("interrupt", r.interrupt), ("irq", r.irq)] {
        if let Some(n) = irq {
            if n > 64 {
                errors.push(err(format!(
                    "device {} '{}' register '{}': {label} {} out of NVIC range 0..=64",
                    d.map, d.instance, r.name, n
                )));
            }
        }
    }

    // Rule: access = ro forbids write_kind ≠ plain (a store to a ro register
    // is not expressible; w1s/w1c on ro would be a phantom write).
    if r.access == AccessKind::Ro && r.write_kind != WriteKind::Plain {
        errors.push(err(format!(
            "device {} '{}' register '{}': access=ro forbids write_kind != plain",
            d.map, d.instance, r.name
        )));
    }

    // Rule: mask must be representable within the register's width.
    if r.width < 64 && r.mask > full_mask(r.width) {
        errors.push(err(format!(
            "device {} '{}' register '{}': mask 0x{:x} exceeds width {} bits",
            d.map, d.instance, r.name, r.mask, r.width
        )));
    }
}

fn validate_verification(desc: &Descriptor, errors: &mut Vec<DescriptorError>) {
    use super::E_DESC_VERIFICATION_INVALID;
    let e = |detail: String| DescriptorError::new(E_DESC_VERIFICATION_INVALID, detail);

    // Rule: `isr_stack_slots` must be positive (default 32 when omitted).
    // `data_stack_slots` is not a descriptor field any more (amended §6.4:
    // N_main is derived from the runtime binary's geometry); a stale key is
    // rejected at parse time with a migration message.
    if desc.verification.isr_stack_slots == 0 {
        errors.push(e(
            "[verification] isr_stack_slots must be > 0 (default 32 when omitted)".into(),
        ));
    }
}

fn validate_allocator(
    desc: &Descriptor,
    pack_root: Option<&Path>,
    errors: &mut Vec<DescriptorError>,
) {
    let Some(a) = &desc.allocator else {
        return;
    };
    // Rule (P8): the declared allocator implementation module must exist in
    // the pack; a dangling `impl` path would advertise an implementation
    // nothing can resolve.
    if let Some(root) = pack_root {
        if !a.impl_path.is_empty() && !root.join(&a.impl_path).is_dir() {
            errors.push(err(format!(
                "allocator impl '{}' does not exist in the platform pack",
                a.impl_path
            )));
        }
    }

    // Rule: allocator region must name a [memory] region.
    let Some(region) = desc.region(&a.region) else {
        errors.push(err(format!(
            "allocator region '{}' does not name a [memory] region",
            a.region
        )));
        return;
    };

    // Rule: [offset, offset + length] must fit the region.
    if a.offset.saturating_add(a.length) > region.length {
        errors.push(err(format!(
            "allocator span [{:#x}, {:#x}) exceeds region '{}' length {:#x}",
            a.offset,
            a.offset + a.length,
            region.name,
            region.length
        )));
    }

    // Rule: must not overlap the ds_region's span [0, ds_size) in the region.
    if let Some(ds_size) = region.ds_size {
        if a.offset < ds_size {
            errors.push(err(format!(
                "allocator offset {:#x} overlaps ds_region span [0, {:#x})",
                a.offset, ds_size
            )));
        }
    }
}

fn validate_metal_trust(
    desc: &Descriptor,
    pack_root: Option<&Path>,
    errors: &mut Vec<DescriptorError>,
) {
    let Some(root) = pack_root else {
        return;
    };
    for word in &desc.metal_trust {
        if !word_declared_in_pack(root, word) {
            errors.push(err(format!(
                "metal.trust word '{word}' not declared in platform pack"
            )));
        }
    }
}

/// True when `word` is declared (`: word ...`) in some `.def`/`.mod` file
/// under `root`.
fn word_declared_in_pack(root: &Path, word: &str) -> bool {
    fn walk(dir: &Path, word: &str, found: &mut bool) {
        if *found {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *found {
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(&path, word, found);
            } else if matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("def") | Some("mod")
            ) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if declared_in_text(&text, word) {
                        *found = true;
                        return;
                    }
                }
            }
        }
    }
    let mut found = false;
    walk(root, word, &mut found);
    found
}

fn declared_in_text(text: &str, word: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(':') else {
            continue;
        };
        if let Some(name) = rest.split_whitespace().next() {
            if name == word {
                return true;
            }
        }
    }
    false
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

    fn codes(text: &str) -> Vec<u16> {
        validate(&parse(text), None).iter().map(|e| e.code).collect()
    }

    #[test]
    fn dense_aperture_ids_pass() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.apertures]]
id = 1
name = "b"
kind = "bus"
size = 0x1000
"#;
        assert!(validate(&parse(text), None).is_empty());
    }

    #[test]
    fn aperture_id_gap_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.apertures]]
id = 2
name = "b"
kind = "bus"
size = 0x1000
"#;
        assert!(validate(&parse(text), None)
            .iter()
            .any(|e| e.detail.contains("densely numbered")));
    }

    #[test]
    fn zero_size_aperture_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0
"#;
        let errors = validate(&parse(text), None);
        assert!(errors.iter().any(|e| e.detail.contains("size must be > 0")));
    }

    #[test]
    fn device_overflowing_aperture_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x100
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0x100
registers = [
  { offset = 0x0, name = "r", width = 32, access = "rw" },
]
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("> aperture")), "{:?}", errors);
    }

    #[test]
    fn misaligned_register_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0
registers = [
  { offset = 0x1, name = "r", width = 32, access = "rw" },
]
"#;
        let errors = validate(&parse(text), None);
        assert!(errors.iter().any(|e| e.detail.contains("not aligned")));
    }

    #[test]
    fn atomic_over_width_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "r", width = 32, access = "rw", atomic_max = 64 },
]
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("atomic_max 64 > width 32")));
    }

    #[test]
    fn ro_with_w1c_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "r", width = 32, access = "ro", write_kind = "w1c" },
]
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("access=ro forbids write_kind")));
    }

    #[test]
    fn effectful_ro_is_legal() {
        // read-to-clear status registers: read_kind = effectful on access = ro
        // is explicitly legal (§5.2).
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x1000
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "status", width = 32, access = "ro", read_kind = "effectful" },
]
"#;
        assert!(validate(&parse(text), None).is_empty());
    }

    #[test]
    fn allocator_region_unknown_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x10000 }
[platform.allocator]
region = "NOTHING"
offset = 0x0
length = 0x100
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("does not name a [memory] region")));
    }

    #[test]
    fn allocator_out_of_bounds_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x1000 }
[platform.allocator]
region = "SRAM"
offset = 0x800
length = 0x1000
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("exceeds region 'SRAM'")));
    }

    #[test]
    fn allocator_overlapping_ds_rejected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x10000 }
ds_region = "SRAM"
ds_size = 0x4000
[platform.allocator]
region = "SRAM"
offset = 0x2000
length = 0x1000
"#;
        let errors = validate(&parse(text), None);
        assert!(errors
            .iter()
            .any(|e| e.detail.contains("overlaps ds_region span")));
    }

    #[test]
    fn allocator_valid_after_ds_passes() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x10000 }
ds_region = "SRAM"
ds_size = 0x4000
[platform.allocator]
region = "SRAM"
offset = 0x4000
length = 0x1000
"#;
        assert!(validate(&parse(text), None).is_empty());
    }

    #[test]
    fn bus_aperture_aliasing_memory_requires_scratch_flag() {
        // The D-14 QEMU scratch is a deliberate fiction; undeclared, it is
        // exactly the "module writes whatever occupies those addresses"
        // hazard symbolic MMIO exists to prevent.
        let text = r#"
[platform]
name = "t"
schema = 2

[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x82000 }

[[platform.apertures]]
id = 0
name = "evil"
kind = "bus"
base = 0x20000000
size = 0x1000
"#;
        let errors = validate(&parse(text), None);
        assert!(
            errors
                .iter()
                .any(|e| e.detail.contains("overlaps memory region 'SRAM'")
                    && e.detail.contains("scratch = true")),
            "unexpected errors: {errors:?}"
        );
    }

    #[test]
    fn scratch_aperture_aliasing_memory_passes() {
        let text = r#"
[platform]
name = "t"
schema = 2

[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x82000 }

[[platform.apertures]]
id = 0
name = "scratch"
kind = "bus"
base = 0x20000000
size = 0x1000
scratch = true
"#;
        assert!(validate(&parse(text), None).is_empty());
    }

    #[test]
    fn allocator_impl_must_exist_in_pack() {
        let text = r#"
[platform]
name = "t"
schema = 2

[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x82000 }

[platform.allocator]
region = "SRAM"
offset = 0x8400
length = 0x1000
impl = "glue/region"
"#;
        let dir = std::env::temp_dir().join("tyu_desc_alloc_impl_missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let errors = validate(&parse(text), Some(&dir));
        assert!(
            errors
                .iter()
                .any(|e| e.detail.contains("allocator impl 'glue/region' does not exist")),
            "unexpected errors: {errors:?}"
        );

        // With the directory present the descriptor validates.
        std::fs::create_dir_all(dir.join("glue/region")).unwrap();
        assert!(validate(&parse(text), Some(&dir)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_violations_collected() {
        let text = r#"
[platform]
name = "demo"
schema = 2
[[platform.apertures]]
id = 0
name = "a"
kind = "bus"
size = 0x100
[[platform.devices]]
map = "G"
instance = "g"
aperture = 0
base_offset = 0xf0
registers = [
  { offset = 0x1, name = "r", width = 32, access = "ro", write_kind = "w1c", atomic_max = 64 },
]
"#;
        let errors = validate(&parse(text), None);
        assert!(errors.len() >= 3, "expected multiple violations: {:?}", errors);
        assert!(codes(text).iter().all(|c| *c == E_DESC_INVALID));
    }

    // -----------------------------------------------------------------------
    // [verification] profile grants (slice P3, E6403)
    // -----------------------------------------------------------------------

    fn verification_pack(verification: &str) -> String {
        format!(
            r#"
[platform]
name = "demo"
schema = 2
family = "demo"
[memory]
sram = {{ name = "SRAM", origin = 0x20000000, length = 0x10000 }}
ds_region = "SRAM"
ds_size = 0x4000
{verification}
"#
        )
    }

    #[test]
    fn verification_absent_is_defaulted() {
        let desc = parse(&verification_pack(""));
        assert_eq!(desc.verification.isr_stack_slots, 32);
        assert!(validate(&desc, None).is_empty());
    }

    #[test]
    fn verification_isr_grant_passes() {
        let desc = parse(&verification_pack(
            r#"[verification]
isr_stack_slots = 32
"#,
        ));
        assert!(validate(&desc, None).is_empty(), "{:?}", validate(&desc, None));
    }

    #[test]
    fn removed_data_stack_slots_key_rejected_with_migration_message() {
        // Amended §6.4: N_main is derived from the runtime binary's geometry,
        // never hand-declared — a stale `data_stack_slots` key must fail loud
        // (E6403) with the migration message, never silently ignore.
        let err = parse_descriptor(&verification_pack(
            r#"[verification]
data_stack_slots = 2048
"#,
        ))
        .unwrap_err();
        assert_eq!(err.code, 6403);
        assert!(
            err.detail.contains("data_stack_slots was removed"),
            "{:?}",
            err
        );
    }

    #[test]
    fn verification_zero_isr_slots_rejected() {
        let desc = parse(&verification_pack(
            r#"[verification]
isr_stack_slots = 0
"#,
        ));
        let errors = validate(&desc, None);
        assert!(
            errors
                .iter()
                .any(|e| e.code == 6403 && e.detail.contains("isr_stack_slots must be > 0")),
            "{:?}",
            errors
        );
    }
}