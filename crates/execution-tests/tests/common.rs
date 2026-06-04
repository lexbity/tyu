use codegen_core::{AssemblerKind, PlatformCapability, Target};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

pub fn runtime_dir(target: Target) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    workspace_root().join("runtime").join(triple)
}

pub fn sysroot_dir() -> PathBuf {
    workspace_root().join("sysroot")
}

pub fn langc_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("langc")
}

pub fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_exec_tests")
        .join(format!("{}_{}", label, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Tool availability
// ---------------------------------------------------------------------------

/// Returns true if the named binary exists somewhere in PATH.
pub fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Environment-aware tool gating (S3 Phase 0).
///
/// Checks that all named tools are available.  In CI (`CI` env var set) a missing
/// tool is a **hard failure** — panics with the binary name.  Locally (no `CI`)
/// the test is skipped with an `eprintln`, preserving the fast inner loop.
///
/// Returns `true` when all tools are present (caller should run the test).
pub fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools
        .iter()
        .filter(|t| !tool_available(t))
        .copied()
        .collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!(
            "Required tools not available under CI: {}. \
             Install them or add them to PATH.",
            missing.join(", ")
        );
    }
    eprintln!(
        "SKIP: required tools not available ({})",
        missing.join(", ")
    );
    false
}

// ---------------------------------------------------------------------------
// Build steps
// ---------------------------------------------------------------------------

/// Compile a .mod source file with langc for the given target. Returns the
/// path to the produced .o file.
///
/// `is_lib`: pass `--lib` to skip the `main` requirement (library modules).
/// The output filename is determined by the module name declared inside the
/// source (not the source filename), so we scan out_dir for the new .o file
/// after compilation rather than predicting the name.
pub fn langc_compile(target: Target, src: &Path, out_dir: &Path, is_lib: bool) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();

    // Snapshot existing .o files before compilation so we can find the new one.
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        format!("--target={triple}"),
        format!("--sysroot={}", sysroot_dir().display()),
        format!("--out-dir={}", out_dir.display()),
        // Fixture .def files live in the fixtures dir; include it so
        // test_runner.mod can resolve imports like `import Arithmetic { ... }`.
        "-I".into(),
        fixtures_dir().to_str().unwrap().into(),
    ];
    if is_lib {
        args.push("--lib".into());
    }
    args.push(src.to_str().unwrap().into());

    let status = Command::new(langc_exe())
        .args(&args)
        .status()
        .expect("langc invocation failed");
    assert!(
        status.success(),
        "langc failed to compile {}",
        src.display()
    );

    // Find the .o file that was created (whichever is new in out_dir).
    std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p)
        })
        .next()
        .expect("langc produced no .o file")
}

/// Assemble `runtime.asm` for the given target.
/// Dispatches to the target-specific assembler via `TargetSpec.assembler`.
/// Returns the path to the produced `runtime.o`.
pub fn assemble_runtime(target: Target, out_dir: &Path) -> PathBuf {
    let spec = target.spec();
    let rt_dir = runtime_dir(target);
    let asm = rt_dir.join("runtime.asm");
    let out = out_dir.join("runtime.o");

    match spec.assembler {
        AssemblerKind::Fasm => {
            let status = Command::new("fasm")
                .args([asm.to_str().unwrap(), out.to_str().unwrap()])
                .status()
                .expect("fasm invocation failed");
            assert!(status.success(), "fasm failed to assemble runtime");
        }
        AssemblerKind::GasArm => {
            let status = Command::new("arm-none-eabi-as")
                .args([
                    "-mcpu=cortex-m3",
                    "-mthumb",
                    asm.to_str().unwrap(),
                    "-o",
                    out.to_str().unwrap(),
                ])
                .status()
                .expect("arm-none-eabi-as invocation failed");
            assert!(
                status.success(),
                "arm-none-eabi-as failed to assemble runtime"
            );
        }
        AssemblerKind::GasRiscV => {
            let status = Command::new("riscv64-unknown-elf-as")
                .args([
                    "-march=rv32i",
                    "-mabi=ilp32",
                    asm.to_str().unwrap(),
                    "-o",
                    out.to_str().unwrap(),
                ])
                .status()
                .expect("riscv64-unknown-elf-as invocation failed");
            assert!(
                status.success(),
                "riscv64-unknown-elf-as failed to assemble runtime"
            );
        }
    }
    out
}

