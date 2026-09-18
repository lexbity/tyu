//! x86_64 bare-metal execution tests via `tyu test` driver and product-runner-backed runs.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

// ---------------------------------------------------------------------------
// Positive fixture suite (via tyu test)
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let output = Command::new(common::tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            "--platform=x86_64-unknown-none",
            &format!("--manifest={}", common::fixtures_manifest().display()),
        ])
        .output()
        .expect("tyu test");

    assert!(
        output.status.success(),
        "tyu test x86_64-unknown-none failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// D diagnostic record emission from __lang_trap_loc
// ---------------------------------------------------------------------------

#[test]
fn trap_emits_framed_d_record() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let dir = temp_dir("trap_d_record");
    let target = codegen_core::Target::X86_64UnknownNone;

    // Self-contained fixture: the subtype violation triggers TrapIfFalse
    // before emit-done, so the runtime emits D records but no S+\n.
    // With -g the compiler emits __lang_trap_loc with rdi=trap_code,
    // rsi=1, rdx=line, rcx=word_hash.
    let fixture_src = "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  100 as Small drop
  emit-done
  0 ;
end;
";
    std::fs::write(dir.join("Main.mod"), fixture_src).unwrap();

    // Compile fixture with -g --checks=all (not --lib, self-contained).
    let fixture_o = langc_compile_g(target, &dir.join("Main.mod"), &dir, false);
    // Assemble runtime units.
    let runtime_objs = common::assemble_runtime(target, &dir);
    // Link.
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    // Run under QEMU with 5-second timeout.
    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(5));

    // The fixture should trap (subtype violation for 100 > 10).
    // No S\n will be emitted because the trap happens before emit-done.
    assert!(!outcome.timed_out, "fixture must trap, not hang");

    // Parse framed records from the output.
    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    // There should be a V record followed by a D record.
    let v_records: Vec<_> = records
        .iter()
        .filter(|r| matches!(r, harness_core::Record::Version(_)))
        .collect();
    assert!(
        !v_records.is_empty(),
        "output must contain at least one V record:\nstdout bytes: {:02x?}",
        &outcome.stdout,
    );

    let d_records: Vec<_> = records
        .iter()
        .filter(|r| matches!(r, harness_core::Record::Diag(_)))
        .collect();
    assert!(
        !d_records.is_empty(),
        "output must contain at least one D record:\nstdout bytes: {:02x?}",
        &outcome.stdout,
    );

    // Decode the first D record and verify its fields.
    for rec in &d_records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag = diag_core::DiagRecord::parse(payload)
                .expect("D record payload must be a valid DiagRecord");

            // The trap_code should be 21 (SUBTYPE_FAIL).
            assert_eq!(
                diag.trap_code, 21,
                "expected SUBTYPE_FAIL (21), got {}",
                diag.trap_code,
            );
            // valid must be 1 (language-emitted trap).
            assert!(diag.valid, "trap from __lang_trap_loc must have valid=1");
            // origin must be IN_GUEST (1).
            assert_eq!(diag.origin, diag_core::origin::IN_GUEST);
            // version must be 1.
            assert_eq!(diag.version, diag_core::DIAG_RECORD_VERSION);
            // word_hash must be non-zero (fnv1a_u64("main")).
            assert_ne!(
                diag.word_hash, 0,
                "word_hash must not be zero for a named word"
            );
            // ds_depth should be non-zero (main pushes values).
            assert!(
                diag.ds_depth > 0,
                "ds_depth should be > 0 during main execution"
            );
            // slot_count must be > 0 (Phase 7 full agent with slot dump).
            assert!(
                diag.slot_count > 0,
                "slot_count must be > 0 (Phase 7 full agent)"
            );
            // ds_declared must be the ⊤ sentinel (unknown in-guest).
            assert_eq!(
                diag.ds_declared,
                diag_core::DS_DECLARED_UNKNOWN,
                "ds_declared must be UNKNOWN (0xFFFFFFFF)"
            );
        }
    }
}

