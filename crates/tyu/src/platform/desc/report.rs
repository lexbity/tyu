//! Descriptor model rendering — the review artifact for board packs.
//!
//! `tyu platform lint` prints this after the pack lint outcome: windows,
//! device tables, register rows with *effective* semantics (defaults
//! resolved), allocator span, and the computed `platform_hash` (design doc
//! §5.11, decision D-14).

use super::canonical::{hash_hex, platform_hash};
use super::Descriptor;
use std::fmt::Write as _;

/// Render the validated descriptor model as a deterministic human-readable
/// report.
pub fn format_descriptor_report(desc: &Descriptor) -> String {
    let mut out = String::new();

    let _ = writeln!(
        &mut out,
        "descriptor {} schema={} family={}",
        desc.name, desc.schema, desc.family
    );
    let _ = writeln!(&mut out, "platform-hash {}", hash_hex(platform_hash(desc)));

    let _ = writeln!(&mut out, "windows {}", desc.windows.len());
    for w in &desc.windows {
        let base = w
            .base
            .map(|b| format!("{:#x}", b))
            .unwrap_or_else(|| "link".to_string());
        let _ = writeln!(
            &mut out,
            "  [{}] {} kind={} base={} size={:#x}",
            w.id, w.name, w.kind.as_str(), base, w.size
        );
    }

    let _ = writeln!(&mut out, "devices {}", desc.devices.len());
    for d in &desc.devices {
        let _ = writeln!(
            &mut out,
            "  {} {} window={} base_offset={:#x} extent={:#x}",
            d.map,
            d.instance,
            d.window,
            d.base_offset,
            d.extent()
        );
        for r in &d.registers {
            let _ = writeln!(
                &mut out,
                "    {} offset={:#06x} width={} access={} write={} read={} atomic={} mask={:#x} reset={:#x} barrier={}",
                r.name,
                r.offset,
                r.width,
                r.access.as_str(),
                r.write_kind.as_str(),
                r.read_kind.as_str(),
                r.atomic_max,
                r.mask,
                r.reset,
                r.barrier.as_str(),
            );
        }
    }

    match &desc.allocator {
        Some(a) => {
            let _ = writeln!(
                &mut out,
                "allocator region={} offset={:#x} length={:#x} impl={} policy={}",
                a.region, a.offset, a.length, a.impl_path, a.policy
            );
        }
        None => {
            let _ = writeln!(&mut out, "allocator (none)");
        }
    }

    match desc.scoped {
        Some(s) => {
            let _ = writeln!(&mut out, "scoped metadata_slots_max={}", s.metadata_slots_max);
        }
        None => {
            let _ = writeln!(&mut out, "scoped (none)");
        }
    }

    if desc.metal_trust.is_empty() {
        let _ = writeln!(&mut out, "metal.trust (none)");
    } else {
        let mut words = desc.metal_trust.clone();
        words.sort();
        let _ = writeln!(&mut out, "metal.trust {}", words.join(", "));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::desc::parse::parse_descriptor;

    #[test]
    fn report_contains_all_model_sections() {
        let text = r#"
[platform]
name = "rp2350"
schema = 2
family = "rp2350"
[[platform.windows]]
id = 0
name = "apb"
kind = "bus"
base = 0x40000000
size = 0x10000
[[platform.devices]]
map = "GPIO"
instance = "gpio0"
window = 0
base_offset = 0xd000
registers = [
  { offset = 0x0, name = "ctrl", width = 32, access = "rw" },
  { offset = 0x4, name = "intr_stat", width = 32, access = "rw", write_kind = "w1c" },
]
[platform.allocator]
region = "SRAM"
offset = 0x8400
length = 0x77000
impl = "glue/region"
policy = "pool-firstfit"
[platform.scoped]
metadata_slots_max = 64
"#;
        let desc = parse_descriptor(text).unwrap().unwrap();
        let report = format_descriptor_report(&desc);
        assert!(report.contains("descriptor rp2350 schema=2 family=rp2350"));
        assert!(report.contains("platform-hash 0x"));
        assert!(report.contains("windows 1"));
        assert!(report.contains("[0] apb kind=bus base=0x40000000 size=0x10000"));
        assert!(report.contains("devices 1"));
        assert!(report.contains("GPIO gpio0 window=0 base_offset=0xd000 extent=0x8"));
        assert!(report.contains("ctrl offset=0x0000 width=32 access=rw write=plain read=plain"));
        assert!(report.contains("intr_stat offset=0x0004 width=32 access=rw write=w1c"));
        assert!(report.contains(
            "allocator region=SRAM offset=0x8400 length=0x77000 impl=glue/region policy=pool-firstfit"
        ));
        assert!(report.contains("scoped metadata_slots_max=64"));
        assert!(report.contains("metal.trust (none)"));
    }
}