//! Descriptor v2 golden and negative-corpus tests (Phase P1).
//!
//! Pins:
//! - `platform_hash` invariance under formatting/key-order/default-omission;
//! - `platform_hash` sensitivity to semantic edits (write_kind flips);
//! - the §5.2 validation rules against the `bad_desc/` negative corpus;
//! - the NFR-2 wall-clock bound for a 256-register descriptor.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use tyu::platform::desc::{
    canonical::{canonical_bytes, platform_hash},
    parse::parse_descriptor,
    validate::validate,
    Descriptor,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bad_desc")
}

fn parse(text: &str) -> Descriptor {
    parse_descriptor(text).unwrap().expect("descriptor present")
}

// ---------------------------------------------------------------------------
// Formatting invariance (FR-19 / NFR-1)
// ---------------------------------------------------------------------------

/// The canonical P1 rp2350 descriptor in three formatting variants that must
/// hash identically: (a) canonical, (b) reordered keys + comments + inline
/// vs block table styles, (c) explicit defaults omitted.
const FORMAT_A: &str = r#"
[platform]
name = "rp2350"
schema = 2
family = "rp2350"

[[platform.windows]]
id = 0
name = "apb"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x40000000
size = 0x10000

[[platform.windows]]
id = 1
name = "mmio"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x20000000
size = 0x1000

[[platform.devices]]
map = "GPIO"
instance = "gpio0"
window = 0
base_offset = 0xD000
registers = [
  { offset = 0x000, name = "ctrl",     width = 32, access = "rw", write_kind = "plain", read_kind = "plain", atomic_max = 32 },
  { offset = 0x004, name = "intr_stat", width = 32, access = "rw", write_kind = "w1c",  read_kind = "plain", atomic_max = 32 },
  { offset = 0x020, name = "fifo",     width = 8,  access = "rw", write_kind = "plain", read_kind = "effectful", atomic_max = 8 },
]

[platform.allocator]
region = "SRAM"
offset = 0x8400
length = 0x77000
impl = "glue/region"
policy = "pool-firstfit"

[platform.scoped]
metadata_slots_max = 64

[platform.metal.trust]
words = []
"#;

const FORMAT_B: &str = r#"
# reformatted: keys reordered, comments added, inline tables, defaults explicit
[platform]
family = "rp2350"            # board family hook (no v1 semantics)
schema = 2
name = "rp2350"

[[platform.windows]]
id = 1
name = "mmio"
size = 0x1000
base = 0x20000000
kind = "bus"
bind = "arm-thumb-ldr-literal"

[[platform.windows]]
id = 0
name = "apb"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x40000000
size = 0x10000

[[platform.devices]]
map = "GPIO"
window = 0
base_offset = 0xD000
instance = "gpio0"
registers = [
  { offset = 0x000, name = "ctrl",     access = "rw", width = 32, write_kind = "plain", read_kind = "plain", atomic_max = 32 },
  { offset = 0x004, name = "intr_stat", access = "rw", width = 32, write_kind = "w1c",  read_kind = "plain", atomic_max = 32 },
  { offset = 0x020, name = "fifo",     access = "rw", width = 8,  write_kind = "plain", read_kind = "effectful", atomic_max = 8 },
]

[platform.allocator]
policy = "pool-firstfit"
impl = "glue/region"
length = 0x77000
offset = 0x8400
region = "SRAM"

[platform.scoped]
metadata_slots_max = 64

[platform.metal.trust]
words = []
"#;

const FORMAT_C: &str = r#"
# defaults omitted: write_kind/read_kind/atomic_max/mask/reset/barrier all
# resolve to their defaults, which must hash identically to FORMAT_A.
[platform]
name = "rp2350"
schema = 2
family = "rp2350"

[[platform.windows]]
id = 0
name = "apb"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x40000000
size = 0x10000

[[platform.windows]]
id = 1
name = "mmio"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x20000000
size = 0x1000

