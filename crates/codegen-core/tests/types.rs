use codegen_core::{AssemblerKind, CodegenError, EmitMode, Target};
use ir;

// ---------------------------------------------------------------------------
// Target::parse
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
    assert!(Target::parse(b"linux-x86_64-hosted").is_none()); // old non-LLVM name
    assert!(Target::parse(b"").is_none());
    assert!(Target::parse(b"garbage").is_none());
}

#[test]
fn target_triple_roundtrips_through_parse() {
    let t = Target::X86_64UnknownLinuxGnu;
    assert_eq!(Target::parse(t.triple()), Some(t));
}

// ---------------------------------------------------------------------------
// TargetSpec
// ---------------------------------------------------------------------------

#[test]
fn target_spec_x86_64_linux_gnu_fields() {
    let spec = Target::X86_64UnknownLinuxGnu.spec();
    assert_eq!(spec.word_bits, 64);
    assert_eq!(spec.pointer_bits, 64);
    assert_eq!(spec.native_int_ty, b"i64");
    assert_eq!(spec.assembler, AssemblerKind::Fasm);
}

// ---------------------------------------------------------------------------
// CodegenError::code() — numeric codes must be stable across refactors
// ---------------------------------------------------------------------------

#[test]
fn codegen_error_codes_are_stable() {
    assert_eq!(CodegenError::UnsupportedOp { op_name: b"x" }.code(), 8001);
    assert_eq!(
        CodegenError::MissingEntryPoint { name: b"main" }.code(),
        8002
    );
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
}

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
}

// ---------------------------------------------------------------------------
// EmitMode
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
fn emit_mode_inspection_is_complement_of_production() {
    for mode in [
        EmitMode::Ast,
        EmitMode::Ir,
        EmitMode::StackCheck,
        EmitMode::Asm,
        EmitMode::Obj,
    ] {
        assert_eq!(mode.is_inspection(), !mode.is_production());
    }
}
