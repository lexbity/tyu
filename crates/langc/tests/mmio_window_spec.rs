//! Golden pair: the x86 emulated MMIO window size is descriptor-sourced (P3,
//! D-7, FR-21). Compiling the same module against two compiled descriptors
//! with different emulated window sizes must change exactly one emitted
//! immediate — the bounds-compare bound — and nothing else.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::time::{SystemTime, UNIX_EPOCH};

use codegen_core::{FeatureSet, MmioWindowKind, MmioWindowSpec, Target};
use frontend::parse::Parser;
use langc::driver::emit_obj_driver;
use semantics::typecheck::ChecksMode;

const SOURCE: &[u8] = b"\
module MmioWin;\n\
register-map Scratch\n\
  0x00 A u32 rw volatile\n\
end;\n\
const scratch = Scratch @ 0x0;\n\
: main ( -- i64 )\n\
  &!scratch.A 42 as u32 !u32\n\
  0\n\
;\n\
export { main };\n\
end;\n";

fn atom(bytes: &[u8]) -> ir::Atom {
    ir::Atom::new(bytes).unwrap()
}

fn emulated_window(size: u32) -> [MmioWindowSpec; 1] {
    [MmioWindowSpec {
        id: 0,
        name: atom(b"mmio"),
        kind: MmioWindowKind::Emulated,
        base: None,
        size,
    }]
}

fn os_bytes(s: &OsStr) -> &[u8] {
    s.as_bytes()
}

/// Compile SOURCE against `windows` and return the emitted asm text.
fn compile_with_windows(windows: &[MmioWindowSpec]) -> String {
    let target = Target::X86_64UnknownNone;
    let out_dir = std::env::temp_dir().join(format!(
        "tyu-mmio-win-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out_dir).unwrap();
    let input = out_dir.join("MmioWin.tyu");
    fs::write(&input, SOURCE).unwrap();

    let module = Parser::new(SOURCE).parse_module_ast().unwrap();
    let input_bytes = os_bytes(input.as_os_str());
    let out_dir_bytes = os_bytes(out_dir.as_os_str());
    let status = emit_obj_driver(
        &module,
        SOURCE,
        &[],
        ChecksMode::All,
        false,
        false,
        out_dir_bytes,
        target,
        false,
        input_bytes,
        FeatureSet::all(),
        None,
        windows,
    );
    assert_eq!(status, 0, "object emission failed with {windows:?}");

    let asm = fs::read_to_string(out_dir.join("MmioWin.asm")).unwrap();
    let _ = fs::remove_dir_all(&out_dir);
    asm
}

#[test]
fn emulated_window_size_sources_bounds_compare() {
    // The driver holds large inline aggregates (ModuleAst, MemOut); run on an
    // 8 MB stack like the other golden tests.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(emulated_window_size_sources_bounds_compare_inner)
        .unwrap()
        .join()
        .unwrap();
}

fn emulated_window_size_sources_bounds_compare_inner() {
    let asm_64k = compile_with_windows(&emulated_window(0x10000));
    let asm_32k = compile_with_windows(&emulated_window(0x8000));

    // The bounds-compare bound is `size - width` (width 4 for a u32 store).
    assert!(
        asm_64k.contains("cmp rax, 65532"),
        "0x10000 window must bound at 65532, got:\n{asm_64k}"
    );
    assert!(
        asm_32k.contains("cmp rax, 32764"),
        "0x8000 window must bound at 32764, got:\n{asm_32k}"
    );

    // Every other emitted byte must be identical — the descriptor sources the
    // window size, nothing else. Two label families are normalized away:
    // `.ds_high_N` is a process-global counter (pre-existing design) and
    // `.mmio_ok_N` is backend-local; neither depends on the window size.
    let mut normalized_64k = normalize_labels(&asm_64k);
    normalized_64k = normalized_64k.replace("65532", "BOUND");
    let mut normalized_32k = normalize_labels(&asm_32k);
    normalized_32k = normalized_32k.replace("32764", "BOUND");
    assert_eq!(
        normalized_64k, normalized_32k,
        "the two window sizes must differ only in the bounds-compare bound"
    );
}

/// Replace `.ds_high_<digits>` / `.mmio_ok_<digits>` label suffixes with a
/// fixed token, so outputs from different label counters compare equal.
fn normalize_labels(asm: &str) -> String {
    let mut out = String::with_capacity(asm.len());
    let mut i = 0usize;
    let bytes = asm.as_bytes();
    while i < bytes.len() {
        // Detect a `.ds_high_` or `.mmio_ok_` label prefix.
        let rest = &bytes[i..];
        let prefix = if rest.starts_with(b".ds_high_") {
            Some(b".ds_high_".len())
        } else if rest.starts_with(b".mmio_ok_") {
            Some(b".mmio_ok_".len())
        } else {
            None
        };
        if let Some(plen) = prefix {
            out.push_str(&asm[i..i + plen]);
            i += plen;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            out.push('N');
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}