//! Platform-pack scaffold generation.

use crate::error::TyuError;
use codegen_core::Target;
use lmod::abi_hash::{compute_abi_hash, RUNTIME_ABI_VERSION};
use lmod::modinfo::MODINFO_VER;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::find_pack_manifest_path;

pub fn scaffold_platform_pack(root: &Path, name: &str) -> Result<PathBuf, TyuError> {
    if find_pack_manifest_path(root, name).is_some() {
        return Err(TyuError::Platform(format!(
            "platform pack '{}' already exists",
            name
        )));
    }

    let pack_root = root.join("platforms").join(name);
    if pack_root.exists() {
        return Err(TyuError::Platform(format!(
            "platform pack directory '{}' already exists",
            pack_root.display()
        )));
    }

    let metal_root = pack_root.join("metal");
    let glue_root = pack_root.join("glue");
    fs::create_dir_all(&metal_root)
        .map_err(|e| TyuError::Platform(format!("creating '{}': {}", metal_root.display(), e)))?;
    fs::create_dir_all(&glue_root)
        .map_err(|e| TyuError::Platform(format!("creating '{}': {}", glue_root.display(), e)))?;

    let target = Target::X86_64UnknownNone;
    let expected_abi_hash = compute_abi_hash(
        target.spec().calling_conv.arch_tag(),
        target.spec().slot_bytes,
        target.spec().word_bits,
        MODINFO_VER,
    );

    let manifest = scaffold_manifest(name, expected_abi_hash);
    fs::write(pack_root.join("platform.toml"), manifest).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            pack_root.join("platform.toml").display(),
            e
        ))
    })?;
    fs::write(metal_root.join("runtime.asm"), scaffold_runtime_asm()).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            metal_root.join("runtime.asm").display(),
            e
        ))
    })?;
    fs::write(metal_root.join("link.ld"), scaffold_linker_script()).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            metal_root.join("link.ld").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("gpio.def"), scaffold_glue_def("gpio")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("gpio.def").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("gpio.mod"), scaffold_glue_mod("gpio")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("gpio.mod").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("uart.def"), scaffold_glue_def("uart")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("uart.def").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("uart.mod"), scaffold_glue_mod("uart")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("uart.mod").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("time.def"), scaffold_glue_def("time")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("time.def").display(),
            e
        ))
    })?;
    fs::write(glue_root.join("time.mod"), scaffold_glue_mod("time")).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            glue_root.join("time.mod").display(),
            e
        ))
    })?;
    fs::write(pack_root.join("datasheet.md"), scaffold_datasheet(name)).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            pack_root.join("datasheet.md").display(),
            e
        ))
    })?;
    fs::write(pack_root.join("tier.md"), scaffold_tier(name)).map_err(|e| {
        TyuError::Platform(format!(
            "writing '{}': {}",
            pack_root.join("tier.md").display(),
            e
        ))
    })?;

    Ok(pack_root)
}

fn scaffold_manifest(name: &str, expected_abi_hash: u64) -> String {
    format!(
        r#"[platform]
name = "{name}"
compiler-interface = {compiler_interface}
description = "{name} scaffold pack"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "0x{expected_abi_hash:016x}"

[metal]
path = "metal"
startup = "runtime.asm"
linker = "link.ld"

[memory]
flash = {{ name = "FLASH", origin = 0x00000000, length = 0x00100000, exec = "xip" }}
sram = {{ name = "SRAM", origin = 0x20000000, length = 0x00010000 }}
ds_region = "SRAM"
ds_size = 0x4000

[deploy]
method = "qemu"
boot = "raw_vectors"

[secure_boot]
supported = false
encryption_implies_secure_boot = true

[test]
rung = "untested"
"#,
        name = name,
        compiler_interface = RUNTIME_ABI_VERSION,
        expected_abi_hash = expected_abi_hash,
    )
}

fn scaffold_runtime_asm() -> String {
    [
        "format ELF64",
        "entry __lang_start",
        "",
        "section '.text' executable",
        "public __lang_start",
        "public __lang_trap",
        "",
        "__lang_start:",
        "    ret",
        "",
        "__lang_trap:",
        "    ret",
        "",
        "section '.data' writeable",
        "public __lang_ds_base",
        "public __lang_ds_limit",
        "public __lang_ds_high",
        "public __lang_expected_abi_hash",
        "",
        "__lang_ds_base:",
        "    dq 0",
        "__lang_ds_limit:",
        "    dq 0",
        "__lang_ds_high:",
        "    dq 0",
        "__lang_expected_abi_hash:",
        "    dq 0",
    ]
    .join("\n")
        + "\n"
}

fn scaffold_linker_script() -> String {
    [
        "ENTRY(__lang_start)",
        "MEMORY",
        "{",
        "    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 0x00100000",
        "    SRAM (rwx) : ORIGIN = 0x20000000, LENGTH = 0x00010000",
        "}",
        "SECTIONS",
        "{",
        "    .text : { *(.text*) } > FLASH",
        "    .rodata : { *(.rodata*) } > FLASH",
        "    .data : { *(.data*) } > SRAM",
        "    .bss : { *(.bss*) *(COMMON) } > SRAM",
        "}",
    ]
    .join("\n")
        + "\n"
}

fn scaffold_glue_def(capability: &str) -> String {
    let entries = match capability {
        "gpio" => vec![
            ("platform.gpio.init", "(pin mode --)"),
            ("platform.gpio.write", "(pin bool --)"),
            ("platform.gpio.read", "(pin -- bool)"),
        ],
        "uart" => vec![
            ("platform.uart.init", "(baud --)"),
            ("platform.uart.tx", "(u8 --)"),
            ("platform.uart.rx", "(-- u8 ok)"),
        ],
        "time" => vec![
            ("platform.time.now_us", "(-- i64)"),
            ("platform.time.reboot", "(--)"),
        ],
        _ => Vec::new(),
    };
    let mut out = String::new();
    for (name, effect) in entries {
        let _ = writeln!(&mut out, ": {} {} performs {{mmio}} ;", name, effect);
    }
    out
}

fn scaffold_glue_mod(capability: &str) -> String {
    format!(
        "module platform.{capability};\n\
         : stub ( -- ) ;\n\
         export {{ stub }};\n\
         end;\n",
    )
}

fn scaffold_datasheet(name: &str) -> String {
    format!(
        "# {name}\n\n\
         Scaffold pack generated by `tyu platform new`.\n\
         \n\
         This pack is intentionally minimal and should be filled in with the\n\
         board-specific memory map, boot flow, and peripheral details.\n",
    )
}

fn scaffold_tier(name: &str) -> String {
    format!(
        "# TestRung\n\n\
         TestRung = untested\n\n\
         Pack: {name}\n",
    )
}
