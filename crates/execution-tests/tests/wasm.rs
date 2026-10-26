//! PLAN-WASM-2 B1: validate and execute the wasm backend against hosted runs.
//!
//! G54 uses the same fixture set as the interpreter/native differential in
//! `wasm-repl/tests/exec_parity.rs`. The native hosted runner is the oracle;
//! Wasmtime's `tyu` preloaded module captures `testio.write-byte` and maps
//! `testio.exit` to WASI process exit.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The executable G53 corpus. Keep this list aligned with
/// `wasm-repl/tests/exec_parity.rs::PARITY` (the oracle corpus is explicit).
const CORE_CORPUS: &[(&str, &str)] = &[
    ("arithmetic", "arithmetic-run"),
    ("deep_stack", "deep-stack-run"),
    ("locals_slices", "locals-slices-run"),
    // Keep each CFG family isolated as well as exercising the rest of the
    // fixture: conditional branches, a back edge, and a value-producing join.
    ("controlflow", "test-if"),
    ("controlflow", "test-while"),
    ("controlflow", "test-nested-branch"),
    ("call_abi", "call-abi-run"),
    ("ds_guard_deep", "ds-guard-run"),
    ("aliasing_probe1", "aliasing-probe1"),
    ("aliasing_probe2", "aliasing-probe2"),
    ("aliasing_probe3", "aliasing-probe3"),
    // A real region allocation exercises the backend's guarded linear-memory
    // Load/Store path in addition to the out-of-line allocator helpers.
    ("region_smoke", "region-smoke-run"),
];

const WASI_TYU_HOST: &str = r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (memory (export "memory") 1)
  (func (export "write_byte") (param $value i64)
    i32.const 32
    local.get $value
    i32.wrap_i64
    i32.store8
    i32.const 16
    i32.const 32
    i32.store
    i32.const 20
    i32.const 1
    i32.store
    i32.const 1
    i32.const 16
    i32.const 1
    i32.const 24
    call $fd_write
    drop)
  (func (export "trap") (param i64 i32 i32)
    unreachable)
  (func (export "exit") (param $value i64)
    local.get $value
    i32.wrap_i64
    call $proc_exit)
  (func (export "barrier") (param i32 i32))
  (func (export "periph_read") (param $ap i32) (param $off i32) (param $width i32) (result i64)
    local.get $ap
    i32.eqz
    if (result i64)
      local.get $width
      i32.const 1
      i32.eq
      if (result i64)
        local.get $off
        i32.load8_u
        i64.extend_i32_u
      else
        local.get $width
        i32.const 2
        i32.eq
        if (result i64)
          local.get $off
          i32.load16_u
          i64.extend_i32_u
        else
          local.get $width
          i32.const 4
          i32.eq
          if (result i64)
            local.get $off
            i32.load
            i64.extend_i32_u
          else
            local.get $off
            i64.load
          end
        end
      end
    else
      unreachable
    end)
  (func (export "periph_write") (param $ap i32) (param $off i32) (param $width i32) (param $value i64)
    local.get $ap
    i32.eqz
    if
      local.get $width
      i32.const 1
      i32.eq
      if
        local.get $off
        local.get $value
        i32.wrap_i64
        i32.store8
      else
        local.get $width
        i32.const 2
        i32.eq
        if
          local.get $off
          local.get $value
          i32.wrap_i64
          i32.store16
        else
          local.get $width
          i32.const 4
          i32.eq
          if
            local.get $off
            local.get $value
            i32.wrap_i64
            i32.store
          else
            local.get $off
            local.get $value
            i64.store
          end
        end
      end
    else
      unreachable
    end))"#;

fn root() -> PathBuf {
    common::workspace_root()
}

fn fixture(stem: &str) -> PathBuf {
    root()
        .join("crates/execution-tests/fixtures")
        .join(format!("{stem}.mod"))
}

