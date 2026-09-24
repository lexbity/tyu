//! Semantics-table coverage and artifact drift tests (static-verification.md
//! slice P1, FR-3).
//!
//! - the table compiles at all (the wildcard-free `match` in
//!   `verifier::semantics` is the structural enforcement that every `OpKind`
//!   variant has a row);
//! - `src/semantics/ops.json` is byte-identical to the regenerated artifact
//!   (drift fails CI); regenerate with
//!   `TYU_EXPORT_SEMANTICS=1 cargo test -p verifier --test semantics_coverage`;
//! - every row's mnemonic is the exact first token the `--emit=ir` printer
//!   emits for its representative op (one canonical printer, two consumers);
//! - the canonical check-form ops (the cmp/trap pairs the subtype-range and
//!   contract checks lower to) all carry a value-level OEL projection or are
//!   control, so no site of classes C1–C7 is opaque by accident.

use std::fs;

use ir::{Block, FixedVec, Op, OpKind, Output, Span, Word};
use verifier::semantics::{semantics, OelProj, SemanticsRow, SEMANTICS_ROWS, SEMANTICS_VERSION};

fn ops_json(rows: &[SemanticsRow]) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"semantics\": \"{SEMANTICS_VERSION}\",\n"));
    out.push_str(&format!("  \"ir_format_ver\": {},\n", ir::FORMAT_VER));
    out.push_str(&format!("  \"row_count\": {},\n", rows.len()));
    out.push_str("  \"rows\": [\n");
    for (i, row) in rows.iter().enumerate() {
        let op_name = op_variant_name(&row.op);
        let effect_bits = row.effect.bits();
        let oel = match row.oel {
            OelProj::Value(v) => {
                let name = match v {
                    verifier::semantics::ValueOp::Const => "const",
                    verifier::semantics::ValueOp::Add => "add",
                    verifier::semantics::ValueOp::Sub => "sub",
                    verifier::semantics::ValueOp::Mul => "mul",
                    verifier::semantics::ValueOp::Cast => "cast",
                    verifier::semantics::ValueOp::Bitcast => "bitcast",
                    verifier::semantics::ValueOp::Cmp(k) => match k {
                        ir::CmpKind::Lt => "cmp_lt",
                        ir::CmpKind::Le => "cmp_le",
                        ir::CmpKind::Gt => "cmp_gt",
                        ir::CmpKind::Ge => "cmp_ge",
                        ir::CmpKind::Eq => "cmp_eq",
                        ir::CmpKind::Ne => "cmp_ne",
                    },
                    verifier::semantics::ValueOp::And => "and",
                    verifier::semantics::ValueOp::Or => "or",
                    verifier::semantics::ValueOp::Not => "not",
                    verifier::semantics::ValueOp::LocalGet => "local_get",
                    verifier::semantics::ValueOp::LocalSet => "local_set",
                    verifier::semantics::ValueOp::Dup => "dup",
                    verifier::semantics::ValueOp::Drop => "drop",
                    verifier::semantics::ValueOp::Swap => "swap",
                };
                format!("value:{name}")
            }
            OelProj::Opaque => "opaque".to_string(),
            OelProj::Control => "control".to_string(),
        };
        let comma = if i + 1 == rows.len() { "" } else { "," };
        out.push_str(&format!(
            "    {{\"op\": \"{op_name}\", \"mnemonic\": \"{}\", \"pops\": {}, \
             \"pushes\": {}, \"effect_bits\": {effect_bits}, \"oel\": \"{oel}\"}}{comma}\n",
            row.mnemonic, row.pops, row.pushes,
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

/// The `OpKind` variant name of a representative op, derived from its Debug
/// form ("Dup { ty: ... }" -> "Dup").
fn op_variant_name(op: &OpKind) -> String {
    let debug = format!("{op:?}");
    debug
        .find(|c: char| c == '(' || c == ' ' || c == '{')
        .map(|i| debug[..i].to_string())
        .unwrap_or(debug)
}

fn artifact_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/semantics/ops.json")
}

/// Render a representative op through the `--emit=ir` printer and return the
/// printed op line (without the 4-space indent).
fn printed_op_text(row: &SemanticsRow) -> String {
    let mut word = Word {
        name: ir::Atom::new(b"f").unwrap(),
        sig: ir::Sig::empty(),
        performs: ir::EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: ir::BlockId(0),
        types: FixedVec::new(),
        type_sizes: FixedVec::new(),
        type_classes: FixedVec::new(),
        apertures: FixedVec::new(),
        subtype_bases: FixedVec::new(),
        blocks: FixedVec::new(),
    };
    let mut block = Block {
        id: ir::BlockId(0),
        entry_stack: FixedVec::new(),
        ops: FixedVec::new(),
    };
    block.ops.push(Op {
        kind: row.op,
        span: Span::UNKNOWN,
    }).unwrap();
    word.blocks.push(block).unwrap();

    struct StrOut(String);
    impl Output for StrOut {
        fn write(&mut self, bytes: &[u8]) {
            self.0.push_str(&String::from_utf8_lossy(bytes));
        }
    }
    let mut out = StrOut(String::new());
    ir::write_word(&mut out, &word);
    // The word renders as: header line, then "  block b0 ()", then the op
    // line indented by 4 spaces. Take the last non-empty line.
    out.0
        .lines()
        .rev()
        .map(str::trim_start)
        .next()
        .unwrap_or("")
        .to_string()
}

/// The mnemonic must be the exact first token the printer emits (or the whole
/// line, for no-operand forms).
#[test]
fn mnemonics_match_ir_printer() {
    for row in semantics() {
        let printed = printed_op_text(&row);
        let expected = row.mnemonic;
        assert!(
            printed == expected
                || printed.strip_prefix(expected)
                    .map_or(false, |rest| rest.starts_with(' ')),
            "mnemonic drift: row '{}' but --emit=ir printer emits {:?}",
            expected,
            printed
        );
    }
}

#[test]
fn row_count_is_stable_and_duplicates_free() {
    let rows = semantics();
    assert_eq!(rows.len(), SEMANTICS_ROWS);
    let mut seen: [&'static str; SEMANTICS_ROWS] = [""; SEMANTICS_ROWS];
    for (i, row) in rows.iter().enumerate() {
        assert!(
            !seen[..i].contains(&row.mnemonic),
            "duplicate mnemonic: {}",
            row.mnemonic
        );
        seen[i] = row.mnemonic;
    }
}

/// The ops that make up the canonical runtime check forms (cmp/trap pairs
/// emitted by `emit_subtype_range_trap` and the `TrapIfFalse` contract trap)
/// must carry a value-level OEL projection or be control — a check-form op
/// must never be opaque by accident, or no discharger could ever reason about
/// obligations at those sites.
#[test]
fn check_form_ops_have_oel_projection() {
    let table = semantics();
    let check_form_mnemonics = [
        "const_i64",
        "const_bool",
        "local_get",
        "cmp_lt",
        "cmp_le",
        "cmp_gt",
        "cmp_ge",
        "cmp_eq",
        "cmp_ne",
        "and_bool",
        "or_bool",
        "not_bool",
    ];
    for mnemonic in check_form_mnemonics {
        let row = table
            .iter()
            .find(|r| r.mnemonic == mnemonic)
            .unwrap_or_else(|| panic!("check-form op {mnemonic} missing from semantics table"));
        assert!(
            matches!(row.oel, OelProj::Value(_)),
            "check-form op {mnemonic} must project to a value op"
        );
    }
    for mnemonic in ["trap_if_false", "br", "br_if", "ret"] {
        let row = table
            .iter()
            .find(|r| r.mnemonic == mnemonic)
            .expect("control op present");
        assert_eq!(row.oel, OelProj::Control, "{mnemonic} must be control");
    }
}

/// Spot checks pinning the (pops, pushes) transitions of the check-form ops
/// against the canonical emitted shapes (`emit_subtype_range_trap`: value,
/// const, cmp, trap_if_false; net -1 per pair element after the value load).
#[test]
fn check_form_stack_transitions() {
    let table = semantics();
    let find =
        |m: &str| table.iter().find(|r| r.mnemonic == m).expect("row present");
    // `local_get i` loads the checked value: ( -- v )
    assert_eq!((find("local_get").pops, find("local_get").pushes), (0, 1));
    // `const_i64 min`: ( -- min )
    assert_eq!((find("const_i64").pops, find("const_i64").pushes), (0, 1));
    // `cmp_ge`: ( v min -- bool )
    assert_eq!((find("cmp_ge").pops, find("cmp_ge").pushes), (2, 1));
    // `trap_if_false`: ( bool -- )
    assert_eq!((find("trap_if_false").pops, find("trap_if_false").pushes), (1, 0));
    // dup of the checked value for the pair: ( v -- v v )
    assert_eq!((find("dup").pops, find("dup").pushes), (1, 2));
}

#[test]
fn semantics_version_is_versioned_string() {
    assert!(SEMANTICS_VERSION.starts_with("tyu.ir-sem/"));
    let (_majors, ver) = SEMANTICS_VERSION
        .rsplit_once('/')
        .expect("version has a / separator");
    let mut parts = ver.split('.');
    let major: u32 = parts.next().expect("major").parse().expect("numeric major");
    let _minor: u32 = parts.next().expect("minor").parse().expect("numeric minor");
    assert!(major >= 1);
}

/// Regenerates `src/semantics/ops.json` from the table. Not part of CI:
/// only runs when TYU_EXPORT_SEMANTICS=1 (usage documented in the module).
#[test]
fn export_semantics_artifact() {
    if std::env::var("TYU_EXPORT_SEMANTICS").as_deref() != Ok("1") {
        return;
    }
    let json = ops_json(&semantics());
    fs::write(artifact_path(), json).expect("write ops.json");
}

/// The committed artifact must be byte-identical to the regenerated one.
#[test]
fn semantics_artifact_is_current() {
    let json = ops_json(&semantics());
    let committed = fs::read_to_string(artifact_path())
        .expect("src/semantics/ops.json is committed; regenerate with TYU_EXPORT_SEMANTICS=1");
    assert_eq!(
        committed, json,
        "ops.json drifted from the semantics table; regenerate with TYU_EXPORT_SEMANTICS=1"
    );
}
