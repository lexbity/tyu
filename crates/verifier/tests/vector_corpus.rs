//! Per-target conformance vectors (PLAN-VERIFY-3 P2.2).
//!
//! `crates/verifier/test-vectors/<triple>/index.json` is a committed,
//! schema-versioned (`tyu.vec/1`) corpus of *transfer-table observations*:
//! for each recognized target, one JSON file of boundary-class vectors
//! (hand-authored normative expectations) plus one vector per semantics row
//! (derived expectations, drift-pinned). The port's conformance runner
//! (PLAN-VERIFY-3 P3.2, P4.1) must reproduce these observations exactly;
//! the Rust side proves the pinned observations by executing them here.
//!
//! Gates:
//! - every semantics row (`verifier::semantics::semantics()`, 42 rows) has
//!   ≥ 1 vector (its `row` mnemonic must appear);
//! - boundary classes ≥ 10 (the spec's "boundary classes ≥ 10 each":
//!   `i64::MAX`/`i64::MIN` at each width, wrap add/sub/mul → ⊤, cast trap
//!   forms, widening-naivety check via load-cut, cmp bounds, bool algebra);
//! - regeneration is deterministic (double-regen byte-equal);
//! - committed files byte-match regeneration (the golden gate).
//!
//! Regenerate with `TYU_REGEN_VECTORS=1 cargo test -p verifier
//! --test vector_corpus`; a committed-file mismatch is a *reviewed update*
//! (the band rule's spirit applies to the transfer table too).

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use common::{
    build_word, canonical_op_text, interval_text, intervals_text, json_esc, tri_name, VecOut,
};
use ir::{Atom, BlockId, CmpKind, OpKind, TypeId};
use verifier::interp::{exit_state, run_cfg, ApertureMem, FlatMem, State, SubtypeRange};
use verifier::interval::{eval_in_range, Interval, Tri};
use verifier::semantics::semantics;
use verifier::target::{TargetSpec, TARGETS};

/// The vector schema identifier.
const VEC_SCHEMA: &str = "tyu.vec/1";
/// Subtype id used by the vectors (`Percent 0..=100`, mirroring the harness).
const SUB_ID: u8 = 2;

fn sr() -> &'static SubtypeRange<'static> {
    &|tid: ir::TypeId| {
        if tid.0 == SUB_ID {
            Some((0, 100))
        } else {
            None
        }
    }
}

