use codegen_core::{PlatformCapability, Target};
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

fn temp_dir(label: &str) -> PathBuf {
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

/// Assemble `runtime.asm` for the given target using FASM.
/// Returns the path to the produced `runtime.o`.
pub fn assemble_runtime(target: Target, out_dir: &Path) -> PathBuf {
    let rt_dir = runtime_dir(target);
    let asm = rt_dir.join("runtime.asm");
    let out = out_dir.join("runtime.o");

    let status = Command::new("fasm")
        .args([asm.to_str().unwrap(), out.to_str().unwrap()])
        .status()
        .expect("fasm invocation failed");
    assert!(status.success(), "fasm failed to assemble runtime");
    out
}

/// Link object files + runtime into an ELF using `ld` and the target's
/// linker script.  Returns the path to the output ELF.
pub fn link_image(target: Target, objs: &[PathBuf], out_dir: &Path) -> PathBuf {
    let rt_dir = runtime_dir(target);
    let linker_script = rt_dir.join("link.ld");
    let out = out_dir.join("test.elf");

    let mut cmd = Command::new("ld");
    cmd.arg("-T").arg(&linker_script).arg("-o").arg(&out);
    for obj in objs {
        cmd.arg(obj);
    }

    let status = cmd.status().expect("ld invocation failed");
    assert!(status.success(), "ld failed to link test image");
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

/// Scan serial output for:
/// - `F` (0x46): a test failure was signalled
/// - `S` (0x53) followed by `\n`: all suites completed
///
/// Returns `(failure_count, completed)`.
pub fn parse_output(stdout: &[u8]) -> (usize, bool) {
    let mut failures = 0usize;
    let mut completed = false;
    let mut i = 0;
    while i < stdout.len() {
        match stdout[i] {
            b'F' => failures += 1,
            b'S' => {
                if stdout.get(i + 1) == Some(&b'\n') {
                    completed = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (failures, completed)
}
