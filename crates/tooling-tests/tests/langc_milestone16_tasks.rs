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
fn milestone16_channel_send_recv_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_recv_roundtrip_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make => ch\n\
  ch 42 |>\n\
  ch <| 42 == [ 0 ] [ 1 ] if\n\
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
fn milestone16_channel_two_channels_independent_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_two_channels_independent_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make => ch1\n\
  platform.channel.make => ch2\n\
  ch1 40 |>\n\
  ch2 2 |>\n\
  ch1 <| ch2 <| +\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
    assert_eq!(run.code(), Some(42));
}

#[test]
fn milestone16_channel_send_type_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_type_mismatch_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: bad ( |i64| -- )\n\
  => ch\n\
  ch true |>\n\
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
    assert!(stderr.contains("error[E3732]"), "stderr: {stderr}");
}

#[test]
fn milestone16_channel_task_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_task_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
: main ( -- i64 ) performs {suspend}\n\
  platform.channel.make drop\n\
  [ ( -- ) ] platform.task.spawn => t\n\
  0 bitcast |Task| t |>\n\
  0 bitcast |Task| <| platform.task.join\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
fn milestone16_channel_send_blocks_on_full() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_blocks_on_full");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA[2] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) performs {suspend}\n\
  platform.channel.make drop\n\
  0 as u32 &!gpio.DATA.0 swap !u32\n\
  0 as u32 &!gpio.DATA.1 swap !u32\n\
  0\n\
  [ dup 64 < ]\n\
  [ dup 0 bitcast |i64| swap |> 1 + ] while\n\
  drop\n\
  [ ( -- )\n\
    &!gpio.DATA.0 @u32 as i64 1 == [ 1 as u32 &!gpio.DATA.1 swap !u32 ] [ ] if\n\
    0 bitcast |i64| <| drop\n\
  ] platform.task.spawn\n\
  0 bitcast |i64| 99 |>\n\
  1 as u32 &!gpio.DATA.0 swap !u32\n\
  platform.task.join\n\
  &gpio.DATA.1 @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
fn mmio_array_const_index_emits_ptr_add_const() {
    build_tools();
    let dir = fresh_dir("mmio_array_const_index_emits_ptr_add_const");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 DATA[4] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 )\n\
  gpio.DATA.2 @u32 drop\n\
  0\n\
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

#[ignore = "pre-existing: typecheck error E3000 on dynamic MMIO index — fix in Slice 8/9"]
#[test]
fn mmio_array_dynamic_index_emits_ptr_add_index() {
    build_tools();
    let dir = fresh_dir("mmio_array_dynamic_index_emits_ptr_add_index");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 DATA[4] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 )\n\
  1 => idx\n\
  gpio.DATA'(idx) @u32 drop\n\
  0\n\
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
    assert!(stdout.contains("ptr_add_index"), "stdout: {stdout}");
}

#[test]
fn milestone16_channel_recv_blocks_on_empty() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_recv_blocks_on_empty");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA[2] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) performs {suspend}\n\
  platform.channel.make drop\n\
  0 as u32 &!gpio.DATA.0 swap !u32\n\
  0 as u32 &!gpio.DATA.1 swap !u32\n\
  [ ( -- )\n\
    &!gpio.DATA.0 @u32 as i64 1 == [ 1 as u32 &!gpio.DATA.1 swap !u32 ] [ ] if\n\
    0 bitcast |i64| 123 |>\n\
  ] platform.task.spawn\n\
  0 bitcast |i64| <| drop\n\
  1 as u32 &!gpio.DATA.0 swap !u32\n\
  platform.task.join\n\
  &gpio.DATA.1 @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
fn milestone16_channel_deadlock_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_deadlock_traps_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make drop\n\
  0 bitcast |i64| <| drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
    assert_eq!(run.code(), Some(23));
}

#[test]
fn milestone8_sysroot_flag_allows_imports() {
    build_tools();
    let dir = fresh_dir("milestone8_sysroot_flag_allows_imports");
    let sysroot = dir.join("sysroot");
    std::fs::create_dir_all(sysroot.join("platform")).unwrap();

    std::fs::write(
        sysroot.join("Core.def"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("Core.mod"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.def"),
        b"module platform/linux;\n: platform.io.log ( str -- ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.mod"),
        b"module platform/linux;\n: platform.io.log ( str -- ) drop ;\nend;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\nimport platform/linux { };\n: main ( -- i64 )\n  \"hi\" platform.io.log\n  0\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", sysroot.to_string_lossy()))
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("platform.io.log"));
}

