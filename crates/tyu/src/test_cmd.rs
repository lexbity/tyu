//! `tyu test` subcommand — suite runner that builds, executes, and verifies
//! test images across one or many targets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use codegen_core::Target;

use crate::args::TestArgs;
use crate::build;
use crate::highwater::check_high_water;
use crate::manifest::{parse_manifest, FixtureEntry, Manifest};
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
        Target::ArmV7MUnknownNone => &["langc", "arm-none-eabi-as", "arm-none-eabi-ld", "qemu-system-arm"],
        Target::RiscV32UnknownNone => &["langc", "riscv64-unknown-elf-as", "riscv64-unknown-elf-ld", "qemu-system-riscv32"],
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
    let fixtures_dir = args.manifest_path.parent()
        .ok_or("manifest has no parent directory")?;

    // Filter by name if --filter given.
    let filtered: Vec<&FixtureEntry> = manifest.fixtures.iter()
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

    for &target in &targets {
        let triple = std::str::from_utf8(target.triple()).unwrap();

        // Check tool availability.
        let tools = required_tools(target);
        let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
        if !missing.is_empty() {
            eprintln!("tyu: target {} skipped — missing tools: {}", triple, missing.join(", "));
            continue;
        }

        // Filter fixtures by capability requirements.
        let target_caps: HashSet<&str> = target.spec().capabilities.iter()
            .map(|c| c.name())
            .collect();

        let eligible: Vec<&&FixtureEntry> = filtered.iter()
            .filter(|f| {
                f.requires.iter().all(|r| target_caps.contains(r.as_str()))
            })
            .collect();

        if eligible.is_empty() {
            eprintln!("tyu: target {} — no eligible fixtures", triple);
            continue;
        }

        eprintln!("tyu: testing target {} ({} suites)", triple, eligible.len());

        // Build and run each suite.
        for fixture in eligible {
            let fixture_path = fixtures_dir.join(&fixture.file);
            if !fixture_path.exists() {
                eprintln!("tyu: fixture '{}' not found at '{}'", fixture.name, fixture_path.display());
                any_failure = true;
                continue;
            }

            match run_single_suite(fixture, target, fixtures_dir) {
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
) -> Result<(), String> {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let out_dir = std::env::temp_dir()
        .join("tyu_test")
        .join(format!("{}_{}_{}", triple, fixture.name, std::process::id()));

    // Build langc first.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
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

    // Compile the fixture as lib.
    let mut objs: Vec<PathBuf> = Vec::new();

    let fixture_path = fixtures_dir.join(&fixture.file);
    let fixture_o = compile_mod(target, &fixture_path, &out_dir, true)?;
    objs.push(fixture_o);

    // Compile the runner.
    let runner_o = compile_mod(target, &runner_path, &out_dir, false)?;
    objs.push(runner_o);

    // Assemble runtime.
    let runtime_o = build::assemble_runtime(target, &out_dir)?;
    objs.push(runtime_o);

    // Link.
    let image = build::link_image(target, &objs, &out_dir)?;

    // Determine runner.
    let runner = Runner::for_target(target);

    // Run with timeout.
    let timeout = Duration::from_secs(10);
    let outcome = runner.run(&image, timeout)?;

    if outcome.timed_out {
        return Err(format!("HANG (timed out after {:?})", timeout));
    }

    // Parse output.
    let summary = harness_core::parse_output(&outcome.stdout);

    if !summary.completed {
        let exit = outcome.exit_code;
        return Err(format!(
            "NO_COMPLETION — exited with code {} but no `S\\n` marker in output",
            exit,
        ));
    }

    if summary.failures > 0 {
        return Err(format!(
            "FAIL_MARKER — {} failure(s) reported via 'F' bytes",
            summary.failures,
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

    // High-water check.
    if summary.high_slots > 0 {
        check_high_water(summary.high_slots, &image)?;
    }

    Ok(())
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
    out.push_str("  83 testio.write-byte\n");  // 'S'
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
fn compile_mod(target: Target, src: &Path, out_dir: &Path, is_lib: bool) -> Result<PathBuf, String> {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();
    let sysroot = workspace.join("sysroot");
    let langc = workspace.join("target").join("debug").join("langc");

    let mut cmd = Command::new(&langc);
    cmd.arg("--emit=obj");
    cmd.arg(format!("--target={}", triple));
    cmd.arg(format!("--sysroot={}", sysroot.display()));
    cmd.arg(format!("--out-dir={}", out_dir.display()));
    cmd.arg("-I");
    cmd.arg(fixtures_dir());
    if is_lib {
        cmd.arg("--lib");
    }
    cmd.arg(src);

    let status = cmd.status()
        .map_err(|e| format!("running langc: {}", e))?;
    if !status.success() {
        return Err(format!("langc failed on '{}'", src.display()));
    }

    // Find the produced .o file.
    let obj_name = src.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let obj_path = out_dir.join(format!("{}.o", obj_name));
    if !obj_path.exists() {
        return Err(format!(".o not produced at '{}'", obj_path.display()));
    }
    Ok(obj_path)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
}

fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