/// One vector: a straight-line transfer-table observation.
#[derive(Debug, Clone)]
struct Vector {
    id: String,
    class: String,
    row: String,
    ops: Vec<OpKind>,
    entry: Vec<Interval>,
    target: (i64, i64),
    cast_site: bool,
    /// The memory model the observation is made against (P2.1): `Flat` =
    /// `FlatMem` (reads `⊤`); `Aperture` = an unmapped `ApertureMem`, whose
    /// MMIO reads answer the width-bounded nondeterministic domain (§Q13).
    model: VecModel,
    /// A word-shaped (CFG) observation — the loop-widening class. `None` is a
    /// straight-line observation.
    word: Option<CfgVec>,
    expect: (Tri, Interval),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VecModel {
    Flat,
    Aperture,
}

/// A loop word: per-block op lists plus the declared signature. The CFG
/// widening ("second-visit", FR-12) is a *word-level* property — the
/// straight-line transfer table cannot observe it.
#[derive(Debug, Clone)]
struct CfgVec {
    blocks: Vec<Vec<OpKind>>,
    sig_in: usize,
    sig_out: usize,
}

impl Vector {
    fn run(&self, spec: TargetSpec) -> (Tri, Interval) {
        // Word-shaped observation: the CFG fixpoint + back-edge widening.
        if let Some(w) = &self.word {
            let word = build_word(&w.blocks, w.sig_in, w.sig_out);
            let mut flat = FlatMem;
            let mut ap = ApertureMem::new((0, 0));
            let cf = match self.model {
                VecModel::Flat => run_cfg(&word, sr(), spec, &mut flat),
                VecModel::Aperture => run_cfg(&word, sr(), spec, &mut ap),
            };
            let mut flat = FlatMem;
            let mut ap = ApertureMem::new((0, 0));
            let exit = match self.model {
                VecModel::Flat => exit_state(&cf, &word, sr(), spec, &mut flat),
                VecModel::Aperture => exit_state(&cf, &word, sr(), spec, &mut ap),
            };
            let v = exit.top_interval();
            return (eval_in_range(v, self.target.0, self.target.1), v);
        }
        // Straight-line observation over the transfer table.
        let mut flat = FlatMem;
        let mut ap = ApertureMem::new((0, 0));
        let mut st = State::fresh(64);
        for &iv in self.entry.iter() {
            st.stack.push(verifier::interp::Slot {
                iv,
                origin: verifier::interp::Origin::Computed,
            });
        }
        let mut pre_cast: Option<Interval> = None;
        for op in &self.ops {
            if matches!(op, OpKind::Cast { to, .. } if to.0 == SUB_ID) {
                pre_cast = Some(st.top_interval());
            }
            match self.model {
                VecModel::Flat => st.step(op, sr(), spec, &mut flat),
                VecModel::Aperture => st.step(op, sr(), spec, &mut ap),
            }
        }
        let v = if self.cast_site {
            pre_cast.unwrap_or(Interval::TOP)
        } else {
            st.top_interval()
        };
        (eval_in_range(v, self.target.0, self.target.1), v)
    }
}

fn iv_range(lo: i64, hi: i64) -> Interval {
    Interval::Range { lo, hi }
}

fn sub_cast() -> OpKind {
    OpKind::Cast {
        from: TypeId(0),
        to: TypeId(SUB_ID),
    }
}

// ---------------------------------------------------------------------------
// Boundary classes (hand-authored normative expectations).
// ---------------------------------------------------------------------------

/// The boundary-class vectors, per target width semantics. The data-domain
/// vectors are shared across triples (the IR data ops are full-width i64 on
/// every target); the per-width classes — the width-bounded aperture MMIO
/// domain (§Q13) — are computed per `spec`.
fn boundary_vectors(spec: TargetSpec) -> Vec<Vector> {
    let const_op = |v: i64| OpKind::ConstI64(v);
    // B1 starts the collection (vec![] — the rest push onto it).
    let mut v = vec![vec_of(
        "boundary/const-i64-max",
        "boundary:i64-max",
        "const_i64",
        vec![const_op(i64::MAX)],
        vec![],
        (0, 100),
        false,
        (Tri::DefFalse, iv_range(i64::MAX, i64::MAX)),
    )];

    // B2: i64::MIN as a constant — never in [0,100] (DefFalse).
    v.push(vec_of(
        "boundary/const-i64-min",
        "boundary:i64-min",
        "const_i64",
        vec![const_op(i64::MIN)],
        vec![],
        (0, 100),
        false,
        (Tri::DefFalse, iv_range(i64::MIN, i64::MIN)),
    ));

    // B3: wrap add — a bound computation overflowing i64 is ⊤ (open), never a
    // false discharge (the i64-with-wrap→⊤ rule; PLAN-VERIFY-3 P2).
    v.push(vec_of(
        "boundary/wrap-add",
        "boundary:wrap-add",
        "add_i64",
        vec![OpKind::AddI64],
        vec![iv_range(i64::MAX - 1, i64::MAX), iv_range(1, 1)],
        (0, 100),
        false,
        (Tri::Top, Interval::TOP),
    ));

    // B4: wrap sub.
    v.push(vec_of(
        "boundary/wrap-sub",
        "boundary:wrap-sub",
        "sub_i64",
        vec![OpKind::SubI64],
        vec![iv_range(i64::MIN + 1, i64::MIN), iv_range(2, 4)],
        (0, 100),
        false,
        (Tri::Top, Interval::TOP),
    ));

    // B5: wrap mul.
    v.push(vec_of(
        "boundary/wrap-mul",
        "boundary:wrap-mul",
        "mul_i64",
        vec![OpKind::MulI64],
        vec![iv_range(i64::MAX - 1, i64::MAX), iv_range(2, 2)],
        (0, 100),
        false,
        (Tri::Top, Interval::TOP),
    ));

    // B6: cast ALWAYS traps below the subtype range (the pre-cast site is
    // out of range ⇒ DefFalse / provably failing).
    v.push(vec_of(
        "boundary/cast-trap-below",
        "boundary:cast-trap",
        "cast",
        vec![sub_cast()],
        vec![iv_range(-150, -150)],
        (0, 100),
        true,
        (Tri::DefFalse, iv_range(-150, -150)),
    ));

    // B7: cast always traps above.
    v.push(vec_of(
        "boundary/cast-trap-above",
        "boundary:cast-trap",
        "cast",
        vec![sub_cast()],
        vec![iv_range(150, 150)],
        (0, 100),
        true,
        (Tri::DefFalse, iv_range(150, 150)),
    ));

    // B8: cast wholly in range discharges the site.
    v.push(vec_of(
        "boundary/cast-in-range",
        "boundary:cast-bounds",
        "cast",
        vec![sub_cast()],
        vec![iv_range(42, 42)],
        (0, 100),
        true,
        (Tri::DefTrue, iv_range(42, 42)),
    ));

    // B9: cast straddling the boundary stays open.
    v.push(vec_of(
        "boundary/cast-straddle",
        "boundary:cast-bounds",
        "cast",
        vec![sub_cast()],
        vec![iv_range(-20, 200)],
        (0, 100),
        true,
        (Tri::Top, iv_range(-20, 200)),
    ));

    // B10: post-cast state of an always-trapping cast is ⊥ (the check, when
    // retained, stops control flow) — ⊥ never discharges.
    v.push(vec_of(
        "boundary/cast-post-bottom",
        "boundary:cast-trap",
        "cast",
        vec![sub_cast()],
        vec![iv_range(150, 150)],
        (0, 100),
        false,
        (Tri::Top, Interval::BOTTOM),
    ));

    // B11: comparison at the maximum boundary — MAX < MIN is provably never.
    v.push(vec_of(
        "boundary/cmp-lt-max-min",
        "boundary:cmp-bounds",
        "cmp_lt",
        vec![OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Lt,
        }],
        vec![iv_range(i64::MAX, i64::MAX), iv_range(i64::MIN, i64::MIN)],
        (0, 100),
        false,
        (Tri::DefTrue, iv_range(0, 0)),
    ));

