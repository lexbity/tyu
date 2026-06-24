use std::{path::PathBuf, process::Command, sync::Once};

static BUILD_ONCE: Once = Once::new();

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_tools() {
    BUILD_ONCE.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .current_dir(workspace_root())
            .args(["build", "-q"])
            .status()
            .expect("cargo build");
        assert!(status.success());
    });
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn runtime_asm_linux_x86_64_hosted() -> PathBuf {
    workspace_root()
        .join("runtime")
        .join("linux-x86_64-hosted.asm")
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_lang_tests").join(format!(
        "{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn repo_sysroot() -> PathBuf {
    workspace_root().join("sysroot")
}

#[test]
fn milestone10_golden_asm_add() {
    build_tools();
    let dir = fresh_dir("milestone10_golden_asm_add");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 )\n  1 2 +\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    fn norm(s: &str) -> String {
        let mut out = String::new();
        for part in s.split_inclusive('\n') {
            let (line, nl) = if let Some(stripped) = part.strip_suffix('\n') {
                (stripped, "\n")
            } else {
                (part, "")
            };
            out.push_str(line.trim_start());
            out.push_str(nl);
        }
        out
    }
    let expected = "\
format ELF64 executable\n\
entry __lang_start\n\
\n\
segment readable executable\n\
__lang_start:\n\
  mov r15, __lang_ds_base\n\
  mov r14, __lang_ds_limit\n\
  call w_1f5962a2ce9803c8\n\
  sub r15, 8\n\
  mov rdi, [r15]\n\
  and rdi, 0xff\n\
  mov rax, 60\n\
  syscall\n\
\n\
__lang_trap:\n\
  mov rax, 60\n\
  syscall\n\
\n\
__stack_overflow:\n\
  mov rdi, 10\n\
  jmp __lang_trap\n\
\n\
w_1f5962a2ce9803c8:\n\
  sub rsp, 16\n\
  jmp .b0_0\n\
.b0_0:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 1\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_0\n\
  mov [__lang_ds_high], r15\n\
.ds_high_0:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 2\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_1\n\
  mov [__lang_ds_high], r15\n\
.ds_high_1:\n\
  sub r15, 8\n\
  mov rcx, [r15]\n\
  sub r15, 8\n\
  mov rax, [r15]\n\
  add rax, rcx\n\
  mov [r15], rax\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_2\n\
  mov [__lang_ds_high], r15\n\
.ds_high_2:\n\
  sub r15, 8\n\
  mov rax, [r15]\n\
  mov [rsp+8], rax\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov rax, [rsp+8]\n\
  mov [r15], rax\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_3\n\
  mov [__lang_ds_high], r15\n\
.ds_high_3:\n\
  jmp .endword_0\n\
.endword_0:\n\
  add rsp, 16\n\
  ret\n\
\n\
segment readable writeable\n\
__lang_ds_base rb 65536\n\
__lang_ds_limit:\n\
__lang_ds_high dq 0\n";

    assert_eq!(norm(&stdout), norm(expected));
}

#[test]
fn milestone11_contract_failure_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_contract_failure_traps_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: bad ( -- ) needs [ false ] ;\n\
: main ( -- i64 )\n\
  bad\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--checks=contracts", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(20));
}

#[test]
fn milestone11_subtype_failure_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_subtype_failure_traps_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
subtype Percent = i64 range 0..100;\n\
: main ( -- i64 )\n\
  -1 as Percent\n\
  drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--checks=all", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(21));
}

#[test]
fn milestone11_stack_overflow_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_stack_overflow_traps_exit_code");

    std::fs::write(
        dir.join("Main.asm"),
        b"format ELF64 executable\n\
entry __lang_start\n\
\n\
segment readable executable\n\
__lang_start:\n\
  mov r15, __lang_ds_base\n\
  mov r14, __lang_ds_limit\n\
.loop:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 0\n\
  add r15, 8\n\
  jmp .loop\n\
\n\
__lang_trap:\n\
  mov rax, 60\n\
  syscall\n\
\n\
__stack_overflow:\n\
  mov rdi, 10\n\
  jmp __lang_trap\n\
\n\
segment readable writeable\n\
__lang_ds_base rb 65536\n\
__lang_ds_limit:\n",
    )
    .unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(10));
}

#[test]
fn milestone12_platform_io_log_writes_stderr() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_io_log_writes_stderr");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  \"hi\\n\" platform.io.log\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).output().unwrap();
    assert_eq!(run.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("hi\n"), "stderr: {stderr:?}");
}

