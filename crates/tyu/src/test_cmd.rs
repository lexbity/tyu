//! `tyu test` subcommand — suite runner that builds, executes, and verifies
//! test images across one or many targets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use codegen_core::{FeatureSet, Target};

use crate::args::TestArgs;
use crate::build;
use crate::highwater::check_stack_witness;
use crate::manifest::{parse_manifest, FixtureEntry, PoisonExpectation};
use crate::runner::Runner;

/// All known targets for `--all-targets`.
const ALL_TARGETS: &[Target] = &[
    Target::X86_64UnknownLinuxGnu,
    Target::X86_64UnknownNone,
    Target::ArmV7MUnknownNone,
    Target::RiscV32UnknownNone,
];

/// Tool names required per target.
fn required_tools(target: Target) -> &'static [&'static str] {
    match target {
        Target::X86_64UnknownLinuxGnu => &["langc"],
        Target::X86_64UnknownNone => &["langc", "fasm", "ld", "qemu-system-x86_64"],
        Target::ArmV7MUnknownNone => {
            &["langc", "arm-none-eabi-as", "arm-none-eabi-ld", "qemu-system-arm"]
        }
        Target::RiscV32UnknownNone => {
            &["langc", "riscv64-unknown-elf-as", "riscv64-unknown-elf-ld", "qemu-system-riscv32"]
        }
    }
}