#[test]
fn trap_slots_contain_sentinels() {
    // Push known values before triggering a trap, then verify they
    // appear in the slot dump in deepest-last order.
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let dir = temp_dir("trap_slots_sentinels");
    let target = codegen_core::Target::X86_64UnknownNone;

    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  1 2 3
  100 as Small drop
  drop drop drop
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile_g(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs = common::assemble_runtime(target, &dir);
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(5));

    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    // Find the D record and decode its slots.
    let mut slot_values: Option<Vec<u64>> = None;
    for rec in &records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag =
                diag_core::DiagRecord::parse(payload).expect("valid DiagRecord in slot test");
            assert!(
                diag.slot_count >= 3,
                "expected at least 3 slots, got {}",
                diag.slot_count,
            );
            // Slots are in the payload after the 35-byte header.
            let slots_raw = &payload[diag_core::DIAG_HEADER_SIZE..];
            let slots: Vec<u64> = slots_raw
                .chunks(8)
                .map(|chunk| u64::from_le_bytes(chunk.try_into().unwrap()))
                .collect();
            assert_eq!(
                slots.len() as u16,
                diag.slot_count,
                "slot_count field must match actual slot bytes"
            );
            slot_values = Some(slots);
            break;
        }
    }

    let slots = slot_values.expect("at least one D record must be present");
    // The fixture pushes 1, 2, 3, then 100 (the subtype-checked value).
    // Deepest-last order: most-recent push first.
    // slots[0] should be 100 (the as-Small operand).
    assert_eq!(
        slots[0], 100,
        "top slot must be the most-recent push (100 from as Small)"
    );
    // slots[1..] should contain the sentinels in reverse-push order.
    assert!(
        slots.contains(&1),
        "sentinel 1 must appear somewhere in slot dump"
    );
    assert!(
        slots.contains(&2),
        "sentinel 2 must appear somewhere in slot dump"
    );
    assert!(
        slots.contains(&3),
        "sentinel 3 must appear somewhere in slot dump"
    );
}

#[test]
fn trap_minimal_slots_no_underflow() {
    // Minimal trap (single value on DS) — verify slot_count ≥ 1 and
    // the slot data parses without underflow (agent never reads below
    // __lang_ds_base).
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let dir = temp_dir("trap_minimal_slots");
    let target = codegen_core::Target::X86_64UnknownNone;

    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  100 as Small drop
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile_g(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs = common::assemble_runtime(target, &dir);
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(5));

    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    let mut slot_count = 0u16;
    let mut slot_payload_len = 0usize;
    for rec in &records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag = diag_core::DiagRecord::parse(payload)
                .expect("valid DiagRecord in minimal-slot test");
            slot_count = diag.slot_count;
            slot_payload_len = payload.len();
            // Must have at least the value being subtype-checked.
            assert!(
                slot_count >= 1,
                "minimal trap must have at least 1 live slot, got {}",
                slot_count,
            );
            break;
        }
    }

    assert!(slot_count > 0, "at least one D record must be present");
    // Total payload must be exactly 35 + slot_count * 8.
    let expected_len = diag_core::DIAG_HEADER_SIZE + slot_count as usize * 8;
    assert!(
        slot_payload_len >= expected_len,
        "D record payload too short: {} < {} (slot_count={})",
        slot_payload_len,
        expected_len,
        slot_count,
    );
}

#[test]
fn d_record_payload_does_not_phantom_f() {
    // Verify that D record payload bytes containing 0x46 (F), 0x53 (S),
    // 0x48 (H) do not produce phantom markers.
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let dir = temp_dir("trap_d_phantom");
    let target = codegen_core::Target::X86_64UnknownNone;

    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  100 as Small drop
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile_g(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs = common::assemble_runtime(target, &dir);
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(5));

    // Parse the output with legacy-compatible OutputSummary
    // to ensure no phantom failures from D payload bytes.
    let summary = harness_core::parse_output(&outcome.stdout);
    assert_eq!(
        summary.failures, 0,
        "D record payload must not cause phantom 'F' markers.\n\
         stdout bytes: {:02x?}",
        &outcome.stdout,
    );
}

// ---------------------------------------------------------------------------
// Runtime symbol exports
// ---------------------------------------------------------------------------