    // B12: bool algebra on [0,0]×[1,1] — and/or equations.
    v.push(vec_of(
        "boundary/bool-and-false",
        "boundary:bool-algebra",
        "and_bool",
        vec![OpKind::AndBool],
        vec![iv_range(0, 0), iv_range(1, 1)],
        (0, 100),
        false,
        (Tri::DefTrue, iv_range(0, 0)),
    ));
    v.push(vec_of(
        "boundary/bool-or-true",
        "boundary:bool-algebra",
        "or_bool",
        vec![OpKind::OrBool],
        vec![iv_range(0, 0), iv_range(1, 1)],
        (0, 100),
        false,
        (Tri::DefTrue, iv_range(1, 1)),
    ));

    // B13: load cuts the value flow to ⊤ — open, never discharged (Q4/P2
    // FlatMem) — the "unmodeled memory" boundary class.
    v.push(vec_of(
        "boundary/load-opaque",
        "boundary:load-unmodeled",
        "load",
        vec![OpKind::Load { ty: TypeId(0) }],
        vec![iv_range(8, 8)],
        (0, 100),
        false,
        (Tri::Top, Interval::TOP),
    ));

    // B14: MMIO aperture read is the width-bounded nondeterministic oracle —
    // for the default FlatMem the read is ⊤ (sound for any width); the site
    // stays open.
    v.push(vec_of(
        "boundary/mmio-read-nondeterministic",
        "boundary:mmio-reg-width",
        "vol_load",
        vec![OpKind::MmioVolLoad {
            ty: TypeId(0),
            place: Atom::new(b"r").unwrap(),
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        }],
        vec![iv_range(0, 0)],
        (0, 100),
        false,
        (Tri::Top, Interval::TOP),
    ));

