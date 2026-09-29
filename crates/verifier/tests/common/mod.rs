//! Shared helpers for the verifier's corpus suites (cleanup item 4): the
//! synthetic-word builder, the canonical `--emit=ir` op-text projection
//! (the interface the port's `parseOps` consumes), and the deterministic
//! JSON rendering helpers. Used by `vector_corpus` (per-target corpora) and
//! `bundle_instance_conformance` (bundle evidence corpora) — one home
//! instead of the per-file copies, so the canonical text and the JSON
//! escaping cannot drift between the two corpus generators.
#![allow(dead_code)] // each corpus binary uses only a subset of the shared surface

use ir::{Atom, BlockId, EffectSet, OpKind, Sig, Span};
use std::path::PathBuf;

/// The workspace root (two levels above the crate manifest directory).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build a synthetic word from per-block op lists (types `i64`/`bool`/
/// `percent`, subtype `TypeId(2)` = `percent`).
pub fn build_word(blocks: &[Vec<OpKind>], sig_in: usize, sig_out: usize) -> ir::Word {
    let mut w = ir::Word {
        name: Atom::new(b"vector").unwrap(),
        sig: Sig {
            in_len: sig_in as u8,
            out_len: sig_out as u8,
            ..Sig::empty()
        },
        performs: EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: BlockId(0),
        types: Default::default(),
        type_sizes: Default::default(),
        type_classes: Default::default(),
        apertures: Default::default(),
        subtype_bases: Default::default(),
        blocks: Default::default(),
    };
    w.types.push(Atom::new(b"i64").unwrap()).unwrap();
    w.types.push(Atom::new(b"bool").unwrap()).unwrap();
    w.types.push(Atom::new(b"percent").unwrap()).unwrap();
    for (i, ops) in blocks.iter().enumerate() {
        let mut block = ir::Block {
            id: ir::BlockId(i as u16),
            entry_stack: Default::default(),
            ops: Default::default(),
        };
        for op in ops {
            block
                .ops
                .push(ir::Op {
                    kind: *op,
                    span: Span::UNKNOWN,
                })
                .unwrap();
        }
        w.blocks.push(block).unwrap();
    }
    w
}

/// A single-block word for a linear op list.
pub fn synthetic_word(ops: &[OpKind]) -> ir::Word {
    let owned: Vec<OpKind> = ops.to_vec();
    build_word(core::slice::from_ref(&owned), 0, 0)
}

/// The `ir::Output` sink for canonical text.
pub struct VecOut(pub Vec<u8>);

impl ir::Output for VecOut {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// Render `ops` as canonical IR op lines (the exact text `write_word_ops`
/// prints, minus the `"block bN"` headers — the port's `parseOps` consumes
/// this text, so it is the interface).
pub fn canonical_op_text(ops: &[OpKind]) -> String {
    let w = synthetic_word(ops);
    let mut buf = VecOut(Vec::new());
    ir::write_word_ops(&mut buf, &w);
    let text = String::from_utf8_lossy(&buf.0).to_string();
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.starts_with("block ") {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n")
}

pub fn json_esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub fn interval_text(iv: verifier::interval::Interval) -> String {
    match iv {
        verifier::interval::Interval::Bottom => "<bottom>".to_string(),
        verifier::interval::Interval::Top => "<top>".to_string(),
        verifier::interval::Interval::Range { lo, hi } => format!("[{lo},{hi}]"),
    }
}

pub fn intervals_text(ivs: &[verifier::interval::Interval]) -> String {
    let mut parts = Vec::new();
    for iv in ivs {
        parts.push(interval_text(*iv));
    }
    parts.join(";")
}

pub fn tri_name(t: verifier::interval::Tri) -> &'static str {
    match t {
        verifier::interval::Tri::DefTrue => "discharged",
        verifier::interval::Tri::DefFalse => "def-false",
        verifier::interval::Tri::Top => "open",
    }
}

pub fn iv_range(lo: i64, hi: i64) -> verifier::interval::Interval {
    verifier::interval::Interval::Range { lo, hi }
}

// --- the in-tree discharge oracle (P14 review dedup: one home) ------------
//
// Both differential suites (`soundness_differential`, `rederive_differential`)
// compute the in-tree engine's verdict for a generated program; the oracle
// below is that ONE implementation, not a per-file fork.

use verifier::gen::{SUB_HI, SUB_ID, SUB_LO};
use verifier::interp::{ApertureMem, FlatMem, Origin, Slot, State, SubtypeRange};
use verifier::interval::{eval_in_range, Tri};

/// The analysis subtype context both engines share: the generator's
/// `percent`-shaped subtype (`SUB_ID` → `[SUB_LO, SUB_HI]`); every other
/// type id is unrange.
pub fn sr() -> &'static SubtypeRange<'static> {
    &|tid: ir::TypeId| {
        if tid.0 == SUB_ID {
            Some((SUB_LO, SUB_HI))
        } else {
            None
        }
    }
}

/// The in-tree discharge of one generated program: `(tri, interval)` —
/// seed the abstract stack from the input domains, run the transfer over the
/// program's `(TargetSpec, MemModel)` pair, capture the pre-cast value for
/// cast-site programs, evaluate the target range.
pub fn abstract_verdict(prog: &verifier::gen::Program) -> (Tri, verifier::interval::Interval) {
    let mut flat = FlatMem;
    let mut ap: Option<ApertureMem> = None;
    if prog.mem.ram.is_some() || !prog.mem.scripted.is_empty() {
        let mut m = ApertureMem::new(prog.mem.ram.unwrap_or((0, 0)));
        for &(k, v) in prog.mem.scripted.iter() {
            m.script_read(&k, verifier::interval::Interval::const_val(v));
        }
        ap = Some(m);
    }
    let mut st = State::callee_entry(prog.n_inputs(), 4);
    st.stack.clear();
    for &(lo, hi) in &prog.domains {
        st.stack.push(Slot {
            iv: verifier::interval::Interval::Range { lo, hi },
            origin: Origin::Arg(0),
        });
    }
    let mut pre_cast: Option<verifier::interval::Interval> = None;
    for op in &prog.ops {
        if matches!(op, OpKind::Cast { to, .. } if to.0 == SUB_ID) {
            pre_cast = Some(st.top_interval());
        }
        match &mut ap {
            Some(m) => st.step(op, sr(), prog.spec, m),
            None => st.step(op, sr(), prog.spec, &mut flat),
        }
    }
    let (tri, val) = if prog.cast_site {
        let v = pre_cast.unwrap_or(verifier::interval::Interval::TOP);
        (eval_in_range(v, prog.target.0, prog.target.1), v)
    } else {
        let v = st.top_interval();
        (eval_in_range(v, prog.target.0, prog.target.1), v)
    };
    (tri, val)
}