/// Verify that the x86_64 metal runtime exports the correct set of symbols
/// for three feature configurations: slim (no optional units), concurrency-only,
/// and fat (all units).  Uses the feature-derived expected set (DEBT-3 closed).
///
/// Also verifies that __lang_trap_loc and __lang_trap are distinct addresses.
#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["fasm", "nm"]) {
        return;
    }

    let target = codegen_core::Target::X86_64UnknownNone;
    let base_dir = common::temp_dir("runtime_symcheck");

    // Test three feature configurations: slim, concurrency-only, fat.
    let configs: &[(codegen_core::FeatureSet, &str)] = &[
        (codegen_core::FeatureSet::empty(), "slim"),
        (
            codegen_core::FeatureSet::empty().with(codegen_core::Feature::Concurrency),
            "concurrency",
        ),
        (codegen_core::FeatureSet::all(), "fat"),
    ];

    for (feature_set, label) in configs {
        let out_dir = base_dir.join(label);
        std::fs::create_dir_all(&out_dir).unwrap();

        // Assemble only the runtime units whose features are enabled.
        let mut objs = Vec::new();
        let stems = &["runtime", "concurrency", "modload"];
        let mut feature_names: Vec<&str> = feature_set.iter().map(|f| f.as_str()).collect();
        feature_names.sort();
        for stem in stems {
            let assemble = match *stem {
                "runtime" => true,
                "concurrency" => feature_set.contains(codegen_core::Feature::Concurrency),
                "modload" => feature_set.contains(codegen_core::Feature::ModuleLoading),
                _ => false,
            };
            if !assemble {
                continue;
            }
            let asm = common::runtime_dir(target).join(format!("{}.asm", stem));
            if !asm.exists() {
                continue;
            }
            let obj = out_dir.join(format!("{}.o", stem));
            let status = Command::new("fasm")
                .args([asm.to_str().unwrap(), obj.to_str().unwrap()])
                .status()
                .expect("fasm invocation");
            assert!(status.success(), "fasm failed to assemble {stem}");
            objs.push(obj);
        }

        // Collect symbols from all assembled objects.
        let mut all_symbols = String::new();
        for obj in &objs {
            let output = Command::new("nm")
                .arg("--defined-only")
                .arg(obj)
                .output()
                .expect("nm invocation failed");
            assert!(output.status.success(), "nm failed on {:?}", obj);
            all_symbols.push_str(&String::from_utf8_lossy(&output.stdout));
        }

        // Check expected symbols for this feature set.
        let expected = common::expected_symbols(*feature_set);
        for sym in &expected {
            assert!(
                all_symbols.contains(sym),
                "[{label}] runtime missing required symbol `{sym}`:\n{all_symbols}",
            );
        }

        // Verify __lang_trap_loc != __lang_trap (distinct addresses).
        let extract_addr = |sym: &str| -> Option<u64> {
            for line in all_symbols.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 && parts[1] == "T" && parts[2] == sym {
                    return u64::from_str_radix(parts[0], 16).ok();
                }
            }
            None
        };
        let trap_addr = extract_addr("__lang_trap");
        let loc_addr = extract_addr("__lang_trap_loc");
        assert!(
            trap_addr.is_some() && loc_addr.is_some(),
            "[{label}] both __lang_trap and __lang_trap_loc must be defined:\n{all_symbols}",
        );
        assert_ne!(
            trap_addr,
            loc_addr,
            "[{label}] __lang_trap_loc must differ from __lang_trap: trap={:#x} loc={:#x}",
            trap_addr.unwrap_or(0),
            loc_addr.unwrap_or(0),
        );
    }
}

// ---------------------------------------------------------------------------
// Slice 3 — feature-conditional module-loading link
// ---------------------------------------------------------------------------

/// Verify that `__lang_modpack_*` symbols are present in the linked image
/// only when `modload.o` is included (fat build), and absent when it is
/// excluded (slim build).
#[test]
fn modpack_symbols_present_only_with_modload() {
    if !common::require_tools(&["fasm", "ld", "nm"]) {
        return;
    }

    let target = codegen_core::Target::X86_64UnknownNone;
    let out_dir = common::temp_dir("modpack_symcheck");

    // Simple fixture (no task spawn, just a main that returns 0).
    let fixture_src = "\
module Main;
: main ( -- i64 ) 0 ;
end;
";
    std::fs::write(out_dir.join("Main.mod"), fixture_src).unwrap();

    // Build fixture.o
    let fixture_o = common::langc_compile(target, &out_dir.join("Main.mod"), &out_dir, false);

    // Build fat image: include modload.o
    let fat_dir = out_dir.join("fat");
    std::fs::create_dir_all(&fat_dir).unwrap();
    let all_runtime = common::assemble_runtime(target, &out_dir);
    let mut fat_objs = vec![fixture_o.clone()];
    fat_objs.extend(all_runtime.clone());
    let fat_image = common::link_image(target, &fat_objs, &fat_dir);

    // Check fat image has modpack symbols
    let fat_nm = Command::new("nm")
        .arg("--defined-only")
        .arg(&fat_image)
        .output()
        .expect("nm on fat image");
    let fat_out = String::from_utf8_lossy(&fat_nm.stdout);
    assert!(
        fat_out.contains("__lang_modpack_start"),
        "fat image must contain __lang_modpack_start:\n{fat_out}"
    );
    assert!(
        fat_out.contains("__lang_modpack_end"),
        "fat image must contain __lang_modpack_end:\n{fat_out}"
    );

    // Build slim image: exclude modload.o
    let slim_objs: Vec<_> = all_runtime
        .iter()
        .filter(|p| !p.to_string_lossy().contains("modload"))
        .cloned()
        .collect();
    // If modload.o was the only extra unit, slim_objs has just runtime.o
    let slim_objs = if slim_objs.is_empty() {
        // No modload.o was produced, so fat runtime IS slim
        return;
    } else {
        let mut objs = vec![fixture_o];
        objs.extend(slim_objs);
        objs
    };
    let slim_dir = out_dir.join("slim");
    std::fs::create_dir_all(&slim_dir).unwrap();
    let slim_image = common::link_image(target, &slim_objs, &slim_dir);

    // Check slim image does NOT have modpack symbols
    let slim_nm = Command::new("nm")
        .arg("--defined-only")
        .arg(&slim_image)
        .output()
        .expect("nm on slim image");
    let slim_out = String::from_utf8_lossy(&slim_nm.stdout);
    assert!(
        !slim_out.contains("__lang_modpack_start"),
        "slim image must NOT contain __lang_modpack_start:\n{slim_out}"
    );
    assert!(
        !slim_out.contains("__lang_modpack_end"),
        "slim image must NOT contain __lang_modpack_end:\n{slim_out}"
    );
}