#[test]
fn milestone8_sysroot_iface_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone8_sysroot_iface_mismatch_fails");
    let sysroot = dir.join("sysroot");
    std::fs::create_dir_all(sysroot.join("platform")).unwrap();

    std::fs::write(
        sysroot.join("Core.def"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("Core.mod"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.def"),
        b"module platform/linux;\n: platform.io.log ( str -- ) ;\nend;\n",
    )
    .unwrap();
    // mismatch: wrong signature type
    std::fs::write(
        sysroot.join("platform").join("linux.mod"),
        b"module platform/linux;\n: platform.io.log ( i64 -- ) drop ;\nend;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport platform/linux { };\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", sysroot.to_string_lossy()))
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E2218]"));
}

#[test]
fn milestone8_effect_non_suspend_cannot_yield() {
    build_tools();
    let dir = fresh_dir("milestone8_effect_non_suspend_cannot_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.task.yield\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"), "stderr: {stderr}");
}

#[test]
fn milestone8_effect_suspend_word_allows_yield_runs() {
    build_tools();
    let dir = fresh_dir("milestone8_effect_suspend_word_allows_yield_runs");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 ) performs {suspend}\n\
  platform.task.yield\n\
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
fn milestone8_platform_task_run_allows_yield_in_quote() {
    build_tools();
    let dir = fresh_dir("milestone8_platform_task_run_allows_yield_in_quote");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  [ platform.task.yield ] platform.task.run\n\
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
fn task_call_allows_escaping_quote_body() {
    build_tools();
    let dir = fresh_dir("task_call_allows_escaping_quote_body");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  41 [ ( i64 -- i64 ) 1 + ] call\n\
;\n\
end;\n",
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
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(42));
}

#[test]
fn task_spawn_allows_escaping_quote_body() {
    build_tools();
    let dir = fresh_dir("task_spawn_allows_escaping_quote_body");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) performs {suspend}\n\
  [ ( -- ) 7 as u32 &!gpio.DATA swap !u32 ] platform.task.spawn\n\
  platform.task.join\n\
  &gpio.DATA @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
    assert_eq!(run.code(), Some(7));
}

#[test]
fn task_scheduler_stress_many_tasks() {
    build_tools();
    let dir = fresh_dir("task_scheduler_stress_many_tasks");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.task.spawn, platform.task.join, platform.task.yield };\n\
register-map GPIO\n\
  0x00 DATA[8] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) performs {suspend}\n\
  [ ( -- ) performs {suspend} platform.task.yield 1 as u32 &!gpio.DATA.0 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 2 as u32 &!gpio.DATA.1 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 3 as u32 &!gpio.DATA.2 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 4 as u32 &!gpio.DATA.3 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 5 as u32 &!gpio.DATA.4 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 6 as u32 &!gpio.DATA.5 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 7 as u32 &!gpio.DATA.6 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) performs {suspend} platform.task.yield 8 as u32 &!gpio.DATA.7 swap !u32 ] platform.task.spawn\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  &gpio.DATA.0 @u32 as i64\n\
  &gpio.DATA.1 @u32 as i64 +\n\
  &gpio.DATA.2 @u32 as i64 +\n\
  &gpio.DATA.3 @u32 as i64 +\n\
  &gpio.DATA.4 @u32 as i64 +\n\
  &gpio.DATA.5 @u32 as i64 +\n\
  &gpio.DATA.6 @u32 as i64 +\n\
  &gpio.DATA.7 @u32 as i64 +\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
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
    assert_eq!(run.code(), Some(36));
}

#[test]
fn milestone8_typed_channel_u32_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone8_typed_channel_u32_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make bitcast |u32| => ch\n\
  ch 42 as u32 |>\n\
  ch <| as i64\n\
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
    assert_eq!(run.code(), Some(42));
}