/// Link object files + runtime into an ELF using the target's linker and
/// linker script.  Returns the path to the output ELF.
pub fn link_image(target: Target, objs: &[PathBuf], out_dir: &Path) -> PathBuf {
    let spec = target.spec();
    let rt_dir = runtime_dir(target);
    let linker_script = rt_dir.join("link.ld");
    let out = out_dir.join("test.elf");

    let linker = core::str::from_utf8(spec.linker).expect("non-UTF-8 linker name");
    let mut cmd = Command::new(linker);
    cmd.arg("-T").arg(&linker_script).arg("-o").arg(&out);
    for obj in objs {
        cmd.arg(obj);
    }

    let status = cmd.status().unwrap_or_else(|_| panic!("{linker} invocation failed"));
    assert!(status.success(), "{linker} failed to link test image");
    out
}

// ---------------------------------------------------------------------------
// Capability filtering
// ---------------------------------------------------------------------------

/// Parse a capability name from manifest.toml into a `PlatformCapability`.
#[allow(dead_code)]
pub fn parse_capability(s: &str) -> Option<PlatformCapability> {
    match s {
        "TaskScheduler" => Some(PlatformCapability::TaskScheduler),
        "DynamicAlloc" => Some(PlatformCapability::DynamicAlloc),
        "Channels" => Some(PlatformCapability::Channels),
        _ => None,
    }
}