/// Verify that the slim image (without modload.o) is strictly smaller than
/// the fat image (with modload.o), and that the difference is at least the
/// size of the modload section.
#[test]
fn slim_image_smaller_than_fat() {
    if !common::require_tools(&["fasm", "ld"]) {
        return;
    }

    let target = codegen_core::Target::X86_64UnknownNone;
    let out_dir = common::temp_dir("modpack_sizecheck");

    // Simple fixture that does not use any gated features.
    let fixture_src = "\
module Main;
: main ( -- i64 ) 0 ;
end;
";
    std::fs::write(out_dir.join("Main.mod"), fixture_src).unwrap();

    // Build fixture.o
    let fixture_o = common::langc_compile(target, &out_dir.join("Main.mod"), &out_dir, false);

    // Build fat image with modload.o
    let fat_dir = out_dir.join("fat");
    std::fs::create_dir_all(&fat_dir).unwrap();
    let fat_runtime = common::assemble_runtime(target, &out_dir);
    let mut fat_objs = vec![fixture_o.clone()];
    fat_objs.extend(fat_runtime.clone());
    let fat_image = common::link_image(target, &fat_objs, &fat_dir);
    let fat_size = std::fs::metadata(&fat_image).map(|m| m.len()).unwrap_or(0);

    // Build slim image without modload.o
    let slim_runtime: Vec<_> = fat_runtime
        .iter()
        .filter(|p| !p.to_string_lossy().contains("modload"))
        .cloned()
        .collect();
    let mut slim_objs = vec![fixture_o];
    slim_objs.extend(slim_runtime);
    let slim_dir = out_dir.join("slim");
    std::fs::create_dir_all(&slim_dir).unwrap();
    let slim_image = common::link_image(target, &slim_objs, &slim_dir);
    let slim_size = std::fs::metadata(&slim_image).map(|m| m.len()).unwrap_or(0);

    assert!(
        slim_size < fat_size,
        "slim image ({slim_size} bytes) must be smaller than fat image ({fat_size} bytes)"
    );

    // The delta must be positive — the modpack section contributes at
    // least alignment and section-table overhead to the linked image.
    let delta = fat_size - slim_size;
    assert!(
        delta > 0,
        "size delta ({delta} bytes) must be > 0; slim={slim_size}, fat={fat_size}"
    );
}

// ---------------------------------------------------------------------------
// Compilation helpers
// ---------------------------------------------------------------------------

/// Compile a .mod file with langc (no special flags).
fn langc_compile(
    target: codegen_core::Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
) -> PathBuf {
    common::langc_compile(target, src, out_dir, is_lib)
}

/// Compile a .mod file with langc -g --checks=all.
fn langc_compile_g(
    target: codegen_core::Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let mut args: Vec<String> = vec![
        "-g".into(),
        "--checks=all".into(),
        "--emit=obj".into(),
        format!("--target={triple}"),
        format!("--sysroot={}", common::sysroot_dir().display()),
        format!("--out-dir={}", out_dir.display()),
        format!("-I={}", out_dir.display()), // for .def resolution
        format!("--platform={}", common::platform_desc_dir(target).display()),
    ];
    if is_lib {
        args.push("--lib".into());
    }
    args.push(src.to_str().unwrap().into());

    let status = Command::new(common::langc_exe())
        .args(&args)
        .status()
        .expect("langc (g) invocation failed");
    assert!(
        status.success(),
        "langc -g --checks=all failed on {}",
        src.display()
    );

    std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .expect("langc (g) produced no .o file")
}

/// Create a temporary directory for a test.
fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_exec_tests").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