    // B15: the *width-relative* aperture read — the ONE boundary class whose
    // observation depends on the target's word width (§Q13, §Q3). An
    // `ApertureMem` has no RAM and no scripts, so the read answers the
    // register's nondeterministic domain: `⊤` at 64 bits (full i64 domain),
    // `[-2^31, 2^31-1]` at 32 bits. This is the class that makes the corpora
    // genuinely per-target (the data-domain transparency assertion excludes
    // it by construction).
    let (dom_lo, dom_hi) = spec.signed_domain();
    let aperture_top = if spec.word_bits >= 64 {
        Interval::TOP
    } else {
        iv_range(dom_lo, dom_hi)
    };
    v.push(Vector {
        id: "boundary/aperture-read-width-domain".to_string(),
        class: "boundary:mmio-reg-width".to_string(),
        row: "vol_load".to_string(),
        ops: vec![OpKind::MmioVolLoad {
            ty: TypeId(0),
            place: Atom::new(b"reg").unwrap(),
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        }],
        entry: vec![iv_range(0, 0)],
        target: (0, 100),
        cast_site: false,
        model: VecModel::Aperture,
        word: None,
        expect: (Tri::Top, aperture_top),
    });

    // B16: loop back-edge widening — the "second-visit" boundary class
    // (FR-12): a loop-carried slot reaches ⊤ on the second header visit and
    // the exit obligation must stay open (a discharge here would be a false
    // elision — the loop DOES carry values outside [0,8]). Word-shaped.
    v.push(Vector {
        id: "boundary/widen-second-visit".to_string(),
        class: "boundary:widen-second-visit".to_string(),
        row: "br_if".to_string(), // the loop-head control op
        ops: Vec::new(),
        entry: Vec::new(),
        target: (0, 8),
        cast_site: false,
        model: VecModel::Flat,
        word: Some(widen_loop_cfg()),
        expect: (Tri::Top, Interval::TOP),
    });

    v
}

/// A data-domain (FlatMem) boundary vector.
#[allow(clippy::too_many_arguments)] // test-vector constructor: id/class/row/ops/entry/target/cast/expect
fn vec_of(
    id: &str,
    class: &str,
    row: &str,
    ops: Vec<OpKind>,
    entry: Vec<Interval>,
    target: (i64, i64),
    cast_site: bool,
    expect: (Tri, Interval),
) -> Vector {
    Vector {
        id: id.to_string(),
        class: class.to_string(),
        row: row.to_string(),
        ops,
        entry,
        target,
        cast_site,
        model: VecModel::Flat,
        word: None,
        expect,
    }
}

/// The synthetic widening loop (mirrors `soundness_differential::loop_word`):
/// `x = input; while (x < 10) { x = x + 1 }; ret x`. The back-edge widening
/// (FR-12) pushes the loop-carried local to `⊤` on the second header visit.
fn widen_loop_cfg() -> CfgVec {
    let ty = TypeId(0);
    CfgVec {
        blocks: vec![
            // b0: x → local 1, br b1
            vec![
                OpKind::LocalSet { slot: 1, ty },
                OpKind::Br { target: BlockId(1) },
            ],
            // b1 (header): if x < 10 → b2 (body) else b3 (exit)
            vec![
                OpKind::LocalGet { slot: 1, ty },
                OpKind::ConstI64(10),
                OpKind::Cmp {
                    out: TypeId(1),
                    kind: CmpKind::Lt,
                },
                OpKind::BrIf {
                    then_tgt: BlockId(2),
                    else_tgt: BlockId(3),
                },
            ],
            // b2 (body): x = x + 1; br b1
            vec![
                OpKind::LocalGet { slot: 1, ty },
                OpKind::ConstI64(1),
                OpKind::AddI64,
                OpKind::LocalSet { slot: 1, ty },
                OpKind::Br { target: BlockId(1) },
            ],
            // b3 (exit): ret x
            vec![OpKind::LocalGet { slot: 1, ty }, OpKind::Ret],
        ],
        sig_in: 1,
        sig_out: 1,
    }
}