/// Returns true if all `requires` entries are present in the target's
/// capability list.
#[allow(dead_code)]
pub fn target_has_capabilities(target: Target, requires: &[&str]) -> bool {
    let caps = target.spec().capabilities;
    requires.iter().all(|r| {
        parse_capability(r)
            .map(|c| caps.contains(&c))
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Test runner generation
// ---------------------------------------------------------------------------

/// Generate a `test_runner.mod` that imports and calls each named fixture's
/// test-runner word, then emits the `S\n` completion marker and returns 0.
///
/// Each fixture exports a word named `<fixture-id-with-hyphens>-run`.
/// For "arithmetic" → `arithmetic-run`; for "stack_ops" → `stack-ops-run`.
pub fn generate_test_runner(fixture_names: &[&str]) -> String {
    let mut out = String::from("module TestRunner;\n");
    for name in fixture_names {
        let module = fixture_module_name(name);
        let word = fixture_run_word(name);
        out.push_str(&format!("import {module} {{ {word} }};\n"));
    }
    out.push_str("import platform/testio { testio.write-byte };\n");
    out.push('\n');
    out.push_str(": emit-done ( -- )\n");
    out.push_str("  83 testio.write-byte\n");  // 'S'
    out.push_str("  10 testio.write-byte ;\n"); // '\n'
    out.push('\n');
    out.push_str(": main ( -- i64 )\n");
    for name in fixture_names {
        out.push_str(&format!("  {}\n", fixture_run_word(name)));
    }
    out.push_str("  emit-done\n");
    out.push_str("  0 ;\n");
    out.push('\n');
    out.push_str("export { main };\n");
    out.push_str("end;\n");
    out
}

/// Word name exported by a fixture: fixture id with underscores → hyphens, plus `-run`.
/// "arithmetic" → "arithmetic-run"; "stack_ops" → "stack-ops-run".
fn fixture_run_word(fixture: &str) -> String {
    format!("{}-run", fixture.replace('_', "-"))
}

fn fixture_module_name(fixture: &str) -> String {
    // Convert snake_case fixture name to PascalCase module name.
    // "arithmetic"  → "Arithmetic"
    // "stack_ops"   → "StackOps"
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

// ---------------------------------------------------------------------------
// High-level build
// ---------------------------------------------------------------------------

/// Compile all named fixtures + a generated test runner, assemble the runtime,
/// link everything, and return the path to the final ELF.
pub fn build_test_image(target: Target, fixture_names: &[&str]) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let out_dir = temp_dir(&format!("build_{triple}"));

    // Build cargo workspace first so langc is up to date
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let mut objs: Vec<PathBuf> = Vec::new();

    // Compile each fixture as a library (no main)
    for name in fixture_names {
        let src = fixtures_dir().join(format!("{name}.mod"));
        objs.push(langc_compile(target, &src, &out_dir, true));
    }

    // Generate and compile test runner (has main — not a lib)
    let runner_src = generate_test_runner(fixture_names);
    let runner_path = out_dir.join("test_runner.mod");
    std::fs::write(&runner_path, runner_src).unwrap();
    objs.push(langc_compile(target, &runner_path, &out_dir, false));

    // Assemble runtime
    objs.push(assemble_runtime(target, &out_dir));

    // Link
    link_image(target, &objs, &out_dir)
}

// ---------------------------------------------------------------------------
// QEMU execution
// ---------------------------------------------------------------------------

pub struct QemuResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
}

/// Run a test ELF under QEMU system mode using the target's `QemuSpec`.
/// Times out after 10 seconds.
pub fn qemu_run(target: Target, image: &Path) -> QemuResult {
    let spec = target
        .spec()
        .qemu
        .expect("target has no QemuSpec — cannot run under QEMU");
    let bin = std::str::from_utf8(spec.system_bin).unwrap();
    let machine = std::str::from_utf8(spec.machine).unwrap();

    let mut cmd = Command::new(bin);
    cmd.arg("-machine").arg(machine);
    for arg in spec.extra_args {
        cmd.arg(std::str::from_utf8(arg).unwrap());
    }
    // Semihosting targets need the QEMU semihosting backend enabled.
    match spec.exit_convention {
        codegen_core::QemuExitConvention::Semihosting => {
            cmd.arg("-semihosting-config");
            cmd.arg("enable=on,target=native");
        }
        codegen_core::QemuExitConvention::IsaDebugExit { .. } => {}
    }
    cmd.arg("-kernel").arg(image);

    // Use a child process with a timeout rather than output() directly
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn QEMU");

    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().expect("try_wait failed") {
            Some(status) => {
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut r| {
                        let mut buf = Vec::new();
                        use std::io::Read;
                        r.read_to_end(&mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                return QemuResult {
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                };
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    panic!("QEMU timed out after 10 seconds");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Serial output parsing
// ---------------------------------------------------------------------------

/// Parsed summary of QEMU serial output.
pub struct OutputSummary {
    pub failures: usize,
    pub completed: bool,
    /// Peak data-stack depth in slots, measured at runtime.
    /// 0 if no high-water marker was emitted.
    pub high_slots: u32,
}

/// Scan serial output for:
/// - `F` (0x46): a test failure was signalled
/// - `S` (0x53) followed by `\n`: all suites completed
/// - `H` (0x48) followed by u32-le: runtime high-water measurement (slots)
///
/// Returns an `OutputSummary`.
pub fn parse_output(stdout: &[u8]) -> OutputSummary {
    let mut failures = 0usize;
    let mut completed = false;
    let mut high_slots = 0u32;
    let mut i = 0;
    while i < stdout.len() {
        match stdout[i] {
            b'F' => failures += 1,
            b'S' => {
                if stdout.get(i + 1) == Some(&b'\n') {
                    completed = true;
                }
            }
            b'H' => {
                if i + 4 < stdout.len() {
                    high_slots = u32::from_le_bytes(
                        stdout[i + 1..i + 5].try_into().unwrap(),
                    );
                }
            }
            _ => {}
        }
        i += 1;
    }
    OutputSummary {
        failures,
        completed,
        high_slots,
    }
}

/// ⊤ sentinel for data-stack bound — means "no finite bound provable".
const TOP_SENTINEL: u32 = 0xFFFF_FFFF;

/// Assert that the runtime-measured high-water does not exceed the re-derived
/// conservative bound extracted from the ELF code bytes.
///
/// If the re-derived bound is `⊤` (`0xFFFF_FFFF`, no finite bound provable),
/// the check is skipped — the analysis already refused to certify it.
/// Otherwise, asserts `measured ≤ re-derived`.  A value exceeding the bound
/// is a hard failure (the static analysis or the re-derivation is unsound).
pub fn assert_high_water(measured: u32, elf_path: &Path, slot_bytes: u8) {
    let rederived = rederive_elf_high(elf_path, slot_bytes);
    if rederived == TOP_SENTINEL {
        return;
    }
    assert!(
        measured <= rederived,
        "high-water mismatch: runtime measured {} slots but re-derived \
         conservative bound is {} slots (measured > re-derived => analysis unsound)",
        measured,
        rederived,
    );
}

/// Re-derive a conservative data-stack high-water bound (in slots) from the
/// code section of an ELF (ELF64 or ELF32).
///
/// Dispatches to the architecture-specific byte-code scanner based on the
/// slot_bytes parameter (8 = x86_64, 4 = ARM Thumb).
pub fn rederive_elf_high(elf_path: &Path, slot_bytes: u8) -> u32 {
    let data = std::fs::read(elf_path).expect("failed to read ELF");

    assert!(data.len() >= 64, "ELF too small");
    assert_eq!(&data[0..4], b"\x7fELF", "not an ELF");

    let elf_class = data[4]; // 1 = ELF32, 2 = ELF64

    let (e_phoff, e_phentsize, e_phnum) = if elf_class == 2 {
        // ELF64 header layout
        let phoff = u64::from_le_bytes(data[0x20..0x28].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(data[0x36..0x38].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(data[0x38..0x3a].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    } else {
        // ELF32 header layout
        let phoff = u32::from_le_bytes(data[0x1c..0x20].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(data[0x2a..0x2c].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(data[0x2c..0x2e].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    };

    let mut total_high = 0u32;

    for i in 0..e_phnum {
        let off = e_phoff + i * e_phentsize;
        let phdr_size = if elf_class == 2 { 56usize } else { 32usize };
        if off + phdr_size > data.len() {
            break;
        }

        // p_type and p_flags are at the same offsets in both formats
        let p_type = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        if p_type != 1 { continue; } // PT_LOAD
        let p_flags =
            u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
        if p_flags & 1 == 0 { continue; } // PF_X

        let (p_offset, p_filesz) = if elf_class == 2 {
            let po = u64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap()) as usize;
            let sz = u64::from_le_bytes(data[off + 32..off + 40].try_into().unwrap()) as usize;
            (po, sz)
        } else {
            let po = u32::from_le_bytes(data[off + 12..off + 16].try_into().unwrap()) as usize;
            let sz = u32::from_le_bytes(data[off + 16..off + 20].try_into().unwrap()) as usize;
            (po, sz)
        };

        if p_offset + p_filesz > data.len() || p_filesz == 0 { continue; }

        let code = &data[p_offset..p_offset + p_filesz];
        let high = if slot_bytes == 8 {
            rederive_x86_64(code, slot_bytes as u32)
        } else {
            let arm = rederive_arm_thumb(code, slot_bytes as u32);
            if arm > 0 { arm } else { rederive_riscv(code, slot_bytes as u32) }
        };
        total_high = total_high.max(high);
    }

    total_high
}

/// Scan ARM Thumb code bytes for `adds r4, r4, #imm3` (16-bit: 0x1Cxx,
/// push, DS grows upward) and `subs r4, r4, #imm3` (16-bit: 0x1Exx,
/// pop), tracking running peak.
fn rederive_arm_thumb(code: &[u8], slot_bytes: u32) -> u32 {
    let mut sp_off: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;
    while i + 1 < code.len() {
        let w = u16::from_le_bytes([code[i], code[i + 1]]);
        if (w >> 11) == 0b00011 {
            let rd = (w & 0x7) as u32;
            let rn = ((w >> 4) & 0x7) as u32;
            let op_is_sub = ((w >> 10) & 1) as u32;
            if rd == rn && rd == 4 {
                let imm3 = ((w >> 7) & 0x7) as i64;
                if op_is_sub == 1 {
                    sp_off -= imm3; // pop
                } else {
                    sp_off += imm3; // push
                }
                if sp_off < 0 && slot_bytes > 0 {
                    let depth = (-sp_off as u32 + slot_bytes - 1) / slot_bytes;
                    peak = peak.max(depth);
                }
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    peak
}
/// Scan RISC-V RV32 code bytes for `addi s2, s2, imm12` (push) and
/// `addi s2, s2, -imm12` (pop), tracking running peak.
fn rederive_riscv(code: &[u8], slot_bytes: u32) -> u32 {
    let mut sp_off: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;
    while i + 3 < code.len() {
        let insn = u32::from_le_bytes(code[i..i + 4].try_into().unwrap());
        let opcode = insn & 0x7f;
        let rd = ((insn >> 7) & 0x1f) as u8;
        let funct3 = ((insn >> 12) & 0x7) as u8;
        let rs1 = ((insn >> 15) & 0x1f) as u8;
        // ADDI s2, s2, imm12: opcode=0x13, funct3=0, rd=18, rs1=18
        if opcode == 0x13 && funct3 == 0 && rd == 18 && rs1 == 18 {
            let imm12 = (insn >> 20) & 0xfff;
            let imm = (((imm12 as i32) << 20) >> 20) as i64;
            sp_off += imm;
            if sp_off < 0 {
                let depth = (-sp_off as u32 + slot_bytes - 1) / slot_bytes;
                peak = peak.max(depth);
            }
            i += 4;
            continue;
        }
        i += 1;
    }
    peak
}

/// Scan x86_64 code bytes for `add r15, imm8` (49 83 c7 XX — push, DS grows
/// upward) and `sub r15, imm8` (49 83 ef XX — pop), tracking running peak.
fn rederive_x86_64(code: &[u8], slot_bytes: u32) -> u32 {
    // Our data-stack grows UPWARD: r15 increases on push, decreases on pop.
    // offset tracks depth in bytes: positive = deeper (values on stack).
    let mut offset: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;
    while i < code.len() {
        let b = code[i];
        if i + 3 < code.len()
            && b == 0x49
            && code[i + 1] == 0x83
            && code[i + 2] == 0xc7
        {
            // add r15, imm8 — PUSH (DS grows upward)
            let imm = code[i + 3] as i8 as i64;
            offset += imm;
            peak = peak.max((offset / slot_bytes as i64) as u32);
            i += 4;
            continue;
        }
        if i + 3 < code.len()
            && b == 0x49
            && code[i + 1] == 0x83
            && code[i + 2] == 0xef
        {
            // sub r15, imm8 — POP (DS shrinks)
            let imm = code[i + 3] as i8 as i64;
            offset -= imm;
            if offset < 0 {
                offset = 0;
            }
            i += 4;
            continue;
        }
        i += 1;
    }
    peak
}
