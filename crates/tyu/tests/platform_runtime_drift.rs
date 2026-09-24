//! Drift gates for platform runtime copies that should stay synchronized with
//! the legacy target runtimes while the migration is in progress.
//!
//! The generic QEMU platform packs (`armv7m-unknown-none`, `riscv32-unknown-none`)
//! are intended to be byte-for-byte equivalent to the legacy `runtime/<triple>`
//! sources they were lifted from. An earlier gate only compared the native-stack
//! guard *block*, which let the rest of the files silently drift (stale
//! semihosting helpers, a duplicated `.modpack` definition, an out-of-date
//! concurrency unit, and a linker script missing `.text.init` ordering all
//! slipped through). This gate now enforces full-file equivalence so any future
//! divergence in an intentionally-equivalent file fails CI with the exact pair.
//!
//! RP2350 is deliberately *forked* (custom board) and is therefore not compared
//! here; its divergence is declared in its runtime header.

use std::fs;
use std::path::Path;

use tyu::test_helpers::workspace_root;

/// (legacy source, platform copy) pairs that MUST remain byte-identical.
const EQUIVALENT_FILES: &[(&str, &str)] = &[
    // armv7m-unknown-none generic pack
    (
        "runtime/armv7m-unknown-none/runtime.asm",
        "platforms/armv7m-unknown-none/metal/runtime.asm",
    ),
    (
        "runtime/armv7m-unknown-none/concurrency.asm",
        "platforms/armv7m-unknown-none/metal/concurrency.asm",
    ),
    (
        "runtime/armv7m-unknown-none/modload.asm",
        "platforms/armv7m-unknown-none/metal/modload.asm",
    ),
    (
        "runtime/armv7m-unknown-none/link.ld",
        "platforms/armv7m-unknown-none/metal/link.ld",
    ),
    (
        "runtime/include/semihosting-arm.s",
        "platforms/armv7m-unknown-none/include/semihosting-arm.s",
    ),
    // riscv32-unknown-none generic pack
    (
        "runtime/riscv32-unknown-none/runtime.asm",
        "platforms/riscv32-unknown-none/metal/runtime.asm",
    ),
    (
        "runtime/riscv32-unknown-none/concurrency.asm",
        "platforms/riscv32-unknown-none/metal/concurrency.asm",
    ),
    (
        "runtime/riscv32-unknown-none/modload.asm",
        "platforms/riscv32-unknown-none/metal/modload.asm",
    ),
    (
        "runtime/riscv32-unknown-none/link.ld",
        "platforms/riscv32-unknown-none/metal/link.ld",
    ),
    (
        "runtime/include/semihosting-riscv.s",
        "platforms/riscv32-unknown-none/include/semihosting-riscv.s",
    ),
    // x86_64-unknown-none generic pack (runtime + concurrency units were
    // lifted; dynamic_entry.asm DOES have a runtime/ counterpart — see the
    // armv7m/riscv32 entries above, and platforms/x86_64-unknown-none/metal/)
    (
        "runtime/x86_64-unknown-none/concurrency.asm",
        "platforms/x86_64-unknown-none/metal/concurrency.asm",
    ),
    (
        "runtime/x86_64-unknown-none/runtime.asm",
        "platforms/x86_64-unknown-none/metal/runtime.asm",
    ),
];

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read `{rel}`: {e}"))
}

#[test]
fn generic_platform_runtimes_match_legacy_byte_for_byte() {
    let root = workspace_root();
    let mut drifted = Vec::new();
    for (legacy, platform) in EQUIVALENT_FILES {
        if read(&root, legacy) != read(&root, platform) {
            drifted.push(format!("  {legacy}\n    != {platform}"));
        }
    }
    assert!(
        drifted.is_empty(),
        "generic platform runtime files drifted from their legacy sources \
         (sync the platform copy or, if the fork is intentional, remove the pair \
         from EQUIVALENT_FILES and document the divergence):\n{}",
        drifted.join("\n"),
    );
}

#[test]
fn generic_platform_runtimes_define_native_stack_guard() {
    let root = workspace_root();
    for triple in ["armv7m-unknown-none", "riscv32-unknown-none"] {
        let rt = read(&root, &format!("platforms/{triple}/metal/runtime.asm"));
        assert!(
            rt.contains("__lang_stack_limit"),
            "platform {triple} runtime must define __lang_stack_limit"
        );
        assert!(
            rt.contains("__stack_overflow"),
            "platform {triple} runtime must define __stack_overflow"
        );
    }
}

/// BUG-016: the x86_64 codegen bounds-checks MMIO offsets against
/// `MMIO_SIZE = 65536` (`crates/codegen-x86_64/src/mmio.rs`) and indexes
/// `__mmio_mem + rax`.  Every runtime the emitted code links against must
/// allocate at least that much, or an offset in `[allocation, 65535]`
/// reads/writes past the array with no trap.
fn mmio_mem_allocation(rt: &str) -> usize {
    let lines: Vec<&str> = rt.lines().collect();
    let idx = lines
        .iter()
        .position(|l| l.contains("__mmio_mem"))
        .unwrap_or_else(|| panic!("runtime must define __mmio_mem"));
    for line in &lines[idx..(idx + 4).min(lines.len())] {
        for (needle, offset) in [("rb ", 3usize), (".space ", 7)] {
            if let Some(pos) = line.find(needle) {
                return line[pos + offset..]
                    .trim()
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("unparseable allocation: {line:?}"));
            }
        }
    }
    panic!("no allocation found after __mmio_mem");
}

#[test]
fn x86_64_runtimes_allocate_mmio_mem_matching_codegen_bound() {
    const MMIO_SIZE: usize = 65536; // MMIO_SIZE in crates/codegen-x86_64/src/mmio.rs
    let root = workspace_root();
    for rel in [
        "runtime/x86_64-unknown-none/runtime.asm",
        "platforms/x86_64-unknown-none/metal/runtime.asm",
        "runtime/linux-x86_64-hosted.asm",
    ] {
        let rt = read(&root, rel);
        assert_eq!(
            mmio_mem_allocation(&rt),
            MMIO_SIZE,
            "{rel}: __mmio_mem must match codegen MMIO_SIZE (BUG-016)"
        );
    }
}
