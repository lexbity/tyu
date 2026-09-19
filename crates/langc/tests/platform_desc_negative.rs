//! Negative: the `--platform` descriptor path rejects an invalid compiled
//! descriptor with E3647 (P3). A malformed `platform.desc` (aperture size 0, or
//! overlapping bus apertures) must be a loud compile-stop — the descriptor is
//! the board claim, and a bad one is never silently compiled around.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use codegen_core::compiled_desc::{
    CompiledDescriptor, COMPILED_DESC_MAX_BYTES, encode_compiled_desc,
};
use codegen_core::{MmioApertureKind, MmioApertureSpec};

fn atom(bytes: &[u8]) -> ir::Atom {
    ir::Atom::new(bytes).unwrap()
}

fn langc_exe() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // The langc binary lives at workspace target/debug/langc.
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/langc")
}

fn temp_platform(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-neg-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_desc(dir: &PathBuf, cd: &CompiledDescriptor) {
    let mut buf = [0u8; COMPILED_DESC_MAX_BYTES];
    let n = encode_compiled_desc(cd, &mut buf).unwrap();
    fs::write(dir.join("platform.desc"), &buf[..n]).unwrap();
}

fn run_langc(dir: &PathBuf) -> String {
    let out = std::env::temp_dir().join(format!(
        "tyu-langc-neg-out-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-none")
        .arg(format!("--platform={}", dir.display()))
        .arg(format!("--out-dir={}", out.display()))
        .arg("nonexistent.mod")
        .output()
        .expect("langc invocation");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&out);
    stderr
}

/// P6 (E3648): a register-map reference to a *bus* aperture with no absolute
/// base cannot be bound into a relocatable site — the reference is rejected.
#[test]
fn e3648_unbindable_bus_aperture() {
    let dir = temp_platform("unbindable");
    let mut cd = CompiledDescriptor::default();
    cd.aperture_count = 1;
    cd.apertures[0] = MmioApertureSpec {
        id: 0,
        name: atom(b"bus"),
        kind: MmioApertureKind::Bus,
        base: None,
        size: 0x10000,
        reloc_isa: Some(codegen_core::RelocIsa::ArmThumbLdrLiteral),
    };
    cd.device_count = 1;
    cd.devices[0] = codegen_core::compiled_desc::CompiledDevice {
        map: atom(b"BusMap"),
        instance: atom(b"bus"),
        aperture: 0,
        base_offset: 0,
        registers: [codegen_core::compiled_desc::CompiledRegister::EMPTY;
            codegen_core::compiled_desc::COMPILED_DESC_REGISTER_CAP],
        register_count: 1,
    };
    cd.devices[0].registers[0] = codegen_core::compiled_desc::CompiledRegister {
        offset: 0,
        name: atom(b"A"),
        width: 32,
        access: codegen_core::compiled_desc::REG_ACCESS_RW,
        write_kind: 0,
        read_kind: 0,
        atomic_max: 32,
        mask: 0,
        reset: 0,
        barrier: 0,
        interrupt: 0xFFFF,
        irq: 0xFFFF,
    };
    write_desc(&dir, &cd);
    fs::write(
        dir.join("Main.mod"),
        "module Main;\n\
         register-map BusMap\n\
           0x00 A u32 rw\n\
         end;\n\
         const bus = BusMap @ board.bus;\n\
         : read ( -- u32 ) &bus.A @u32 ;\n\
         : main ( -- i64 ) read drop 0 ;\n\
         end;\n",
    )
    .unwrap();
    let out = std::env::temp_dir().join(format!(
        "tyu-langc-e3648-out-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out).unwrap();
    let output = Command::new(langc_exe())
        .current_dir(&dir)
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-none")
        .arg(format!("--platform={}", dir.display()))
        .arg(format!("--out-dir={}", out.display()))
        .arg("Main.mod")
        .output()
        .expect("langc invocation");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&out);
    assert!(
        stderr.contains("E3648"),
        "bus aperture with no base must be E3648 (unbindable), got: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn size_zero_aperture_desc_is_e3647() {
    let dir = temp_platform("size-zero");
    let mut cd = CompiledDescriptor::default();
    cd.aperture_count = 1;
    cd.apertures[0] = MmioApertureSpec {
        id: 0,
        name: atom(b"mmio"),
        kind: MmioApertureKind::Emulated,
        base: None,
        size: 0,
        reloc_isa: None,
    };
    write_desc(&dir, &cd);

    let stderr = run_langc(&dir);
    assert!(
        stderr.contains("E3647") && stderr.contains("aperture size must be > 0"),
        "size-zero descriptor must be E3647, got: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overlapping_apertures_desc_is_e3647() {
    let dir = temp_platform("overlap");
    let mut cd = CompiledDescriptor::default();
    cd.aperture_count = 2;
    cd.apertures[0] = MmioApertureSpec {
        id: 0,
        name: atom(b"a"),
        kind: MmioApertureKind::Bus,
        base: Some(0x40000000),
        size: 0x1000,
        reloc_isa: Some(codegen_core::RelocIsa::ArmThumbLdrLiteral),
    };
    cd.apertures[1] = MmioApertureSpec {
        id: 1,
        name: atom(b"b"),
        kind: MmioApertureKind::Bus,
        base: Some(0x40000800),
        size: 0x1000,
        reloc_isa: Some(codegen_core::RelocIsa::ArmThumbLdrLiteral),
    };
    write_desc(&dir, &cd);

    let stderr = run_langc(&dir);
    assert!(
        stderr.contains("E3647") && stderr.contains("bus apertures overlap"),
        "overlapping descriptor must be E3647, got: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}