[[platform.devices]]
map = "GPIO"
instance = "gpio0"
window = 0
base_offset = 0xD000
registers = [
  { offset = 0x000, name = "ctrl",     width = 32, access = "rw" },
  { offset = 0x004, name = "intr_stat", width = 32, access = "rw", write_kind = "w1c" },
  { offset = 0x020, name = "fifo",     width = 8,  access = "rw", read_kind = "effectful" },
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

#[test]
fn hash_invariant_under_formatting() {
    let a = parse(FORMAT_A);
    let b = parse(FORMAT_B);
    let c = parse(FORMAT_C);

    let ha = platform_hash(&a);
    assert_eq!(
        ha,
        platform_hash(&b),
        "reordering keys/rows or reformatting must not change platform_hash"
    );
    assert_eq!(
        ha,
        platform_hash(&c),
        "explicit-vs-omitted defaults must not change platform_hash"
    );

    // Golden pin for the P1 rp2350 descriptor.
    assert_eq!(
        ha,
        0x6098_dc89_8c6e_f1fa,
        "rp2350 platform_hash golden must be stable"
    );
}

#[test]
fn hash_changes_on_semantic_edit() {
    let base = parse(FORMAT_A);
    let edited = FORMAT_A.replace(
        r#"{ offset = 0x004, name = "intr_stat", width = 32, access = "rw", write_kind = "w1c",  read_kind = "plain", atomic_max = 32 },"#,
        r#"{ offset = 0x004, name = "intr_stat", width = 32, access = "rw", write_kind = "w1s",  read_kind = "plain", atomic_max = 32 },"#,
    );
    let flipped = parse(&edited);

    let h_base = platform_hash(&base);
    let h_flipped = platform_hash(&flipped);
    assert_ne!(
        h_base, h_flipped,
        "changing a register write_kind must change platform_hash"
    );
    // Pin both hex values so a semantic drift is caught on either side.
    assert_eq!(h_base, 0x6098_dc89_8c6e_f1fa);
    assert_eq!(h_flipped, 0x6afd_da16_2ff9_0f47);
}

#[test]
fn canonical_bytes_are_sorted_and_deterministic() {
    let a = parse(FORMAT_A);
    let b = parse(FORMAT_B);
    assert_eq!(
        canonical_bytes(&a),
        canonical_bytes(&b),
        "canonical serialization must be independent of source layout"
    );
}

// ---------------------------------------------------------------------------
// Negative corpus — one fixture per §5.2 validation rule (E3646/E3647).
// ---------------------------------------------------------------------------

fn check_fixture(name: &str, expected_code: u16, needle: &str) {
    let text = fs::read_to_string(fixture_dir().join(name))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"));
    match parse_descriptor(&text) {
        Err(e) => {
            assert_eq!(e.code, expected_code, "fixture {name}");
            assert!(
                e.detail.contains(needle),
                "fixture {name}: detail '{}' missing needle '{}'",
                e.detail,
                needle
            );
        }
        Ok(Some(desc)) => {
            let errors = validate(&desc, None);
            let hit = errors.iter().any(|e| {
                e.code == expected_code && e.detail.contains(needle)
            });
            assert!(hit, "fixture {name}: no {expected_code} with '{}' among {:?}", needle, errors);
        }
        Ok(None) => panic!("fixture {name} should produce a descriptor"),
    }
}

#[test]
fn validate_rejects_every_rule() {
    check_fixture(
        "bad_window_id_dup.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "densely numbered",
    );
    check_fixture(
        "bad_window_id_gap.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "densely numbered",
    );
    check_fixture(
        "bad_window_size_zero.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "size must be > 0",
    );
    check_fixture(
        "bad_window_name_dup.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "window names must be unique",
    );
    check_fixture(
        "bad_window_base_overlap.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "overlap",
    );
    check_fixture(
        "bad_device_unknown_window.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "references unknown window id",
    );
    check_fixture(
        "bad_device_overflows_window.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "> window",
    );
    check_fixture(
        "bad_reg_offset_dup.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "duplicate register offset",
    );
    check_fixture(
        "bad_reg_misaligned.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "not aligned to width",
    );
    check_fixture(
        "bad_atomic_over_width.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "atomic_max 64 > width 32",
    );
    check_fixture(
        "bad_ro_write_kind.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "access=ro forbids write_kind",
    );
    check_fixture(
        "bad_allocator_region_unknown.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "does not name a [memory] region",
    );
    check_fixture(
        "bad_allocator_out_of_bounds.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "exceeds region 'SRAM'",
    );
    check_fixture(
        "bad_allocator_overlaps_ds.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "overlaps ds_region span",
    );
    check_fixture(
        "bad_mask_width.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "exceeds width 8 bits",
    );
    check_fixture(
        "bad_schema.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "schema 1",
    );
    check_fixture(
        "bad_unknown_key.toml",
        tyu::platform::desc::E_DESC_INVALID,
        "parse error",
    );
    check_fixture(
        "bad_window_kind.toml",
        tyu::platform::desc::E_DESC_UNKNOWN_KIND,
        "bus|emulated",
    );
    check_fixture(
        "bad_write_kind.toml",
        tyu::platform::desc::E_DESC_UNKNOWN_KIND,
        "plain|w1s|w1c",
    );
    check_fixture(
        "bad_read_kind.toml",
        tyu::platform::desc::E_DESC_UNKNOWN_KIND,
        "plain|effectful",
    );
}

#[test]
fn metal_trust_word_must_exist_in_pack() {
    let text = fs::read_to_string(fixture_dir().join("bad_metal_trust_word.toml")).unwrap();
    let desc = parse(&text);
    assert_eq!(desc.metal_trust, vec!["platform.boot.enter_xip"]);

    // A pack root with no .def/.mod declaring the word → E3647.
    let root = std::env::temp_dir().join("tyu_bad_metal_trust");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("glue")).unwrap();
    fs::write(
        root.join("glue/gpio.def"),
        ": platform.gpio.init ( usize usize -- ) {mmio}\n",
    )
    .unwrap();

    let errors = validate(&desc, Some(&root));
    let hit = errors
        .iter()
        .any(|e| e.detail.contains("metal.trust word 'platform.boot.enter_xip' not declared"));
    assert!(hit, "{:?}", errors);

    // A pack root that DOES declare the word passes.
    let root2 = std::env::temp_dir().join("tyu_good_metal_trust");
    let _ = fs::remove_dir_all(&root2);
    fs::create_dir_all(root2.join("platform")).unwrap();
    fs::write(
        root2.join("platform/boot.def"),
        ": platform.boot.enter_xip ( -- )\n",
    )
    .unwrap();
    assert!(
        validate(&desc, Some(&root2)).is_empty(),
        "declared word must validate"
    );
}