// ---------------------------------------------------------------------------
// Row coverage vectors (one per semantics row, derived expectations).
// ---------------------------------------------------------------------------

/// One vector per row of the normative semantics table. The entry stack is
/// sized by the row's `pops`; expectations are *derived* from the engine and
/// drift-pinned by the golden files (row coverage + determinism, not
/// normative hand-checks — those are the boundary class).
fn row_vectors() -> Vec<Vector> {
    let mut out = Vec::new();
    for row in semantics().iter() {
        let mut entry: Vec<Interval> = Vec::new();
        for i in 0..row.pops {
            entry.push(iv_range(4 + i as i64, 4 + i as i64));
        }
        let mut v = Vector {
            id: format!("row/{}", row.mnemonic),
            class: "row".into(),
            row: row.mnemonic.to_string(),
            ops: vec![row.op],
            entry: entry.clone(),
            target: (0, 100),
            cast_site: false,
            model: VecModel::Flat,
            word: None,
            expect: (Tri::Top, Interval::TOP), // overwritten from the engine
        };
        let (head, top) = v.run(TargetSpec::X86_64_UNKNOWN_NONE);
        v.expect = (head, top);
        out.push(v);
    }
    out
}

/// Build the full vector set for one target: the shared data-domain boundary
/// classes, the per-width aperture class, the widening class, and one vector
/// per semantics row.
fn vectors_for(spec: TargetSpec) -> Vec<Vector> {
    let mut all = boundary_vectors(spec);
    all.append(&mut row_vectors());
    all
}

// ---------------------------------------------------------------------------
// Canonical op text (drift-locked against `ir::write_word_ops`).
// ---------------------------------------------------------------------------

/// The full canonical block text of a CFG vector (block headers INCLUDED —
/// the port needs the block structure, not just an op list).
fn canonical_cfg_text(cfg: &CfgVec) -> String {
    let w = build_word(&cfg.blocks, cfg.sig_in, cfg.sig_out);
    let mut buf = VecOut(Vec::new());
    ir::write_word_ops(&mut buf, &w);
    String::from_utf8_lossy(&buf.0).trim_end().to_string()
}

// ---------------------------------------------------------------------------
// JSON serialization (deterministic; no maps, fixed order).
// ---------------------------------------------------------------------------

fn vec_dir(triple: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(triple)
}

