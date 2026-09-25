use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use codegen_core::{FeatureSet, Target};
use frontend::parse::Parser;
use langc::driver::emit_obj_driver;
use semantics::typecheck::ChecksMode;

const FIXTURES: &[&str] = &["const_return", "arith_branch"];
const TARGETS: &[(Target, &str)] = &[
    (Target::X86_64UnknownNone, "x86_64-unknown-none"),
    (Target::ArmV7MUnknownNone, "armv7m-unknown-none"),
    (Target::RiscV32UnknownNone, "riscv32-unknown-none"),
];

#[test]
fn golden_codegen_asm_is_stable() {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(run_golden_codegen_asm_is_stable)
        .unwrap()
        .join()
        .unwrap();
}

fn run_golden_codegen_asm_is_stable() {
    for fixture in FIXTURES {
        for &(target, target_name) in TARGETS {
            let asm = compile_fixture(fixture, target);
            let baseline = baseline_path(fixture, target_name);
            if std::env::var_os("TYU_BLESS_GOLDENS").is_some() {
                fs::write(&baseline, &asm)
                    .unwrap_or_else(|err| panic!("failed to bless {}: {err}", baseline.display()));
                continue;
            }

            let expected = fs::read_to_string(&baseline).unwrap_or_else(|err| {
                panic!(
                    "missing golden baseline {}: {err}; rerun with TYU_BLESS_GOLDENS=1",
                    baseline.display()
                )
            });
            assert_eq!(
                expected, asm,
                "golden asm changed for fixture {fixture} on {target_name}"
            );
        }
    }
}

fn compile_fixture(fixture: &str, target: Target) -> String {
    let input = fixture_path(fixture);
    let src = fs::read(&input).unwrap_or_else(|err| panic!("read {}: {err}", input.display()));
    let module = Parser::new(&src)
        .parse_module_ast()
        .unwrap_or_else(|err| panic!("parse {}: {err:?}", input.display()));
    let out_dir = unique_out_dir(fixture, target.triple());
    fs::create_dir_all(&out_dir).unwrap_or_else(|err| panic!("mkdir {}: {err}", out_dir.display()));

    let search_dir = input.parent().unwrap();
    let search_dir_bytes = os_bytes(search_dir.as_os_str());
    let out_dir_bytes = os_bytes(out_dir.as_os_str());
    let input_bytes = os_bytes(input.as_os_str());
    let status = emit_obj_driver(
        &module,
        &src,
        &[search_dir_bytes],
        ChecksMode::All,
        false,
        false,
        out_dir_bytes,
        target,
        false,
        input_bytes,
        FeatureSet::all(),
        None,
        target.spec().mmio_apertures,
        false,
        None,
        false,
    );

    let asm_path = out_dir.join("Main.asm");
    let asm = fs::read_to_string(&asm_path)
        .unwrap_or_else(|err| panic!("read generated {}: {err}", asm_path.display()));

    if assembler_available(target) {
        assert_eq!(status, 0, "object emission failed for {}", input.display());
        let obj_path = out_dir.join("Main.o");
        assert!(
            obj_path.exists(),
            "object file missing at {}",
            obj_path.display()
        );
    }

    let _ = fs::remove_dir_all(&out_dir);
    asm
}

fn fixture_path(fixture: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/fixtures")
        .join(format!("{fixture}.tyu"))
}

fn baseline_path(fixture: &str, target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/baseline")
        .join(target)
        .join(format!("{fixture}.asm"))
}

fn unique_out_dir(fixture: &str, target: &[u8]) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let target = std::str::from_utf8(target).unwrap_or("target");
    std::env::temp_dir().join(format!(
        "tyu-golden-{fixture}-{target}-{}-{nanos}",
        std::process::id()
    ))
}

fn os_bytes(s: &OsStr) -> &[u8] {
    s.as_bytes()
}

fn assembler_available(target: Target) -> bool {
    match target {
        Target::X86_64UnknownNone => has_cmd("fasm") && has_cmd("ld"),
        Target::ArmV7MUnknownNone => has_cmd("arm-none-eabi-as"),
        Target::RiscV32UnknownNone => has_cmd("riscv32-unknown-elf-as"),
        Target::X86_64UnknownLinuxGnu => has_cmd("cc"),
    }
}

fn has_cmd(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths).find(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .is_some()
}
