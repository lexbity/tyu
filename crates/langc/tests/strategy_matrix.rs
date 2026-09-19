//! Strategy-matrix golden (design doc §5.6, P5): each backend's exhaustive
//! `(strategy, aperture-kind, op)` classification rendered and diffed against a
//! committed artifact. A cell changing without its golden changing is
//! impossible. Every `Supported(pattern)` row maps to a QEMU fixture that
//! exercises it (see the per-target `mmio_strategies_*` fixtures).
//!
//! Lives here (not codegen-core) because the renderer needs all three
//! backends, and codegen-core cannot depend on them (cycle).

use codegen_core::strategy::{render_strategy_matrix, MmioOp, StrategyCell};
use ir::{ApertureKind, WriteKind};

fn golden_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

#[test]
fn strategy_matrices_match_golden() {
    let mut rendered = String::new();
    for (name, cell) in [
        ("arm", codegen_arm::mmio_cell as fn(_, _, _) -> StrategyCell),
        ("riscv", codegen_riscv::mmio_cell as fn(_, _, _) -> StrategyCell),
        ("x86_64", codegen_x86_64::mmio_cell as fn(_, _, _) -> StrategyCell),
    ] {
        rendered.push_str(&render_strategy_matrix(name, cell));
    }

    let path = golden_dir().join("strategy_matrix.txt");
    if std::env::var_os("TYU_BLESS_STRATEGY_MATRIX").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "missing strategy-matrix golden {}: {err}; rerun with TYU_BLESS_STRATEGY_MATRIX=1",
            path.display()
        )
    });
    assert_eq!(expected, rendered, "strategy matrix drifted");
}

#[test]
fn matrix_cells_are_exhaustive() {
    for strategy in [WriteKind::Plain, WriteKind::W1s, WriteKind::W1c] {
        for kind in [ApertureKind::Bus, ApertureKind::Emulated] {
            for op in [MmioOp::Load, MmioOp::Store, MmioOp::LoadField, MmioOp::StoreField] {
                let _ = codegen_arm::mmio_cell(strategy, kind, op);
                let _ = codegen_riscv::mmio_cell(strategy, kind, op);
                let _ = codegen_x86_64::mmio_cell(strategy, kind, op);
            }
        }
    }
}