#[test]
fn effectful_ro_register_is_legal() {
    // read-to-clear status register: read_kind = effectful on access = ro is
    // the canonical legal pattern (§5.2). The negative corpus has no fixture
    // for it because it is NOT a violation.
    let text = r#"
[platform]
name = "ok"
schema = 2
family = "ok"
[[platform.windows]]
id = 0
name = "a"
kind = "bus"
base = 0x40000000
size = 0x1000
[[platform.devices]]
map = "G"
instance = "g"
window = 0
base_offset = 0
registers = [
  { offset = 0x0, name = "status", width = 32, access = "ro", read_kind = "effectful" },
]
"#;
    let desc = parse(text);
    assert!(validate(&desc, None).is_empty(), "effectful ro must be legal");
}

// ---------------------------------------------------------------------------
// NFR-2: descriptor validation wall-clock bound
// ---------------------------------------------------------------------------

#[test]
fn large_descriptor_validates_within_budget() {
    // 256 registers in one device, plus allocator/scoped.
    let mut text = String::from(
        r#"
[platform]
name = "big"
schema = 2
family = "big"
[[platform.windows]]
id = 0
name = "apb"
kind = "bus"
bind = "arm-thumb-ldr-literal"
base = 0x40000000
size = 0x10000
[[platform.devices]]
map = "BIG"
instance = "big0"
window = 0
base_offset = 0
registers = [
"#,
    );
    for i in 0..256u32 {
        let offset = i * 4;
        let kind = if i % 3 == 0 { "w1c" } else { "plain" };
        text.push_str(&format!(
            "  {{ offset = 0x{:x}, name = \"r{}\", width = 32, access = \"rw\", write_kind = \"{}\" }},\n",
            offset, i, kind
        ));
    }
    text.push_str(
        r#"]
[memory]
sram = { name = "SRAM", origin = 0x20000000, length = 0x10000 }
ds_region = "SRAM"
ds_size = 0x4000
[platform.allocator]
region = "SRAM"
offset = 0x4000
length = 0x1000
"#,
    );

    let start = Instant::now();
    let desc = parse(&text);
    let errors = validate(&desc, None);
    let elapsed = start.elapsed();
    assert!(
        errors.is_empty(),
        "256-register descriptor must validate clean: {:?}",
        errors
    );
    assert!(
        elapsed.as_millis() <= 50,
        "256-register descriptor validation took {} ms (> 50 ms)",
        elapsed.as_millis()
    );
}

// ---------------------------------------------------------------------------
// Determinism across repeated runs (NFR-1)
// ---------------------------------------------------------------------------

#[test]
fn hash_repeatable_across_parses() {
    let first = platform_hash(&parse(FORMAT_A));
    for _ in 0..8 {
        assert_eq!(first, platform_hash(&parse(FORMAT_A)));
    }
}

#[test]
fn real_pack_descriptors_parse_and_validate() {
    // The in-repo packs must carry valid schema-2 descriptors. This is the
    // G14 guard's model-level counterpart.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    for (name, path) in [
        ("rp2350", "platforms/rp2350/platform.toml"),
        ("x86_64-unknown-none", "platforms/x86_64-unknown-none/platform.toml"),
        ("armv7m-unknown-none", "platforms/armv7m-unknown-none/platform.toml"),
        ("riscv32-unknown-none", "platforms/riscv32-unknown-none/platform.toml"),
        ("linux-x86_64-hosted", "runtime/linux-x86_64-hosted.platform.toml"),
    ] {
        let text = fs::read_to_string(root.join(path))
            .unwrap_or_else(|e| panic!("reading {name}: {e}"));
        let desc = parse_descriptor(&text).unwrap_or_else(|e| {
            panic!("parsing {name}: E{} {}", e.code, e.detail)
        });
        let desc = desc.unwrap_or_else(|| panic!("{name} must carry a descriptor"));
        let errors = validate(&desc, Some(&root));
        assert!(
            errors.is_empty(),
            "{name} descriptor must validate: {:?}",
            errors
        );
    }
}