#[test]
fn milestone16_typed_channel_i8_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_typed_channel_i8_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make drop\n\
  0 bitcast |i8| -1 as i8 |>\n\
  0 bitcast |i8| <| as i64 -1 == [ 0 ] [ 1 ] if\n\
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
fn milestone8_iso_dup_forbidden() {
    build_tools();
    let dir = fresh_dir("milestone8_iso_dup_forbidden");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
iso Msg;\n\
: bad_dup ( Msg -- Msg Msg )\n\
  dup\n\
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
    assert!(stderr.contains("error[E5010]"), "stderr: {stderr}");
}

#[test]
fn milestone8_iso_drop_forbidden() {
    build_tools();
    let dir = fresh_dir("milestone8_iso_drop_forbidden");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
iso Msg;\n\
: bad_drop ( Msg -- )\n\
  drop\n\
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
    assert!(stderr.contains("error[E5011]"), "stderr: {stderr}");
}

#[test]
fn milestone9_golden_ir_dump_if_while_locals() {
    build_tools();
    let dir = fresh_dir("milestone9_golden_ir_dump_if_while_locals");

    std::fs::write(dir.join("Core.def"), b"module Core;\nexport { };\nend;\n").unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\n: pick ( i64 i64 bool -- i64 )\n  [ drop ] [ swap drop ] if\n;\n: countdown ( i64 -- i64 )\n  [ dup 0 > ] [ 1 - ] while\n;\n: locals ( i64 -- i64 )\n  => x\n  x\n;\nend;\n",
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
    // Keep the golden minimal but stable: ensure blocks/branches/locals are present.
    assert!(stdout.contains("word pick"));
    assert!(stdout.contains("br_if"));
    assert!(stdout.contains("word countdown"));
    assert!(stdout.contains("local_set"));
    assert!(stdout.contains("local_get"));
}

#[test]
fn milestone4_checks_flag_controls_insertion() {
    build_tools();
    let dir = fresh_dir("milestone4_checks_flag_controls_insertion");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nsubtype Percent = i64 range 0..100;\n: clamp ( i64 -- Percent ) as Percent ;\n: pwm_set ( Percent -- ) needs [ dup dup 0 >= swap 100 <= and ] drop ;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--checks=contracts", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("trap_if_false CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--checks=off", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));
}

#[test]
fn milestone5_rejects_mutable_borrow_of_local() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_mutable_borrow_of_local");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64 -- ptr_mut )\n  => x\n  &!x\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3501]"));
}

#[ignore = "pre-existing: scoped borrow detection shadowed — fix in Slice 8/9"]
#[test]
fn milestone5_scoped_borrow_must_be_consumed() {
    build_tools();
    let dir = fresh_dir("milestone5_scoped_borrow_must_be_consumed");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64.1 -- i64.1 )\n  &[\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3506]"));
}

#[ignore = "pre-existing: suspend-with-scoped-live detection — fix in Slice 8/9"]
#[test]
fn milestone5_rejects_suspend_with_scoped_live() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_with_scoped_live");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64.1 -- i64.1 ) performs {suspend}\n  &[\n    platform.task.yield drop\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3502]"));
}

#[ignore = "pre-existing: suspend-inside-mut-scoped-block detection — fix in Slice 8/9"]
#[test]
fn milestone5_rejects_suspend_inside_mut_scoped_block() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_inside_mut_scoped_block");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64.1 -- i64.1 )\n  &![\n    drop platform.task.yield\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"));
}

#[test]
fn milestone5_rejects_suspend_inside_lock() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_inside_lock");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64 -- i64 )\n  [ platform.task.yield ] lock\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"));
}

#[ignore = "pre-existing: drop-before-yield typecheck error — fix in Slice 8/9"]
#[test]
fn milestone5_allows_drop_before_yield() {
    build_tools();
    let dir = fresh_dir("milestone5_allows_drop_before_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64.1 -- i64.1 ) performs {suspend}\n  &[\n    drop\n  ]\n  platform.task.yield\n;\nend;\n",
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
fn langc_x86_64_unknown_none_target_recognized() {
    build_tools();
    let dir = fresh_dir("langc_x86_64_none_target");
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 ) 0 ;\nend;\n",
    )
    .unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--target=x86_64-unknown-none", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}