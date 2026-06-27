//! Phase 18 QEMU oracle: static QEMU == default dynamic QEMU == host loader.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use hosted::loader::HostedLoaderPlatform;
use hosted::mem;
use loader_core::load::{load_module, LoadedSet};
use loader_core::symbols::SymMap;

const ORACLE_MOD: &str = "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";

fn build_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build langc tyu");
    assert!(status.success(), "cargo build langc tyu failed");
}

fn build_with_tyu(
    target: common::DynamicTarget,
    mode: Option<&str>,
    source_path: &Path,
    out_dir: &Path,
) {
    let sysroot = common::workspace_root().join("sysroot");
    let mut cmd = Command::new(common::tyu_exe());
    cmd.current_dir(common::workspace_root()).arg("build");
    if let Some(mode) = mode {
        cmd.arg(format!("--mode={mode}"));
    }
    cmd.args([
        format!("--target={}", target.triple),
        format!("--sysroot={}", sysroot.display()),
        format!("--out-dir={}", out_dir.display()),
        source_path.display().to_string(),
    ]);

    let output = cmd.output().expect("tyu build");
    assert!(
        output.status.success(),
        "{} tyu build mode={:?} failed:\nstdout:\n{}\nstderr:\n{}",
        target.triple,
        mode,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn host_loader_value(source: &str, dir: &Path) -> i64 {
    let source_path = dir.join("HostMain.mod");
    std::fs::write(&source_path, source).unwrap();
    let obj = common::langc_compile(
        codegen_core::Target::X86_64UnknownLinuxGnu,
        &source_path,
        dir,
        false,
    );
    let obj_bytes = std::fs::read(&obj).unwrap();
    let lmod_bytes = lmod_pack::pack(&obj_bytes).expect("pack host object");
    let container = lmod::validate::Container::parse(&lmod_bytes).unwrap();
    let abi_hash = lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER);
    let bsize =
        (container.code().len() + container.rodata().len() + container.data().len() + 4095) & !4095;
    let mut platform = HostedLoaderPlatform::new(abi_hash);
    platform.reserve(bsize).unwrap();
    let mut map: SymMap<'_, 256> = SymMap::new();
    let ds_high = allocate_runtime_page();
    register_host_runtime_symtab(&mut map, ds_high);
    let mut set = LoadedSet::<64>::new();
    load_module(&container, &mut platform, &mut map, &mut set).unwrap();

    let main = map.lookup_by_name(b"main").unwrap().addr;
    const DS_SIZE: usize = 65536;
    let ds_buf = vec![0u8; DS_SIZE];
    let ds_base = ds_buf.as_ptr() as u64;
    let ds_limit = ds_base + DS_SIZE as u64;
    let result: i64;
    unsafe {
        core::arch::asm!(
            "mov r15, {base}",
            "mov r14, {limit}",
            "call {main}",
            "mov {result}, rax",
            base = in(reg) ds_base,
            limit = in(reg) ds_limit,
            main = in(reg) main,
            result = lateout(reg) result,
            out("r15") _,
            out("r14") _,
            out("rax") _,
            out("rcx") _,
            out("rdx") _,
            out("rsi") _,
            out("rdi") _,
        );
    }
    result
}

fn register_host_runtime_symtab(map: &mut SymMap<'_, 256>, ds_high_addr: usize) {
    extern "C" fn runtime_stub() {}
    let stub = runtime_stub as *const () as usize;
    let entries = [
        (lmod::hash::fnv1a_u64(b"__stack_overflow"), stub),
        (lmod::hash::fnv1a_u64(b"__lang_ds_high"), ds_high_addr),
        (lmod::hash::fnv1a_u64(b"__lang_trap"), stub),
        (lmod::hash::fnv1a_u64(b"__lang_trap_loc"), stub),
    ];
    let mut bytes = Vec::with_capacity(8 + entries.len() * 16);
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for (hash, addr) in entries {
        bytes.extend_from_slice(&hash.to_le_bytes());
        bytes.extend_from_slice(&(addr as u64).to_le_bytes());
    }
    map.register_symtab_bytes(&bytes).unwrap();
}

fn allocate_runtime_page() -> usize {
    unsafe { mem::mmap_anon(4096, mem::prot::READ | mem::prot::WRITE).unwrap() as usize }
}

fn assert_qemu_equivalence(target: common::DynamicTarget) {
    if !common::require_tool_groups(target.tools) {
        return;
    }
    build_tools();

    let dir = common::temp_dir(&format!("phase18_qemu_equivalence_{}", target.triple));
    let source_path = dir.join("Main.mod");
    std::fs::write(&source_path, ORACLE_MOD).unwrap();
    assert_eq!(host_loader_value(ORACLE_MOD, &dir), 0);

    let static_out = dir.join("static");
    build_with_tyu(target, Some("static"), &source_path, &static_out);
    let static_artifact = static_out.join("Main.lmod");
    assert!(
        static_artifact.exists(),
        "{} static build must produce the deploy lmod",
        target.triple
    );
    let static_outcome = tyu::runner::Runner::for_target(target.target)
        .run_static_artifact(&static_artifact, Duration::from_secs(10))
        .expect("static product runner");
    assert!(
        !static_outcome.timed_out,
        "{} static QEMU run timed out",
        target.triple
    );

    let dynamic_out = dir.join("dynamic_default");
    build_with_tyu(target, None, &source_path, &dynamic_out);
    let dynamic_firmware = dynamic_out.join("image.elf");
    assert!(
        dynamic_firmware.exists(),
        "{} default build must produce dynamic firmware",
        target.triple
    );
    assert!(
        dynamic_out.join("modpack_generated.o").exists(),
        "{} dynamic firmware must embed a generated modpack",
        target.triple
    );
    let dynamic_outcome = tyu::runner::Runner::for_target(target.target)
        .run(&dynamic_firmware, Duration::from_secs(10))
        .expect("dynamic product runner");
    assert!(
        !dynamic_outcome.timed_out,
        "{} dynamic QEMU run timed out",
        target.triple
    );

    assert_eq!(
        dynamic_outcome.exit_code, static_outcome.exit_code,
        "{} dynamic/default QEMU exit must match static",
        target.triple
    );
    assert_eq!(
        dynamic_outcome.stdout, static_outcome.stdout,
        "{} dynamic/default QEMU output must match static",
        target.triple
    );
}

#[test]
fn dynamic_default_matches_static_and_host_loader_under_qemu() {
    for target in common::DYNAMIC_TARGETS {
        assert_qemu_equivalence(*target);
    }
}