fn require_tools() {
    let missing = ["wasm-tools", "wasmtime", "fasm", "ld"]
        .into_iter()
        .filter(|tool| {
            !Command::new("which")
                .arg(tool)
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "G54 requires the documented tools on PATH: {}",
        missing.join(", ")
    );
}

fn require_drivers() {
    for binary in ["langc", "tyu"] {
        let path = root().join("target/release").join(binary);
        assert!(
            path.is_file(),
            "G54 requires {}; run `cargo build -p langc -p tyu --release` first",
            path.display()
        );
    }
}

fn wrap_source(source: &str, run_word: &str) -> String {
    let mut wrapped = source.to_owned();
    if let Some(open) = wrapped.find("export {") {
        let close = wrapped[open..]
            .find('}')
            .expect("export declaration has a closing brace")
            + open;
        wrapped.insert_str(close, " main");
    } else {
        wrapped.insert_str(0, "export { main };\n");
    }
    let definition = format!("\n: main ( -- i64 )\n  {run_word}\n  0\n;\n");
    let end = wrapped
        .rfind("end;")
        .expect("fixture has a module terminator");
    wrapped.insert_str(end, &definition);
    wrapped
}

fn compile_wasm(source_path: &Path, module_name: &str, out_dir: &Path) -> PathBuf {
    fs::create_dir_all(out_dir).expect("create wasm output directory");
    let output = Command::new(root().join("target/release/langc"))
        .current_dir(root())
        .args([
            source_path.to_str().expect("UTF-8 source path"),
            "--emit=obj",
            "--target=wasm32-unknown-unknown",
            &format!(
                "--sysroot={}",
                root().join("sysroot/wasm32-unknown-unknown").display()
            ),
            &format!("--out-dir={}", out_dir.display()),
        ])
        .output()
        .expect("run langc for wasm");
    assert!(
        output.status.success(),
        "wasm compilation failed for {module_name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wasm = out_dir.join(format!("{module_name}.wasm"));
    assert!(wasm.is_file(), "langc did not create {}", wasm.display());
    wasm
}

fn compile_wasm_platform(source_path: &Path, module_name: &str, out_dir: &Path) -> PathBuf {
    fs::create_dir_all(out_dir).expect("create platform wasm output directory");
    let output = Command::new(root().join("target/release/langc"))
        .current_dir(root())
        .args([
            source_path.to_str().expect("UTF-8 source path"),
            "--emit=obj",
            "--target=wasm32-unknown-unknown",
            &format!(
                "--sysroot={}",
                root().join("sysroot/wasm32-unknown-unknown").display()
            ),
            &format!(
                "--platform={}",
                root().join("platforms/wasm32-unknown-unknown").display()
            ),
            &format!("--out-dir={}", out_dir.display()),
        ])
        .output()
        .expect("run langc for platform wasm");
    assert!(
        output.status.success(),
        "platform wasm compilation failed for {module_name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wasm = out_dir.join(format!("{module_name}.wasm"));
    assert!(wasm.is_file(), "langc did not create {}", wasm.display());
    wasm
}

fn validate(wasm: &Path) {
    let output = Command::new("wasm-tools")
        .args(["validate", "--features=mvp"])
        .arg(wasm)
        .output()
        .expect("run wasm-tools validate");
    assert!(
        output.status.success(),
        "wasm-tools rejected {}:\n{}",
        wasm.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn build_wasi_host(out_dir: &Path) -> PathBuf {
    let wat = out_dir.join("tyu-host.wat");
    let wasm = out_dir.join("tyu-host.wasm");
    fs::write(&wat, WASI_TYU_HOST).expect("write deterministic Tyu host module");
    let output = Command::new("wasm-tools")
        .args(["parse"])
        .arg(&wat)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("run wasm-tools parse for Tyu host module");
    assert!(
        output.status.success(),
        "wasm-tools could not assemble the G54 host module:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    wasm
}

fn wasmtime_run(wasm: &Path, host: &Path) -> Output {
    Command::new("timeout")
        .args(["30s", "wasmtime", "run", "--preload"])
        .arg(format!("tyu={}", host.display()))
        .args(["--invoke", "tyu_main"])
        .arg(wasm)
        .output()
        .expect("run compiled module in wasmtime")
}

fn native_run(wrapped_source: &Path, out_dir: &Path) -> (Vec<u8>, i32) {
    fs::create_dir_all(out_dir).expect("create native output directory");
    let mut command = Command::new(root().join("target/release/tyu"));
    command
        .current_dir(root())
        .args(["build"])
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(format!("--sysroot={}", root().join("sysroot").display()));
    let build = command
        .arg(wrapped_source)
        .output()
        .expect("run hosted tyu build");
    assert!(
        build.status.success(),
        "hosted build failed for {}:\n{}",
        wrapped_source.display(),
        String::from_utf8_lossy(&build.stderr)
    );
    let image = out_dir.join("image.elf");
    let run = Command::new("timeout")
        .arg("10s")
        .arg(&image)
        .output()
        .expect("run hosted native fixture under timeout");
    assert_ne!(
        run.status.code(),
        Some(124),
        "hosted native fixture timed out"
    );
    (run.stdout, run.status.code().unwrap_or(-1))
}

fn compile_and_run_fixture(
    stem: &str,
    run_word: &str,
    work: &Path,
    host: &Path,
) -> (Vec<u8>, i32, Vec<u8>, i32) {
    fs::create_dir_all(work).expect("create fixture work directory");
    let source = fs::read_to_string(fixture(stem)).expect("read G53 fixture");
    let wrapped = wrap_source(&source, run_word);
    let wrapped_path = work.join(format!("{stem}-wrapped.mod"));
    fs::write(&wrapped_path, wrapped).expect("write wrapped fixture");

    let (native_output, native_code) = native_run(&wrapped_path, &work.join("native"));
    let wasm = compile_wasm(&wrapped_path, module_name(&source), &work.join("wasm"));
    validate(&wasm);
    let wasm_run = wasmtime_run(&wasm, host);
    assert_ne!(
        wasm_run.status.code(),
        Some(124),
        "wasmtime timed out for {stem}"
    );
    assert!(
        wasm_run.status.success(),
        "wasmtime failed for {stem} (status {:?}):\n{}",
        wasm_run.status.code(),
        String::from_utf8_lossy(&wasm_run.stderr)
    );
    // `--invoke tyu_main` prints the i32 return value after guest stdout. Our
    // wrapper returns zero; remove exactly that framing suffix to recover the
    // byte stream emitted by `testio.write-byte`.
    let mut wasm_output = wasm_run.stdout;
    assert!(
        wasm_output.ends_with(b"0\n"),
        "wasmtime did not report the wrapper's zero return for {stem}: {:?}",
        String::from_utf8_lossy(&wasm_output)
    );
    wasm_output.truncate(wasm_output.len() - 2);
    (native_output, native_code, wasm_output, 0)
}

fn module_name(source: &str) -> &str {
    let rest = source
        .strip_prefix("module ")
        .or_else(|| source.split_once("\nmodule ").map(|(_, rest)| rest))
        .expect("source starts with a module declaration");
    rest.split([';', '\n']).next().expect("module name").trim()
}

#[test]
fn backend_emits_valid_wasm_for_the_g53_core_corpus() {
    require_tools();
    require_drivers();
    let work = common::temp_dir("wasm-g54-validate");
    for (stem, run_word) in CORE_CORPUS {
        let source = fs::read_to_string(fixture(stem)).expect("read G53 fixture");
        let wrapped_path = work.join(format!("{stem}-wrapped.mod"));
        fs::write(&wrapped_path, wrap_source(&source, run_word)).expect("write wrapped fixture");
        let wasm = compile_wasm(&wrapped_path, module_name(&source), &work.join(stem));
        validate(&wasm);
    }
}

#[test]
fn backend_matches_hosted_native_goldens_on_the_g53_core_corpus() {
    require_tools();
    require_drivers();
    let work = common::temp_dir("wasm-g54-parity");
    let host = build_wasi_host(&work);

    for (stem, run_word) in CORE_CORPUS {
        let (native_output, native_code, wasm_output, wasm_code) =
            compile_and_run_fixture(stem, run_word, &work.join(stem), &host);
        assert_eq!(
            wasm_output, native_output,
            "{stem}/{run_word}: wasm testio bytes differ from the hosted-native oracle"
        );
        assert_eq!(
            wasm_code, native_code,
            "{stem}/{run_word}: wasm exit code differs from the hosted-native oracle"
        );
    }
}

#[test]
fn backend_mmio_imports_match_register_map_fixture_checks() {
    require_tools();
    require_drivers();
    let work = common::temp_dir("wasm-g54-mmio");
    let host = build_wasi_host(&work);

    for (stem, run_word) in [
        ("mmio_smoke_x86", "mmio-smoke-x86-run"),
        ("mmio_strategies_x86", "mmio-strategies-x86-run"),
    ] {
        let source = fs::read_to_string(fixture(stem)).expect("read MMIO fixture");
        let wrapped = wrap_source(&source, run_word);
        let source_path = work.join(format!("{stem}-wrapped.mod"));
        fs::write(&source_path, wrapped).expect("write wrapped MMIO fixture");
        let wasm = compile_wasm_platform(&source_path, module_name(&source), &work.join(stem));
        validate(&wasm);
        let output = wasmtime_run(&wasm, &host);
        assert!(
            output.status.success(),
            "wasmtime MMIO run failed for {stem}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout, b"0\n",
            "{stem}: MMIO import semantics failed the fixture's register checks"
        );
    }
}

#[test]
fn exported_main_result_matches_hosted_native_exit_status() {
    require_tools();
    require_drivers();
    let work = common::temp_dir("wasm-g54-testio-exit");
    let source = work.join("WasmAddResult.mod");
    fs::write(
        &source,
        "module WasmAddResult;\n: main ( -- i64 ) 40 2 + ;\nexport { main };\nend;\n",
    )
    .expect("write arithmetic result fixture");
    let native = native_run(&source, &work.join("native"));
    let wasm = compile_wasm(&source, "WasmAddResult", &work.join("wasm"));
    validate(&wasm);
    let host = build_wasi_host(&work);
    let wasm = wasmtime_run(&wasm, &host);
    assert_ne!(wasm.status.code(), Some(124), "wasmtime timed out");
    assert!(native.0.is_empty(), "arithmetic fixture has no I/O");
    assert!(wasm.status.success(), "wasmtime invocation failed");
    let wasm_result = String::from_utf8(wasm.stdout)
        .expect("wasmtime result is UTF-8")
        .trim()
        .parse::<i32>()
        .expect("wasmtime prints the exported i32 result");
    assert_eq!(
        wasm_result, native.1,
        "exported main arithmetic result differs from hosted-native process status"
    );
}

#[test]
fn wasm_tools_rejects_malformed_backend_section_payloads() {
    require_tools();
    let work = common::temp_dir("wasm-g54-validator-negative");
    let wasm = work.join("malformed.wasm");
    const HEADER: &[u8] = b"\0asm\x01\0\0\0";

    // Exercise each standard section the backend emits (and its data section)
    // with a correctly framed section containing an invalid vector LEB. This
    // proves G54 consults the format validator, rather than merely checking
    // the executable modules happen to run.
    for section_id in [1u8, 2, 3, 5, 7, 10, 11] {
        let bytes = [HEADER, &[section_id, 1, 0xff]].concat();
        fs::write(&wasm, bytes).expect("write malformed section fixture");
        let result = Command::new("wasm-tools")
            .args(["validate", "--features=mvp"])
            .arg(&wasm)
            .output()
            .expect("run wasm-tools against malformed section");
        assert!(
            !result.status.success(),
            "wasm-tools unexpectedly accepted malformed section id {section_id}"
        );
    }
}

/// W7: `platform.io.log` (and `testio.write-str`) print on the wasm backend
/// through the emit_str helper — the same decoded bytes the hosted native
/// runtime prints. This is the book's Lab 1.1 flow at the artifact level.
#[test]
fn io_log_and_write_str_print_identically_on_both_substrates() {
    require_tools();
    require_drivers();
    let work = common::temp_dir("wasm-g54-io-log");
    let host = build_wasi_host(&work);

    let source = work.join("WasmIoLog.mod");
    fs::write(
        &source,
        "module WasmIoLog;\nimport platform/linux { platform.io.log };\n\n: main ( -- i64 )\n  \"assembling in tyu\\n\" platform.io.log\n  \"S\\n\" platform.io.log\n  0 ;\n\nexport { main };\nend;\n",
    )
    .expect("write io.log fixture");

    let native = native_run(&source, &work.join("native"));
    // The hosted runtime's diagnostic channel (io.log, testio.*) is STDERR
    // (fd 2, runtime/linux-x86_64-hosted.asm) — the wasm host's write_byte
    // mirrors it onto guest stdout, so the parity comparison is native's
    // stderr against the wasm run's stdout.
    let native_err = {
        let build = Command::new(root().join("target/release/tyu"))
            .current_dir(root())
            .args(["build"])
            .arg(format!("--out-dir={}", work.join("native2").display()))
            .arg(format!("--sysroot={}", root().join("sysroot").display()))
            .arg(&source)
            .output()
            .expect("run hosted tyu build");
        assert!(
            build.status.success(),
            "hosted build failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let run = Command::new("timeout")
            .args(["10s"])
            .arg(work.join("native2").join("image.elf"))
            .output()
            .expect("run hosted native fixture");
        run.stderr
    };
    assert_eq!(
        native_err,
        &b"assembling in tyu\nS\n"[..],
        "hosted native oracle prints the lab output on stderr"
    );
    assert_eq!(native.0, b"", "the lab prints nothing on native stdout");

    let wasm = compile_wasm(&source, "WasmIoLog", &work.join("wasm"));
    validate(&wasm);
    let run = wasmtime_run(&wasm, &host);
    assert_ne!(run.status.code(), Some(124), "wasmtime timed out");
    assert!(
        run.status.success(),
        "wasmtime failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let mut wasm_output = run.stdout;
    assert!(
        wasm_output.ends_with(b"0\n"),
        "wasmtime did not report main's zero return: {:?}",
        String::from_utf8_lossy(&wasm_output)
    );
    wasm_output.truncate(wasm_output.len() - 2);
    assert_eq!(
        wasm_output, native_err,
        "wasm io.log output differs from the hosted-native oracle"
    );

    // testio.write-str shares the helper: escapes decode identically.
    let source = work.join("WasmWriteStr.mod");
    fs::write(
        &source,
        "module WasmWriteStr;\nimport platform/testio { testio.write-str };\n\n: main ( -- i64 )\n  \"tab\\tline\\n\\\"q\\\"\\\\\" testio.write-str\n  0 ;\n\nexport { main };\nend;\n",
    )
    .expect("write write-str fixture");
    let _ = native_run(&source, &work.join("native2"));
    let native_err = {
        let run = Command::new("timeout")
            .args(["10s"])
            .arg(work.join("native2").join("image.elf"))
            .output()
            .expect("run hosted native fixture");
        run.stderr
    };
    let wasm = compile_wasm(&source, "WasmWriteStr", &work.join("wasm2"));
    validate(&wasm);
    let run = wasmtime_run(&wasm, &host);
    assert!(
        run.status.success(),
        "wasmtime failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let mut wasm_output = run.stdout;
    assert!(wasm_output.ends_with(b"0\n"), "missing zero return");
    wasm_output.truncate(wasm_output.len() - 2);
    // The oracle for `testio.write-str` is the DECODED string: the
    // interpreter's TestioWriteStr emits exactly the literal's bytes, and the
    // wasm emit_str helper must agree byte for byte. (The hosted-native
    // diagnostic channel dumps the whole intern record — the u64 length
    // prefix plus adjacent arena bytes — which is a native-side coarseness
    // the corpus never pinned; both REPL substrates emit the string itself.)
    assert_eq!(
        wasm_output, b"tab\tline\n\"q\"\\",
        "wasm write-str must emit the decoded literal bytes"
    );
}
