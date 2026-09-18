//! The Executable-mode `__mmio_mem` BSS reservation MUST be sized from the
//! same descriptor-sourced emulated window as the bounds check (P3, FR-21;
//! platform-layer audit finding F1). A reservation smaller than the checked
//! bound would admit MMIO past the array into adjacent `.bss`.

use codegen_x86_64::X86_64HostedBackend;
use codegen_core::{AsmMode, CodegenBackend, CodegenError, MmioWindowKind, MmioWindowSpec};
use ir::{OpKind, Word, TY_I64};

mod util;
use util::*;

fn mmio_store_word() -> Word {
    single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::AddrOf {
                place: atom(b"x"),
                mutable: true,
                base: ir::AddrOfBase::Mmio {
                    window: 0,
                    offset: 0x10,
                },
            },
            OpKind::ConstI64(7),
            OpKind::Store { ty: TY_I64 },
            OpKind::ConstI64(0),
            OpKind::Ret,
        ],
    )
}

/// Emit an MMIO word + postlude against a single emulated window of `size`.
fn postlude_with_emulated(size: u32) -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = X86_64HostedBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
    backend
        .set_mmio_windows(&[MmioWindowSpec {
            id: 0,
            name: atom(b"mmio"),
            kind: MmioWindowKind::Emulated,
            base: None,
            size,
        }])
        .unwrap();
    backend.emit_word(&mmio_store_word()).unwrap();
    backend.emit_postlude().unwrap();
    out.as_str().to_string()
}

#[test]
fn reservation_size_matches_emulated_window() {
    run_8mb!({
        let out = postlude_with_emulated(0x8000);
        assert!(
            out.contains("__mmio_mem rb 32768\n"),
            "0x8000 window must reserve 32768 bytes, got:\n{out}"
        );
        let out = postlude_with_emulated(0x20000);
        assert!(
            out.contains("__mmio_mem rb 131072\n"),
            "0x20000 window must reserve 131072 bytes, got:\n{out}"
        );
        // The old hardcoded default must not reappear for a matching size —
        // the bytes come from the descriptor, not a constant.
        let out = postlude_with_emulated(0x10000);
        assert!(
            out.contains("__mmio_mem rb 65536\n"),
            "0x10000 window must reserve 65536 bytes, got:\n{out}"
        );
    });
}

#[test]
fn mmio_without_emulated_window_is_loud_error() {
    run_8mb!({
        let mod_ast = empty_module(b"module m; end;");
        let mut out = TestOut::new();
        let mut backend =
            X86_64HostedBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
        backend.set_mmio_windows(&[]).unwrap();
        // No declared window: the bounds check must fail loudly (NoMmioWindow)
        // rather than lower against an implicit default.
        let err = backend.emit_word(&mmio_store_word()).unwrap_err();
        assert!(matches!(err, CodegenError::NoMmioWindow), "got: {err:?}");
    });
}