/// Run the `test` subcommand.
pub fn run(args: &TestArgs) -> Result<(), String> {
    // Determine which targets to run on.
    let targets: Vec<Target> = if args.all_targets {
        ALL_TARGETS.to_vec()
    } else {
        vec![args.target]
    };

    // Read manifest.
    let manifest = parse_manifest(&args.manifest_path)?;
    let fixtures_dir = args
        .manifest_path
        .parent()
        .ok_or("manifest has no parent directory")?;

    // Filter by name if --filter given.
    let filtered: Vec<&FixtureEntry> = manifest
        .fixtures
        .iter()
        .filter(|f| {
            if let Some(ref pat) = args.filter {
                f.name.contains(pat.as_str())
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        eprintln!("tyu: no matching fixtures");
        return Ok(());
    }

    let mut any_failure = false;
    let feature_set = args.feature_set;

    for &target in &targets {
        let triple = std::str::from_utf8(target.triple()).unwrap();

        // Check tool availability.
        let tools = required_tools(target);
        let missing: Vec<&str> = tools
            .iter()
            .filter(|t| !tool_available(t))
            .copied()
            .collect();
        if !missing.is_empty() {
            if std::env::var("CI").is_ok() {
                return Err(format!(
                    "tyu: target {} — missing required tools under CI: {}. \
                     Install them or add them to PATH.",
                    triple,
                    missing.join(", "),
                ));
            }
            eprintln!(
                "tyu: target {} skipped — missing tools: {}",
                triple,
                missing.join(", "),
            );
            continue;
        }

        // Filter fixtures by capability requirements.
        let target_caps: HashSet<&str> = target
            .spec()
            .capabilities
            .iter()
            .map(|c| c.name())
            .collect();

        let eligible: Vec<&&FixtureEntry> = filtered
            .iter()
            .filter(|f| f.requires.iter().all(|r| target_caps.contains(r.as_str())))
            .collect();

        if eligible.is_empty() {
            eprintln!("tyu: target {} — no eligible fixtures", triple);
            continue;
        }

        eprintln!(
            "tyu: testing target {} ({} suites)",
            triple,
            eligible.len()
        );

        // Build and run each suite.
        for fixture in eligible {
            let fixture_path = fixtures_dir.join(&fixture.file);
            if !fixture_path.exists() {
                eprintln!(
                    "tyu: fixture '{}' not found at '{}'",
                    fixture.name,
                    fixture_path.display()
                );
                any_failure = true;
                continue;
            }

            let result = run_single_suite(fixture, target, fixtures_dir, feature_set);
            let result = poison_verdict(fixture, result);
            match result {
                Ok(()) => eprintln!("  {} ... ok", fixture.name),
                Err(e) => {
                    eprintln!("  {} ... FAILED: {}", fixture.name, e);
                    any_failure = true;
                }
            }
        }
    }

    if any_failure {
        Err("some tests failed".into())
    } else {
        Ok(())
    }
}

/// Build and run a single test suite (a set of fixtures).
fn run_single_suite(
    fixture: &FixtureEntry,
    target: Target,
    fixtures_dir: &Path,
    feature_set: FeatureSet,
) -> Result<(), String> {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let out_dir = std::env::temp_dir()
        .join("tyu_test")
        .join(format!("{}_{}_{}", triple, fixture.name, std::process::id()));

    // Build langc first.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace)
        .args(["build", "-q", "-p", "langc"])
        .status();

    // Generate test runner.
    let runner_src = generate_runner(&[fixture]);
    let runner_path = out_dir.join("test_runner.mod");
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("creating out_dir: {}", e))?;
    std::fs::write(&runner_path, &runner_src)
        .map_err(|e| format!("writing test_runner: {}", e))?;

    // Write a .def file for the fixture so the generated runner can import it.
    // langc does not emit .def files, so we write one from the fixture metadata.
    let mod_name = fixture_module_name(&fixture.name);
    let run_word = fixture_run_word(&fixture.name);
    let def_content = format!(
        "module {mod_name};\nexport {{ {run_word} }};\n: {run_word} ( -- ) ;\nend;\n"
    );
    let def_path = out_dir.join(format!("{}.def", mod_name));
    let _ = std::fs::write(&def_path, &def_content);

    // Compile the fixture as lib.
    let mut objs: Vec<PathBuf> = Vec::new();

    let fixture_path = fixtures_dir.join(&fixture.file);
    let fixture_o = compile_mod(target, &fixture_path, &out_dir, true, feature_set)?;
    objs.push(fixture_o.clone());

    // Compile the runner.
    let runner_o = compile_mod(target, &runner_path, &out_dir, false, feature_set)?;
    objs.push(runner_o);

    // Assemble runtime units.
    let runtime_objs = build::assemble_runtime(target, &out_dir, feature_set)?;
    objs.extend(runtime_objs);

    // Link.
    let image = build::link_image(target, &objs, &out_dir)?;

    // Determine runner.
    let runner = Runner::for_target(target);

    // Run with timeout.
    let timeout = Duration::from_secs(10);
    let outcome = runner.run(&image, timeout)?;

    if outcome.timed_out {
        let classify = crate::debug_escalate::classify_hang(&image, target);
        return Err(format!(
            "HANG (timed out after {:?})\n  {}",
            timeout, classify,
        ));
    }

    // Parse output.
    let summary = harness_core::parse_output(&outcome.stdout);

    // Try to decode any D diagnostic records from the output.
    let diag_text = decode_diags_from_stdout(&outcome.stdout, &fixture_o, Some(&fixture_path));

    // Build source map for escalation (if needed).
    let mut source_map = diag_core::render::SourceMap::new();
    source_map.add_file(&fixture_path);

    if !summary.completed {
        let exit = outcome.exit_code;

        // If no D records, try A-side escalation.
        let escalate_text = if diag_text.is_empty() && target.spec().qemu.is_some() {
            let esc = crate::debug_escalate::escalate(
                &image,
                target,
                Some((&fixture_path, &source_map)),
            );
            esc.diagnostic_string.or(esc.error)
        } else {
            None
        };

        let extra = escalate_text
            .or_else(|| {
                if diag_text.is_empty() {
                    None
                } else {
                    Some(diag_text)
                }
            })
            .map(|t| format!("\n{}", t))
            .unwrap_or_default();

        return Err(format!(
            "NO_COMPLETION — exited with code {} but no `S\\n` marker in output{}",
            exit, extra,
        ));
    }

    if summary.failures > 0 {
        let extra = if diag_text.is_empty() {
            String::new()
        } else {
            format!("\n{}", diag_text)
        };
        return Err(format!(
            "FAIL_MARKER — {} failure(s) reported via 'F' bytes{}",
            summary.failures, extra,
        ));
    }

    // Check QEMU/native exit code.
    let expected = match target.spec().qemu {
        Some(spec) => spec.exit_convention.host_pass_exit(),
        None => 0,
    };
    if outcome.exit_code != expected {
        return Err(format!(
            "EXIT_MISMATCH — exit code {} != expected {}",
            outcome.exit_code, expected,
        ));
    }

    // Assertion-count check.
    if let Some(expected) = fixture.expects {
        if summary.assertions == 0 {
            return Err(format!(
                "NO_ASSERTIONS: fixture '{}' declares expects={} but zero assertions were executed. \
                 The `P` record counter mechanism may not be wired.",
                fixture.name, expected,
            ));
        }
        if summary.assertions < expected {
            return Err(format!(
                "UNDERRAN: fixture '{}' declares expects={} but only {} assertions executed",
                fixture.name, expected, summary.assertions,
            ));
        }
        if summary.assertions > expected {
            // A fixture running extra assertions is a test-integrity concern.
            // We warn rather than fail to avoid brittleness.
            eprintln!(
                "tyu: fixture '{}' ran {} assertions (expects={})",
                fixture.name, summary.assertions, expected,
            );
        }
    }

    // Stack-bound witness check (generalized H/D witness).
    check_stack_witness(summary.high_slots, summary.diagnostics > 0, &image)?;

    Ok(())
}

/// Parse D records from serial output and attempt to decode them against
/// the fixture's `.lang.modinfo`.  Returns a formatted diagnostic string,
/// or empty if no D records are found or modinfo is unavailable.
///
/// `fixture_o` is the path to the compiled object file (for ELF section
/// reading).  `fixture_src` is the path to the source `.mod` file (for
/// source-context rendering); pass `None` to skip source context.
fn decode_diags_from_stdout(stdout: &[u8], fixture_o: &Path, fixture_src: Option<&Path>) -> String {
    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(stdout).collect();

    let d_records: Vec<&[u8]> = records
        .iter()
        .filter_map(|r| {
            if let harness_core::Record::Diag(payload) = r {
                Some(*payload)
            } else {
                None
            }
        })
        .collect();

    if d_records.is_empty() {
        return String::new();
    }

    // Try .lang.debug first (full coverage), fall back to .lang.modinfo.
    let elf_bytes = match std::fs::read(fixture_o) {
        Ok(b) => b,
        Err(_) => return format!("[{} D record(s) present but cannot read fixture]", d_records.len()),
    };

    // Read both sections from the ELF.
    let debug_bytes = read_elf_section_by_name(&elf_bytes, b".lang.debug");
    let modinfo_bytes = read_elf_section_by_name(&elf_bytes, b".lang.modinfo");

    let index = if let Some(ref dbg) = debug_bytes {
        match diag_core::decode::ModinfoIndex::from_debug_bytes(dbg) {
            Some(idx) => idx,
            None => return format!("[{} D record(s) present but .lang.debug is malformed]", d_records.len()),
        }
    } else if let Some(ref minfo) = modinfo_bytes {
        match diag_core::decode::ModinfoIndex::from_modinfo_bytes(minfo) {
            Some(idx) => idx,
            None => return format!("[{} D record(s) present but .lang.modinfo is malformed]", d_records.len()),
        }
    } else {
        return format!("[{} D record(s) present but no .lang.debug or .lang.modinfo in fixture]", d_records.len());
    };

    // Load the fixture source into a SourceMap for context.
    let mut source_map = diag_core::render::SourceMap::new();
    if let Some(src_path) = fixture_src {
        source_map.add_file(src_path);
    }

    let mut lines: Vec<String> = Vec::new();
    for payload in &d_records {
        match diag_core::DiagRecord::parse(payload) {
            Some(record) => {
                let diag = diag_core::decode::resolve(&record, &index);
                let rendered = diag.render(fixture_src, &source_map);
                lines.push(format!("  D: {}", rendered));
            }
            None => lines.push("  D: <malformed DiagRecord>".into()),
        }
    }
    lines.join("\n")
}

/// Adjust a suite result for poison fixtures.
///
/// For a normal (non-poison) fixture the result passes through unchanged.
/// For a poison fixture the verdict is inverted:
/// - Expected failure → pass (`Ok(())`).
/// - Clean run or unexpected failure → `Err("POISON_DID_NOT_FAIL")`.
fn poison_verdict(
    fixture: &FixtureEntry,
    run_result: Result<(), String>,
) -> Result<(), String> {
    let poison = match fixture.poison {
        Some(ref p) => p,
        None => return run_result,
    };

    match run_result {
        Ok(()) => Err("POISON_DID_NOT_FAIL — poison fixture completed without the expected failure".into()),
        Err(ref msg) => {
            let poison_occurred = match poison {
                PoisonExpectation::FailMarker => msg.contains("FAIL_MARKER"),
                PoisonExpectation::NoCompletion => {
                    msg.contains("NO_COMPLETION") || msg.contains("HANG")
                }
                PoisonExpectation::Trap(code) => {
                    let no_comp_or_hang =
                        msg.contains("NO_COMPLETION") || msg.contains("HANG");
                    if !no_comp_or_hang {
                        return Err(format!(
                            "POISON_DID_NOT_FAIL — expected trap:{} but got different failure: {}",
                            code, msg,
                        ));
                    }
                    if *code == 0 {
                        // Trap code 0 = accept any trap.
                        true
                    } else {
                        // On x86_64 the error message includes the exit code.
                        // Trap code appears as "code <N>" in the NO_COMPLETION message.
                        let code_str = format!("code {}", code);
                        msg.contains(&code_str)
                    }
                }
            };
            if poison_occurred {
                Ok(())
            } else {
                Err(format!(
                    "POISON_DID_NOT_FAIL — expected poison outcome did not occur: {}",
                    msg,
                ))
            }
        }
    }
}

/// Generate a test_runner.mod that imports the given fixture and calls its
/// test-run word, then emits S\n and returns 0.
fn generate_runner(fixtures: &[&FixtureEntry]) -> String {
    let mut out = String::from("module TestRunner;\n");
    for f in fixtures {
        let mod_name = fixture_module_name(&f.name);
        let word = fixture_run_word(&f.name);
        out.push_str(&format!("import {mod_name} {{ {word} }};\n"));
    }
    out.push_str("import platform/testio { testio.write-byte };\n");
    out.push('\n');
    out.push_str(": emit-done ( -- )\n");
    out.push_str("  83 testio.write-byte\n"); // 'S'
    out.push_str("  10 testio.write-byte ;\n"); // '\n'
    out.push('\n');
    out.push_str(": main ( -- i64 )\n");
    for f in fixtures {
        out.push_str(&format!("  {}\n", fixture_run_word(&f.name)));
    }
    out.push_str("  emit-done\n");
    out.push_str("  0 ;\n");
    out.push('\n');
    out.push_str("export { main };\n");
    out.push_str("end;\n");
    out
}

fn fixture_run_word(fixture: &str) -> String {
    format!("{}-run", fixture.replace('_', "-"))
}

fn fixture_module_name(fixture: &str) -> String {
    fixture
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

/// Compile a .mod file with langc.
fn compile_mod(
    target: Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
    feature_set: FeatureSet,
) -> Result<PathBuf, String> {
    let sysroot = workspace_root().join("sysroot");
    // Include both the standard fixtures dir AND the out_dir so that
    // the test runner can import fixtures compiled into the same output
    // directory (their .def files are produced there).
    let mut include_dirs = vec![fixtures_dir()];
    include_dirs.push(out_dir.to_path_buf());
    build::compile_simple(target, src, out_dir, is_lib, Some(&sysroot), &include_dirs, feature_set)
        .map_err(|e| e.to_string())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixtures_dir() -> PathBuf {
    workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
}

fn tool_available(name: &str) -> bool {
    crate::toolchain::find_in_path(name).is_some()
}

// ---------------------------------------------------------------------------
// ELF .lang.modinfo extraction
// ---------------------------------------------------------------------------

/// Read the contents of an ELF section by name (internal, no path lookup).
/// Returns `None` if the section is not found or the ELF is malformed.
pub(crate) fn read_elf_section_by_name_internal(data: &[u8], section_name: &[u8]) -> Option<Vec<u8>> {
    read_elf_section_by_name(data, section_name)
}

/// Read the contents of an ELF section by name.
/// Returns `None` if the section is not found or the ELF is malformed.
fn read_elf_section_by_name(data: &[u8], section_name: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 64 || &data[0..4] != b"\x7fELF" {
        return None;
    }
    let elf64 = data[4] == 2;
    let ehdr_size = if elf64 { 64usize } else { 52usize };
    if data.len() < ehdr_size { return None; }

    let (shoff, shentsz, shnum, shstrndx) = if elf64 {
        let shoff = u64::from_le_bytes(data[0x28..0x30].try_into().ok()?) as usize;
        let shentsz = u16::from_le_bytes(data[0x3a..0x3c].try_into().ok()?) as usize;
        let shnum = u16::from_le_bytes(data[0x3c..0x3e].try_into().ok()?) as usize;
        let shstrndx = u16::from_le_bytes(data[0x3e..0x40].try_into().ok()?) as usize;
        (shoff, shentsz, shnum, shstrndx)
    } else {
        let shoff = u32::from_le_bytes(data[0x20..0x24].try_into().ok()?) as usize;
        let shentsz = u16::from_le_bytes(data[0x2e..0x30].try_into().ok()?) as usize;
        let shnum = u16::from_le_bytes(data[0x30..0x32].try_into().ok()?) as usize;
        let shstrndx = u16::from_le_bytes(data[0x32..0x34].try_into().ok()?) as usize;
        (shoff, shentsz, shnum, shstrndx)
    };

    if shstrndx >= shnum || shentsz < 1 { return None; }

    let shstr_off = shoff + shstrndx * shentsz;
    if shstr_off + shentsz > data.len() { return None; }
    let (str_off, str_size) = if elf64 {
        let off = u64::from_le_bytes(data[shstr_off + 0x18..shstr_off + 0x20].try_into().ok()?) as usize;
        let sz = u64::from_le_bytes(data[shstr_off + 0x20..shstr_off + 0x28].try_into().ok()?) as usize;
        (off, sz)
    } else {
        let off = u32::from_le_bytes(data[shstr_off + 0x10..shstr_off + 0x14].try_into().ok()?) as usize;
        let sz = u32::from_le_bytes(data[shstr_off + 0x14..shstr_off + 0x18].try_into().ok()?) as usize;
        (off, sz)
    };
    if str_off + str_size > data.len() { return None; }
    let strtab = &data[str_off..str_off + str_size];

    for i in 0..shnum {
        let sh_off = shoff + i * shentsz;
        if sh_off + shentsz > data.len() { break; }
        let (name_off, sec_off, sec_size) = if elf64 {
            let no = u32::from_le_bytes(data[sh_off..sh_off + 4].try_into().ok()?) as usize;
            let so = u64::from_le_bytes(data[sh_off + 0x18..sh_off + 0x20].try_into().ok()?) as usize;
            let sz = u64::from_le_bytes(data[sh_off + 0x20..sh_off + 0x28].try_into().ok()?) as usize;
            (no, so, sz)
        } else {
            let no = u32::from_le_bytes(data[sh_off..sh_off + 4].try_into().ok()?) as usize;
            let so = u32::from_le_bytes(data[sh_off + 0x10..sh_off + 0x14].try_into().ok()?) as usize;
            let sz = u32::from_le_bytes(data[sh_off + 0x14..sh_off + 0x18].try_into().ok()?) as usize;
            (no, so, sz)
        };
        if name_off >= str_size { continue; }
        let name_end = strtab[name_off..].iter().position(|&b| b == 0).unwrap_or(str_size - name_off);
        let sec_name = &strtab[name_off..name_off + name_end];

        if sec_name == section_name {
            if sec_off + sec_size > data.len() { return None; }
            return Some(data[sec_off..sec_off + sec_size].to_vec());
        }
    }
    None
}