#[test]
fn milestone12_platform_time_now_ms_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_time_now_ms_runs");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.time.now_ms drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone12_platform_critical_noop_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_critical_noop_runs");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.critical.enter\n\
  platform.critical.exit\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone12_platform_task_sleep_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_task_sleep_runs");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 ) performs {suspend}\n\
  0 as usize platform.task.sleep-ms\n\
  0 as usize platform.task.sleep-us\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone13_array_type_and_scoped_slice_typechecks() {
    build_tools();
    let dir = fresh_dir("milestone13_array_type_and_scoped_slice_typechecks");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64.4 -- i64.4 )\n\
  &[\n\
    drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("i64'4"));
    assert!(stdout.contains("Slice(i64)"));
    assert!(stdout.contains("scoped_enter"));
}

#[test]
fn milestone13_borrow_destructuring_emits_ptr_offsets() {
    build_tools();
    let dir = fresh_dir("milestone13_borrow_destructuring_emits_ptr_offsets");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i64\n\
  y : i64\n\
end;\n\
resource r : Point;\n\
: f ( -- )\n\
  r lock [\n\
    &r => { &x &y }\n\
    x drop\n\
    y drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ptr_add_const"), "stdout: {stdout}");
}

#[test]
fn milestone13_borrowed_slice_cannot_escape_via_return() {
    build_tools();
    let dir = fresh_dir("milestone13_borrowed_slice_cannot_escape_via_return");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64.4 -- Slice(i64) )\n\
  &[\n\
    swap drop\n\
    return\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5020]"), "stderr: {stderr}");
}

#[test]
fn milestone13_borrowed_slice_live_in_local_blocks_yield() {
    build_tools();
    let dir = fresh_dir("milestone13_borrowed_slice_live_in_local_blocks_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64.4 -- i64.4 ) performs {suspend}\n\
  &[\n\
    => s\n\
    platform.task.yield\n\
    s drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"), "stderr: {stderr}");
}

#[test]
fn milestone14_borrow_resource_outside_lock_fails() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_resource_outside_lock_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  &!counter drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5004]"), "stderr: {stderr}");
}

#[test]
fn milestone14_borrow_resource_inside_lock_ok() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_resource_inside_lock_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [ &!counter drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone14_shared_borrow_resource_inside_lock_ok() {
    build_tools();
    let dir = fresh_dir("milestone14_shared_borrow_resource_inside_lock_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [ &counter drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone14_borrow_other_resource_inside_lock_fails() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_other_resource_inside_lock_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
resource other : i64;\n\
: f ( -- )\n\
  counter [ &!other drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5004]"), "stderr: {stderr}");
}

#[test]
fn milestone14_nested_lock_rejected() {
    build_tools();
    let dir = fresh_dir("milestone14_nested_lock_rejected");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [\n\
    counter [ ] lock\n\
  ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5002]"), "stderr: {stderr}");
}

#[test]
fn milestone15_struct_field_borrow_and_load_ok() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_borrow_and_load_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
  y : i32\n\
end;\n\
: getx ( Point -- i32 )\n\
  => p\n\
  &p.x @i32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone15_struct_ptr_field_access_emits_ptr_add_const() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_ptr_field_access_emits_ptr_add_const");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
  y : i32\n\
end;\n\
: getx ( Point -- i32 )\n\
  => p\n\
  &p .x @i32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("addr_of") || stdout.contains("ptr_add_const"),
        "stdout: {stdout}"
    );
}

#[test]
fn milestone15_struct_field_unknown_field_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_unknown_field_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
end;\n\
: bad ( Point -- )\n\
  => p\n\
  &p.z drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3716]"), "stderr: {stderr}");
}

#[test]
fn milestone15_struct_field_load_type_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_load_type_mismatch_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
end;\n\
: bad ( Point -- bool )\n\
  => p\n\
  &p.x @bool\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3717]"), "stderr: {stderr}");
}

#[test]
fn milestone15_enum_variant_literal_ok() {
    build_tools();
    let dir = fresh_dir("milestone15_enum_variant_literal_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
enum State : u8\n\
  Idle = 0x00\n\
  Run  = 0x01\n\
end;\n\
: f ( -- State )\n\
  State.Idle\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone15_unknown_enum_variant_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_unknown_enum_variant_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
enum State : u8\n\
  Idle = 0\n\
end;\n\
: f ( -- State )\n\
  State.Missing\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3725]"), "stderr: {stderr}");
}