fn render_json(spec: TargetSpec) -> String {
    let vectors = vectors_for(spec);
    if vectors.is_empty() {
        panic!("vector corpus for {} is empty", spec.triple);
    }
    let mut c = Vec::new();
    c.push(format!(
        "{{\n  \"schema\": \"{VEC_SCHEMA}\",\n  \"triple\": \"{}\",\n  \"target\": {{",
        spec.triple
    ));
    c.push(format!(
        "    \"slot_bytes\": {},\n    \"word_bits\": {},\n    \"arch_tag\": {}",
        spec.slot_bytes, spec.word_bits, spec.arch_tag
    ));
    c.push("  },".to_string());
    c.push("  \"vectors\": [".to_string());
    for (i, v) in vectors.iter().enumerate() {
        let comma = if i + 1 < vectors.len() { "," } else { "" };
        c.push("    {".to_string());
        c.push(format!("      \"id\": \"{}\",", json_esc(&v.id)));
        c.push(format!("      \"class\": \"{}\",", json_esc(&v.class)));
        c.push(format!("      \"row\": \"{}\",", json_esc(&v.row)));
        c.push(format!(
            "      \"model\": \"{}\",",
            match v.model {
                VecModel::Flat => "flat",
                VecModel::Aperture => "aperture",
            }
        ));
        match &v.word {
            None => {
                c.push("      \"mode\": \"linear\",".to_string());
                c.push(format!(
                    "      \"ops\": \"{}\",",
                    json_esc(&canonical_op_text(&v.ops))
                ));
                c.push(format!(
                    "      \"entry\": \"{}\",",
                    json_esc(&intervals_text(&v.entry))
                ));
            }
            Some(cfg) => {
                c.push("      \"mode\": \"cfg\",".to_string());
                c.push(format!("      \"sig_in\": {},", cfg.sig_in));
                c.push(format!("      \"sig_out\": {},", cfg.sig_out));
                c.push(format!(
                    "      \"blocks\": \"{}\",",
                    json_esc(&canonical_cfg_text(cfg))
                ));
            }
        }
        c.push(format!(
            "      \"target\": [{}, {}],",
            v.target.0, v.target.1
        ));
        let cs = if v.cast_site { "true" } else { "false" };
        c.push(format!("      \"cast_site\": {cs},"));
        c.push(format!(
            "      \"expect\": {{\"head\": \"{}\", \"top\": \"{}\"}}",
            tri_name(v.expect.0),
            interval_text(v.expect.1)
        ));
        c.push(format!("    }}{comma}"));
    }
    c.push("  ]\n}".to_string());
    c.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The committed per-target files must byte-match regeneration (the golden
/// gate; the transfer table is pinned, like statement hashes).
#[test]
fn vector_corpus_files_match_regeneration() {
    let mut regen = false;
    for spec in TARGETS.iter().copied() {
        let dir = vec_dir(spec.triple);
        let path = dir.join("index.json");
        if std::env::var("TYU_REGEN_VECTORS").is_ok() {
            fs::create_dir_all(&dir).expect("create vector dir");
            fs::write(&path, render_json(spec)).expect("write vector corpus");
            regen = true;
        } else {
            let expected = render_json(spec);
            let actual = fs::read_to_string(&path)
                .expect("vector corpus file present (run with TYU_REGEN_VECTORS=1)");
            assert_eq!(
                actual,
                expected,
                "vector corpus for {} drifted from the engine — regenerate + review ({})",
                spec.triple,
                path.display()
            );
        }
    }
    if regen {
        eprintln!("TYU_REGEN_VECTORS=1: regenerated all vector corpora");
    }
}

/// Determinism: regeneration is byte-identical (FR-16's determinism gate,
/// applied to the vector surface).
#[test]
fn vector_corpus_regeneration_is_deterministic() {
    for spec in TARGETS.iter().copied() {
        let a = render_json(spec);
        let b = render_json(spec);
        assert_eq!(
            a, b,
            "vector corpus for {} must be deterministic",
            spec.triple
        );
    }
}

/// Every one of the 42 semantics rows has ≥ 1 vector; boundary classes ≥ 10
/// (the spec's sizing for P2.2).
#[test]
fn vector_corpus_coverage() {
    for spec in TARGETS.iter().copied() {
        let vectors = vectors_for(spec);
        // Row coverage: every mnemonic of the normative semantics table.
        let mut rows_covered: BTreeSet<&str> = BTreeSet::new();
        let mut classes: BTreeSet<&str> = BTreeSet::new();
        for v in vectors.iter() {
            rows_covered.insert(&v.row);
            classes.insert(&v.class);
        }
        for row in semantics().iter() {
            assert!(
                rows_covered.contains(row.mnemonic),
                "{}: no vector for semantics row '{}' (table drift)",
                spec.triple,
                row.mnemonic
            );
        }
        // Reverse direction: every vector's row label IS a semantics mnemonic
        // (a typo'd row label would silently void the coverage claim).
        let canonical: BTreeSet<&str> = semantics().iter().map(|r| r.mnemonic).collect();
        for v in vectors.iter() {
            assert!(
                canonical.contains(v.row.as_str()),
                "{}: vector {} labels row '{}' which is not a semantics mnemonic",
                spec.triple,
                v.id,
                v.row
            );
        }
        // Boundary classes ≥ 10 (spec §11.4: "boundary classes ≥ 10").
        let boundary = classes
            .iter()
            .filter(|c| c.starts_with("boundary:"))
            .count();
        assert!(
            boundary >= 10,
            "{}: boundary classes {boundary} < 10 — the P2.2 sizing contract",
            spec.triple
        );
        assert!(
            vectors.len() >= 42,
            "{}: vector count {} < row count 42",
            spec.triple,
            vectors.len()
        );
    }
}

/// The vectors execute: every committed observation is reproduced by the
/// engine today (a regression in the transfer table fails here BEFORE the
/// port conformance ever sees a drifted corpus).
#[test]
fn vector_corpus_executes_and_satisfies_expect() {
    for spec in TARGETS.iter().copied() {
        let vectors = vectors_for(spec);
        for v in vectors.iter() {
            let (head, top) = v.run(spec);
            assert_eq!(
                head,
                v.expect.0,
                "{} [{}] head drifted: expected {:?}, engine says {:?} (top {})",
                spec.triple,
                v.id,
                v.expect.0,
                head,
                interval_text(top)
            );
            assert_eq!(
                top, v.expect.1,
                "{} [{}] interval drifted",
                spec.triple, v.id
            );
        }
    }
}

/// Cross-target transparency: the SAME data-domain vectors produce identical
/// observations under every target (the width parameterization is a no-op on
/// the data domain — the rune that makes port vectors portable). The ONE
/// width-relative class (`model: aperture`) is excluded by construction —
/// its whole point is to differ per target (§Q13).
#[test]
fn vector_corpus_targets_are_data_transparent() {
    let reference = TargetSpec::X86_64_UNKNOWN_NONE;
    let data_flat = |v: &Vector| -> Option<(String, Tri, Interval)> {
        if v.model == VecModel::Flat {
            Some((v.id.clone(), v.expect.0, v.expect.1))
        } else {
            None
        }
    };
    let ref_vectors: Vec<(String, Tri, Interval)> = vectors_for(reference)
        .iter()
        .filter_map(data_flat)
        .collect();
    for spec in TARGETS.iter().copied() {
        if spec == reference {
            continue;
        }
        let vectors: Vec<(String, Tri, Interval)> =
            vectors_for(spec).iter().filter_map(data_flat).collect();
        assert_eq!(
            vectors.len(),
            ref_vectors.len(),
            "{}: flat-vector count differs",
            spec.triple
        );
        for (v, rv) in vectors.iter().zip(ref_vectors.iter()) {
            assert_eq!(
                (v.1, v.2),
                (rv.1, rv.2),
                "{} [{}]: expect differs from x86_64",
                spec.triple,
                v.0
            );
        }
    }
}

/// The width-relative class is exactly as width-relative as intended: the
/// aperture read domain is `⊤` at 64 bits and the signed 32-bit domain at
/// 32 bits — a 32-bit register cannot return a 64-bit value (§Q13).
#[test]
fn aperture_width_domain_vector_is_width_relative() {
    let find_aperture = |spec: TargetSpec| {
        vectors_for(spec)
            .iter()
            .find(|v| v.id == "boundary/aperture-read-width-domain")
            .expect("aperture vector present")
            .clone()
    };
    let x64 = find_aperture(TargetSpec::X86_64_UNKNOWN_NONE);
    assert_eq!(
        x64.expect.1,
        Interval::TOP,
        "64-bit aperture read is the full i64 domain"
    );
    for spec in [
        TargetSpec::ARM_V7M_UNKNOWN_NONE,
        TargetSpec::RISCV32_UNKNOWN_NONE,
    ] {
        let v = find_aperture(spec);
        assert_eq!(
            v.expect.1,
            iv_range(-(1i64 << 31), (1i64 << 31) - 1),
            "{}: 32-bit aperture read is the signed 32-bit domain",
            spec.triple
        );
        // And the engine reproduces it (execution consistency for the class).
        assert_eq!(v.run(spec).0, Tri::Top);
    }
}
