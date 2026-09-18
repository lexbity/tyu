use codegen_core::{AssemblerKind, CodegenError, EmitMode, Target};
use ir;

// ---------------------------------------------------------------------------
// Target::parse — exhaustive roundtrip via Target::ALL
// ---------------------------------------------------------------------------

#[test]
fn target_parse_known_triple() {
    assert_eq!(
        Target::parse(b"x86_64-unknown-linux-gnu"),
        Some(Target::X86_64UnknownLinuxGnu)
    );
}

#[test]
fn target_parse_unknown_triples_return_none() {
    assert!(Target::parse(b"arm-none-eabi").is_none());
    assert!(Target::parse(b"thumbv6m-none-eabi").is_none());
    assert!(Target::parse(b"linux-x86_64-hosted").is_none());
    assert!(Target::parse(b"").is_none());
    assert!(Target::parse(b"garbage").is_none());
}

#[test]
fn target_all_roundtrips_through_parse() {
    for t in &Target::ALL {
        let parsed = Target::parse(t.triple());
        assert_eq!(parsed, Some(*t), "roundtrip failed for {t:?}");
    }
}

// ---------------------------------------------------------------------------
// TargetSpec — field assertions per architecture
// ---------------------------------------------------------------------------

#[test]
fn target_spec_x86_64_linux_gnu_fields() {
    let spec = Target::X86_64UnknownLinuxGnu.spec();
    assert_eq!(spec.word_bits, 64);
    assert_eq!(spec.pointer_bits, 64);
    assert_eq!(spec.native_int_ty, b"i64");
    assert_eq!(spec.assembler, AssemblerKind::Fasm);
}

#[test]
fn riscv_target_spec_fields() {
    let spec = Target::RiscV32UnknownNone.spec();
    assert_eq!(spec.slot_bytes, 4);
    assert_eq!(spec.word_bits, 32);
    assert_eq!(spec.pointer_bits, 32);
    assert_eq!(spec.native_int_ty, b"i64"); // i64 is the universal native int type
    assert_eq!(spec.assembler, AssemblerKind::GasRiscV);
    assert_eq!(spec.linker, b"riscv32-elf-ld");
}

#[test]
fn riscv_target_qemu_exit_convention() {
    let qemu = Target::RiscV32UnknownNone.spec().qemu.unwrap();
    assert_eq!(qemu.system_bin, b"qemu-system-riscv32");
    assert_eq!(qemu.machine, b"virt");
    assert!(matches!(
        qemu.exit_convention,
        codegen_core::QemuExitConvention::Semihosting
    ));
    assert_eq!(qemu.exit_convention.host_pass_exit(), 0);
}

#[test]
fn arm_target_spec_fields() {
    let spec = Target::ArmV7MUnknownNone.spec();
    assert_eq!(spec.slot_bytes, 4);
    assert_eq!(spec.word_bits, 32);
    assert_eq!(spec.assembler, AssemblerKind::GasArm);
    assert_eq!(spec.linker, b"arm-none-eabi-ld");
}

#[test]
fn arm_target_qemu_exit_convention() {
    let qemu = Target::ArmV7MUnknownNone.spec().qemu.unwrap();
    assert!(matches!(
        qemu.exit_convention,
        codegen_core::QemuExitConvention::Semihosting
    ));
    assert_eq!(qemu.exit_convention.host_pass_exit(), 0);
}

#[test]
fn x86_targets_unchanged_after_arm_added() {
    let none = Target::X86_64UnknownNone.spec();
    assert_eq!(none.linker, b"ld");
    assert_eq!(none.assembler, AssemblerKind::Fasm);
    assert_eq!(none.slot_bytes, 8);
    let linux = Target::X86_64UnknownLinuxGnu.spec();
    assert_eq!(linux.linker, b"ld");
    assert_eq!(linux.assembler, AssemblerKind::Fasm);
}

// ---------------------------------------------------------------------------
// CodegenError::code() — numeric codes must be stable across refactors
// ---------------------------------------------------------------------------

#[test]
fn codegen_error_named_variant_codes() {
    assert_eq!(CodegenError::UnsupportedOp { op_name: b"x" }.code(), 8001);
    assert_eq!(CodegenError::MissingEntryPoint { name: b"x" }.code(), 8002);
    assert_eq!(CodegenError::OutputCapacityExceeded.code(), 8003);
    assert_eq!(
        CodegenError::InvalidCast {
            from: ir::TY_I64,
            to: ir::TY_BOOL
        }
        .code(),
        8004
    );
    assert_eq!(CodegenError::UnsupportedEmitMode.code(), 8005);
    assert_eq!(CodegenError::MalformedStringLiteral.code(), 8006);
    assert_eq!(CodegenError::MalformedIr { detail: 0 }.code(), 8007);
    assert_eq!(CodegenError::UnsupportedAddrOf.code(), 8008);
    assert_eq!(
        CodegenError::UnknownTypeProperties {
            type_id: ir::TY_I64
        }
        .code(),
        8009
    );
    assert_eq!(CodegenError::UnsupportedCheckSubtype.code(), 8010);
    assert_eq!(CodegenError::StringLiteralCapacityExceeded.code(), 8011);
    assert_eq!(CodegenError::ScopedAllocationOverflow.code(), 8012);
    assert_eq!(CodegenError::ModInfoTooLarge.code(), 8013);
}

// ---------------------------------------------------------------------------
// EmitMode — exhaustive iteration via EmitMode::ALL
// ---------------------------------------------------------------------------

#[test]
fn emit_mode_only_obj_is_production() {
    assert!(EmitMode::Obj.is_production());
    assert!(!EmitMode::Ast.is_production());
    assert!(!EmitMode::Ir.is_production());
    assert!(!EmitMode::StackCheck.is_production());
    assert!(!EmitMode::Asm.is_production());
}

#[test]
fn emit_mode_all_complement() {
    // Exhaustive: every EmitMode has consistent is_production/is_inspection.
    for mode in &EmitMode::ALL {
        assert_eq!(mode.is_inspection(), !mode.is_production());
    }
}
