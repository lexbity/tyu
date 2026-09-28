//! Bundle-instance conformance (PLAN-VERIFY-3 P12.2).
//!
//! The three modeled bare-metal bundles (`x86_64-unknown-none`,
//! `armv7m-unknown-none`, `riscv32-unknown-none`) each ship a model artifact
//! (`model/model.toml`: id + `[memory] ram` window) and an `evidence/`
//! vector corpus. This suite is the **Rust×Lean conformance gate** over that
//! surface (§Q11 item 4 / FR-11 applied to the bundle instances):
//!
//! - the RAM geometry flows out of the artifact (`model/model.toml [memory]
//!   ram`) and must equal the Lean port's instance geometry — pinned against
//!   the port's `--level bundles` report (one source, both sides);
//! - the committed bundle corpus (`platforms/<triple>/evidence/vectors.json`,
//!   a `tyu.vec/1` document with a `"ram"` header) is executed by the Rust
//!   `ApertureMem` instance **and** by the port's `conformance` exe (whose
//!   `MemModel.bundle` mirrors it); any divergence fails closed;
//! - regeneration is deterministic (FR-16 / the golden discipline) and the
//!   committed files must byte-match (the band rule applies to the bundle
//!   surface too).
//!
//! Regenerate with `TYU_REGEN_BUNDLE_VECTORS=1 cargo test -p verifier
//! --test bundle_instance_conformance`.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use common::{
    canonical_op_text, interval_text, intervals_text, iv_range, json_esc, tri_name, workspace_root,
};
use ir::{Atom, OpKind, TypeId};
use verifier::interp::{ApertureMem, State};
use verifier::interval::{eval_in_range, Interval, Tri};
use verifier::target::{TargetSpec, TARGETS};

const VEC_SCHEMA: &str = "tyu.vec/1";
const SUB_ID: u8 = 2;

fn sr() -> &'static verifier::interp::SubtypeRange<'static> {
    &|tid: ir::TypeId| {
        if tid.0 == SUB_ID {
            Some((0, 100))
        } else {
            None
        }
    }
}

/// One modeled bundle: its target triple, its **pack directory** name (the
/// pack dir — `platforms/<triple>` for the per-triple packs, `platforms/<name>`
/// for a multi-ISA board pack like `rp2350`), its model-id label
/// (`tyu.model/<bundle>`), and its **inclusive** modeled RAM window.
///
/// The model artifact declares `ram = { origin, length }` — a half-open
/// region `[origin, origin + length)`. The window the engines actually
/// model is the *inclusive* `[origin, origin + length - 1]` (the artifact's
/// last byte) — the same inclusive end the Lean `Tyu.Bundles` instances and
/// the `ApertureMem` ram pair use; the corpus `"ram"` header carries it
/// verbatim. The `declared` half-open region is kept separately for the
/// geometry-chain test against `model.toml`.
struct Bundle {
    triple: &'static str,
    /// The pack directory under `platforms/`.
    pack: &'static str,
    /// The evidence corpus `"bundle"` label (`tyu.model/<bundle>`).
    label: &'static str,
    /// The inclusive model window `[first, last]` (last = origin + length - 1).
    window: (u64, u64),
    /// The artifact's half-open region `(origin, origin + length)`.
    declared: (u64, u64),
}

impl Bundle {
    fn spec(&self) -> TargetSpec {
        *TARGETS
            .iter()
            .find(|t| t.triple == self.triple)
            .expect("recognized triple")
    }

    /// The pack's evidence corpus dir.
    fn evidence_dir(&self) -> PathBuf {
        workspace_root()
            .join("platforms")
            .join(self.pack)
            .join("evidence")
    }
}

/// The declared bundles (G46/G47's modeled set): geometry via the ONE
/// `[memory] ram` parser (`verifier::bundle::model_memory_ram` — the same
/// parser the tyu accessor and the corpus header use; cleanup item 3). The
/// per-triple packs plus the rp2350 board pack (P13.2 — a multi-ISA board
/// whose default ISA is armv7m; its own `tyu.model/rp2350/1` model id and a
/// window over the first 64 KiB of SRAM).
fn bundles() -> Vec<Bundle> {
    let mut out = Vec::new();
    for (triple, pack, label) in [
        (
            "x86_64-unknown-none",
            "x86_64-unknown-none",
            "tyu.model/x86_64-unknown-none",
        ),
        (
            "armv7m-unknown-none",
            "armv7m-unknown-none",
            "tyu.model/armv7m-unknown-none",
        ),
        (
            "riscv32-unknown-none",
            "riscv32-unknown-none",
            "tyu.model/riscv32-unknown-none",
        ),
        ("armv7m-unknown-none", "rp2350", "tyu.model/rp2350"),
    ] {
        let text = fs::read_to_string(
            workspace_root()
                .join("platforms")
                .join(pack)
                .join("model/model.toml"),
        )
        .expect("model artifact present");
        let declared = verifier::bundle::model_memory_ram(&text)
            .expect("model artifact declares [memory] ram");
        // The inclusive window: the declared half-open region minus the
        // exclusive top byte (model.toml `length` = number of modeled bytes).
        assert!(
            declared.1 > declared.0,
            "{pack}: [memory] ram must be non-empty"
        );
        let window = (declared.0, declared.1 - 1);
        out.push(Bundle {
            triple,
            pack,
            label,
            window,
            declared,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Vectors
// ---------------------------------------------------------------------------

/// Ops for a linear vector; each travels as the canonical op text. The
/// entry stack is separate (`"entry"`), exactly like the shared per-target
/// corpora.
#[derive(Debug, Clone)]
struct Vec1 {
    id: String,
    class: String,
    row: &'static str,
    model: &'static str, // "flat" | "bundle"
    entry: Vec<Interval>,
    ops: Vec<OpKind>,
    target: (i64, i64),
    cast_site: bool,
    expect: (Tri, Interval),
}

impl Vec1 {
    fn run(&self, spec: TargetSpec, ram: (u64, u64)) -> (Tri, Interval) {
        let mut mem = ApertureMem::new(ram);
        let mut st = State::fresh(64);
        for iv in &self.entry {
            st.stack.push(verifier::interp::Slot {
                iv: *iv,
                origin: verifier::interp::Origin::Computed,
            });
        }
        let mut pre_cast: Option<Interval> = None;
        for op in &self.ops {
            if matches!(op, OpKind::Cast { to, .. } if to.0 == SUB_ID) {
                pre_cast = Some(st.top_interval());
            }
            st.step(op, sr(), spec, &mut mem);
        }
        let v = if self.cast_site {
            pre_cast.unwrap_or(Interval::TOP)
        } else {
            st.top_interval()
        };
        (eval_in_range(v, self.target.0, self.target.1), v)
    }
}

fn const_op(v: i64) -> OpKind {
    OpKind::ConstI64(v)
}

fn sub_cast() -> OpKind {
    OpKind::Cast {
        from: TypeId(0),
        to: TypeId(SUB_ID),
    }
}

/// An MMIO aperture read op for the bundle corpus. The **abstract**
/// aperture domain is derived from the target's `word_bits` (the file's
/// `target` record), never from the op payload — so `atomic_max` is set to
/// that same width to keep the decoration *honest* (a 64-bit annotation on
/// a 32-bit vector would misstate the width source; it is inert in the
/// transfer either way).
fn mmio_read(place: &[u8], word_bits: u8) -> OpKind {
    OpKind::MmioVolLoad {
        ty: TypeId(0),
        place: Atom::new(place).unwrap(),
        read_kind: ir::ReadKind::Plain,
        atomic_max: word_bits,
        barrier: ir::BarrierKind::None,
    }
}

/// The bundle corpus vectors: the memory-model classes that require the
/// bundle instance (store/load roundtrips inside the window, the frame
/// property, unmodeled reads) plus the shared data-domain boundary classes
/// and the width-relative aperture class.
#[allow(clippy::vec_init_then_push)] // structured entries read clearer as pushes
fn vectors_for(b: &Bundle) -> Vec<Vec1> {
    let (lo, hi) = b.window;
    let (a, c) = (lo as i64, hi as i64);
    // The first byte below/above the inclusive window (out-of-window probes).
    let other: i64 = if lo > 0 {
        (lo - 1) as i64
    } else {
        (lo + 1) as i64
    };
    let past_hi: i64 = (hi + 1) as i64;
    let mut v = Vec::new();

    // --- T-D store-load: a point store inside the window reads back. -----
    v.push(Vec1 {
        id: "td/store-load-in-window".into(),
        class: "td:store-load".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(a),
            const_op(42),
            OpKind::Store { ty: TypeId(0) },
            const_op(a),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::DefTrue, iv_range(42, 42)),
    });

    // --- T-D store-load: newest store wins (replace semantics). ----------
    v.push(Vec1 {
        id: "td/store-load-replace".into(),
        class: "td:store-load".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(a),
            const_op(42),
            OpKind::Store { ty: TypeId(0) },
            const_op(a),
            const_op(7),
            OpKind::Store { ty: TypeId(0) },
            const_op(a),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::DefTrue, iv_range(7, 7)),
    });

    // --- T-D frame: a store to A leaves the load of B (≠ A) intact. ------
    // B is `c` — a *distinct, in-window* address (the window's last byte);
    // an in-window-unrecorded load reads top before and after the store. A
    // store-smearing defect (A's write bleeding into B's read) would return
    // the stored value here.
    v.push(Vec1 {
        id: "td/frame-unrecorded-other".into(),
        class: "td:frame".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(a),
            const_op(42),
            OpKind::Store { ty: TypeId(0) },
            const_op(c),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });

    // --- T-D frame, recorded-cell form: a store to A must NOT clobber the
    // cell already at B (the actual frame law: B's read is unchanged). B is
    // `a + 1` (in-window, recorded first). ----
    v.push(Vec1 {
        id: "td/frame-preserves-recorded".into(),
        class: "td:frame".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(a + 1),
            const_op(42),
            OpKind::Store { ty: TypeId(0) },
            const_op(a),
            const_op(7),
            OpKind::Store { ty: TypeId(0) },
            const_op(a + 1),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::DefTrue, iv_range(42, 42)),
    });

    // --- T-D store-load, window boundary: the window's *last* declared
    // byte (hi = origin + length − 1) is modeled — a store there reads
    // back. Catches a window that is one byte SHORTER than declared.
    v.push(Vec1 {
        id: "td/boundary-high-byte".into(),
        class: "td:store-load".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(c),
            const_op(9),
            OpKind::Store { ty: TypeId(0) },
            const_op(c),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::DefTrue, iv_range(9, 9)),
    });

    // --- T-D store-load, past the window: the first byte AFTER the declared
    // region (hi + 1 = origin + length) is NOT modeled — a store there has
    // no observable effect. Catches a window that is one byte LONGER than
    // declared (the length+1 off-by-one).
    v.push(Vec1 {
        id: "td/boundary-past-window".into(),
        class: "td:store-load".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(past_hi),
            const_op(9),
            OpKind::Store { ty: TypeId(0) },
            const_op(past_hi),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });

    // --- Unmodeled reads are top (bundle or not). -------------------------
    v.push(Vec1 {
        id: "td/load-unrecorded-in-window".into(),
        class: "td:load".into(),
        row: "load",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![const_op(a), OpKind::Load { ty: TypeId(0) }],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });
    v.push(Vec1 {
        id: "td/load-outside-window".into(),
        class: "td:load".into(),
        row: "load",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![const_op(other), OpKind::Load { ty: TypeId(0) }],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });
    // A store *outside* the window must have no observable effect: it
    // records nothing, so a subsequent load of that address stays top.
    v.push(Vec1 {
        id: "td/store-outside-window-inert".into(),
        class: "td:store".into(),
        row: "store",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![
            const_op(other - 5),
            const_op(1),
            OpKind::Store { ty: TypeId(0) },
            const_op(other - 5),
            OpKind::Load { ty: TypeId(0) },
        ],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });

    // --- §Q13 aperture read: the width-bounded nondeterministic domain. --
    let (dom_lo, dom_hi) = b.spec().signed_domain();
    let aperture_top = if b.spec().word_bits >= 64 {
        Interval::TOP
    } else {
        iv_range(dom_lo, dom_hi)
    };
    v.push(Vec1 {
        id: "td/aperture-width-domain".into(),
        class: "td:aperture-width".into(),
        row: "vol_load",
        model: "bundle",
        entry: Vec::new(),
        ops: vec![mmio_read(b"tdreg", b.spec().word_bits)],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, aperture_top),
    });

    // --- Shared data-domain boundary classes (flat; target-independent). --
    v.push(Vec1 {
        id: "boundary/const-i64-max".into(),
        class: "boundary:i64-max".into(),
        row: "const_i64",
        model: "flat",
        entry: Vec::new(),
        ops: vec![const_op(i64::MAX)],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::DefFalse, iv_range(i64::MAX, i64::MAX)),
    });
    v.push(Vec1 {
        id: "boundary/wrap-add".into(),
        class: "boundary:wrap-add".into(),
        row: "add_i64",
        model: "flat",
        entry: vec![iv_range(i64::MAX - 1, i64::MAX), iv_range(1, 1)],
        ops: vec![OpKind::AddI64],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });
    v.push(Vec1 {
        id: "boundary/sub-wraps-to-top".into(),
        class: "boundary:wrap-sub".into(),
        row: "sub_i64",
        model: "flat",
        entry: vec![iv_range(i64::MIN + 1, i64::MIN), iv_range(2, 4)],
        ops: vec![OpKind::SubI64],
        target: (0, 100),
        cast_site: false,
        expect: (Tri::Top, Interval::TOP),
    });
    v.push(Vec1 {
        id: "boundary/cast-in-range".into(),
        class: "boundary:cast-bounds".into(),
        row: "cast",
        model: "flat",
        entry: vec![iv_range(42, 42)],
        ops: vec![sub_cast()],
        target: (0, 100),
        cast_site: true,
        expect: (Tri::DefTrue, iv_range(42, 42)),
    });
    v.push(Vec1 {
        id: "boundary/cast-trap-above".into(),
        class: "boundary:cast-trap".into(),
        row: "cast",
        model: "flat",
        entry: vec![iv_range(150, 150)],
        ops: vec![sub_cast()],
        target: (0, 100),
        cast_site: true,
        expect: (Tri::DefFalse, iv_range(150, 150)),
    });
    v
}

/// Render the committed bundle corpus document (`tyu.vec/1` + the `"ram"`
/// header). Deterministic: fixed key order, no maps.
fn render_json(b: &Bundle) -> String {
    let spec = b.spec();
    let vectors = vectors_for(b);
    assert!(!vectors.is_empty(), "{} corpus is empty", b.triple);
    let mut c = Vec::new();
    c.push(format!(
        "{{\n  \"schema\": \"{VEC_SCHEMA}\",\n  \"bundle\": \"{}\",\n  \"triple\": \"{}\",\n  \"target\": {{",
        b.label, b.triple
    ));
    c.push(format!(
        "    \"slot_bytes\": {},\n    \"word_bits\": {},\n    \"arch_tag\": {}",
        spec.slot_bytes, spec.word_bits, spec.arch_tag
    ));
    c.push("  },".to_string());
    c.push(format!("  \"ram\": [{}, {}],", b.window.0, b.window.1));
    c.push("  \"vectors\": [".to_string());
    for (i, v) in vectors.iter().enumerate() {
        let comma = if i + 1 < vectors.len() { "," } else { "" };
        c.push("    {".to_string());
        c.push(format!("      \"id\": \"{}\",", json_esc(&v.id)));
        c.push(format!("      \"class\": \"{}\",", json_esc(&v.class)));
        c.push(format!("      \"row\": \"{}\",", v.row));
        c.push(format!("      \"model\": \"{}\",", v.model));
        c.push("      \"mode\": \"linear\",".to_string());
        c.push(format!(
            "      \"ops\": \"{}\",",
            json_esc(&canonical_op_text(&v.ops))
        ));
        c.push(format!(
            "      \"entry\": \"{}\",",
            json_esc(&intervals_text(&v.entry))
        ));
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

fn evidence_index(b: &Bundle) -> PathBuf {
    b.evidence_dir().join("vectors.json")
}

/// Compute each vector's expectation from the engine (the derived path —
/// the corpus's `expect` is pinned by the golden files, exactly like the
/// shared per-target corpora).
fn derived_vectors(b: &Bundle) -> Vec<Vec1> {
    let mut out = vectors_for(b);
    for v in out.iter_mut() {
        let (head, top) = v.run(b.spec(), b.window);
        v.expect = (head, top);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The committed bundle corpora must byte-match regeneration (the golden
/// gate). Regenerate with `TYU_REGEN_BUNDLE_VECTORS=1`.
#[test]
fn bundle_corpus_files_match_regeneration() {
    let mut regen = false;
    for b in bundles() {
        let path = evidence_index(&b);
        if std::env::var("TYU_REGEN_BUNDLE_VECTORS").is_ok() {
            fs::create_dir_all(path.parent().unwrap()).expect("create evidence dir");
            fs::write(&path, render_json(&b)).expect("write bundle corpus");
            regen = true;
        } else {
            let expected = render_json(&b);
            let actual = fs::read_to_string(&path)
                .expect("bundle corpus present (run with TYU_REGEN_BUNDLE_VECTORS=1)");
            assert_eq!(
                actual, expected,
                "bundle corpus for {} drifted from the engine — regenerate + review",
                b.triple
            );
        }
    }
    if regen {
        eprintln!("TYU_REGEN_BUNDLE_VECTORS=1: regenerated all bundle corpora");
    }
}

/// Determinism (FR-16).
#[test]
fn bundle_corpus_regeneration_is_deterministic() {
    for b in bundles() {
        let a = render_json(&b);
        let b2 = render_json(&b);
        assert_eq!(a, b2, "{} corpus must be deterministic", b.triple);
    }
}

/// Every committed vector is reproduced by the Rust `ApertureMem` instance
/// consuming the model artifact's geometry (the Rust leg).
#[test]
fn bundle_corpus_executes_on_rust_aperture_mem() {
    for b in bundles() {
        let text = fs::read_to_string(evidence_index(&b)).expect("corpus present");
        // Re-derive (engine-derived expectations) and compare with the
        // committed document by re-parsing the committed expect fields.
        let committed = parse_committed(&text);
        let derived = derived_vectors(&b);
        assert_eq!(
            committed.len(),
            derived.len(),
            "{}: committed/derived vector counts differ",
            b.triple
        );
        for (cv, dv) in committed.iter().zip(derived.iter()) {
            assert_eq!(cv.0, dv.id, "{}: vector order", b.triple);
            assert_eq!(
                ((cv.1), (cv.2)),
                ((dv.expect.0), (dv.expect.1)),
                "{} [{}]: committed expect vs engine",
                b.triple,
                cv.0
            );
        }
    }
}

/// The Lean leg: the port's `conformance` exe must reproduce every bundle
/// corpus byte-exactly (zero divergence), and — for the same models — the
/// Rust leg above must agree (this test invokes the exe; the Rust agreement
/// is `bundle_corpus_executes_on_rust_aperture_mem`).
#[test]
fn bundle_corpus_matches_lean_conformance() {
    let port_dir = workspace_root().join("verification/ports/lean");
    let exe = port_dir.join(".lake/build/bin/conformance");
    if !exe.is_file() {
        eprintln!("skipping: port conformance exe not built (lake build)");
        return;
    }
    // The exe reads `<dir>/index.json` or `<dir>/vectors.json` (P12.2 fallback
    // for the bundle evidence corpora) — pass the evidence dirs directly.
    let mut args: Vec<String> = vec!["--corpus".to_string()];
    args.extend(
        bundles()
            .iter()
            .map(|b| b.evidence_dir().display().to_string()),
    );

    let out = Command::new(&exe)
        .args(&args)
        .output()
        .expect("conformance run");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() || text.contains("mismatches=0"),
        "bundle conformance failed: {text}"
    );
    let mismatches = text
        .lines()
        .find_map(|l| {
            l.strip_prefix("RESULT: ")
                .and_then(|r| r.split(" ").find_map(|tok| tok.strip_prefix("mismatches=")))
        })
        .map(|v| v.parse::<usize>().unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX);
    assert_eq!(mismatches, 0, "bundle conformance mismatch: {text}");
    for b in bundles() {
        let n = vectors_for(&b).len();
        assert!(
            text.contains(&format!("{} vectors", n))
                && !text.contains(&format!("DIVERGE: {}", b.triple)),
            "bundle conformance did not pass {} corpus: {text}",
            b.triple
        );
    }
}

/// The refinement-band pin (PLAN-VERIFY-3 P13.2, P3 finding): the Lean
/// bundle instance's declared refinement device (`mask`/`mode`, the
/// `--level bands` report) MUST equal the model artifact's
/// `[[refinements.device]]` transcription — the datasheet band is a
/// reviewed artifact-pair (Lean instance + manifest), never prose; and the
/// band the registry consumes (`uartfr_band_domain_at_device`) equals the
/// report's value mechanically.
#[test]
fn refinement_band_matches_lean_instance_and_manifest() {
    let port_dir = workspace_root().join("verification/ports/lean");
    let exe = port_dir.join(".lake/build/bin/conformance");
    if !exe.is_file() {
        eprintln!("skipping: port conformance exe not built (lake build)");
        return;
    }
    let out = Command::new(&exe)
        .arg("--level")
        .arg("bands")
        .output()
        .expect("bands report");
    assert!(out.status.success(), "conformance --level bands failed");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let json: serde_json::Value = serde_json::from_str(&text).expect("tyu.bands/1 parses");
    let devices = json["devices"].as_array().expect("devices array");

    // Every modeled bare pack declares an EMPTY `[refinements]` (no devices)
    // — nothing to pin; the rp2350 board pack declares the one device, which
    // the LEAN instance must report with the manifest's mask/mode.
    for pack in [
        "x86_64-unknown-none",
        "armv7m-unknown-none",
        "riscv32-unknown-none",
    ] {
        let artifact = fs::read_to_string(
            workspace_root()
                .join("platforms")
                .join(pack)
                .join("model/model.toml"),
        )
        .unwrap();
        assert!(
            refinement_devices(&artifact).is_empty(),
            "{pack}: bare modeled packs carry no refinement device"
        );
    }
    let rp =
        fs::read_to_string(workspace_root().join("platforms/rp2350/model/model.toml")).unwrap();
    let declared = refinement_devices(&rp);
    assert_eq!(
        declared.len(),
        1,
        "rp2350 declares exactly one refinement device"
    );

    // The report carries the rp2350 entry with the manifest's band + mode.
    let entry = devices
        .iter()
        .find(|d| d["id"].as_str().unwrap_or("") == "rp2350.uart-fr")
        .expect("rp2350.uart-fr present in the bands report");
    let (mask, mode) = &declared[0];
    assert_eq!(*mask, 0xF9, "the datasheet band (P3 finding)");
    assert_eq!(mode.as_str(), "ro", "the UARTFR access-mode (P3 finding)");
    assert_eq!(entry["mask"].as_i64().unwrap() as u64, *mask as u64);
    assert_eq!(entry["mode"].as_str().unwrap_or(""), mode.as_str());
}

/// The `[[refinements.device]]` transcription of a `model/model.toml`: the
/// `(mask, mode)` pairs, in declaration order (a minimal scanner for the
/// committed shape — the same shape the tyu lint validates strictly).
fn refinement_devices(text: &str) -> Vec<(u32, String)> {
    let mut in_refinements = false;
    let mut mask: Option<u32> = None;
    let mut mode: Option<String> = None;
    let mut out = Vec::new();
    let flush =
        |mask: &mut Option<u32>, mode: &mut Option<String>, out: &mut Vec<(u32, String)>| {
            if let (Some(m), Some(mo)) = (mask.take(), mode.take()) {
                out.push((m, mo));
            }
        };
    for line in text.lines() {
        let t = line.trim();
        if t == "[refinements]" {
            in_refinements = true;
            continue;
        }
        if !in_refinements {
            continue;
        }
        if t.starts_with("[[refinements.device]]") {
            flush(&mut mask, &mut mode, &mut out);
            continue;
        }
        if let Some(v) = t.strip_prefix("mask =") {
            mask = parse_int_literal(v.trim());
        } else if let Some(v) = t.strip_prefix("mode =") {
            mode = Some(v.trim().trim_matches('"').to_string());
        }
    }
    flush(&mut mask, &mut mode, &mut out);
    out
}

/// Parse `0x…` or a decimal literal into a u32.
fn parse_int_literal(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

/// The Lean instance name for each modeled bundle (the `Tyu.Bundles` defs
/// the geometry report prints). Keyed by PACK name — the rp2350 board pack
/// is its own instance alongside the per-triple packs.
fn instance_name(pack: &str) -> &str {
    match pack {
        "x86_64-unknown-none" => "x86_64",
        "armv7m-unknown-none" => "armv7m",
        "riscv32-unknown-none" => "riscv32",
        "rp2350" => "rp2350",
        other => panic!("no Lean instance for pack {other}"),
    }
}

/// The geometry chain: the model artifact's `[memory] ram` window must equal
/// the Lean bundle instance's window (the port's `--level bundles` report).
#[test]
fn bundle_geometry_matches_lean_instance() {
    let port_dir = workspace_root().join("verification/ports/lean");
    let exe = port_dir.join(".lake/build/bin/conformance");
    if !exe.is_file() {
        eprintln!("skipping: port conformance exe not built (lake build)");
        return;
    }
    let out = Command::new(&exe)
        .args(["--level", "bundles"])
        .output()
        .expect("bundles geometry report");
    assert!(out.status.success(), "conformance --level bundles failed");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let json: serde_json::Value = serde_json::from_str(&text).expect("tyu.bind/1 report parses");
    let bundles_json = json["bundles"].as_array().expect("bundles array");
    let by_name: std::collections::BTreeMap<&str, &serde_json::Value> = bundles_json
        .iter()
        .map(|v| (v["id"].as_str().unwrap_or(""), v))
        .collect();
    for b in bundles() {
        let instance = by_name
            .get(instance_name(b.pack))
            .expect("instance present in report");
        let lo = instance["ramLo"].as_i64().expect("ramLo");
        let hi = instance["ramHi"].as_i64().expect("ramHi");
        // The Lean instances carry the *inclusive* window ([origin,
        // origin+length-1]); the model artifact declares the half-open
        // region (origin, origin+length). hi+1 must equal the exclusive end.
        assert_eq!(
            (lo as u64, hi as u64 + 1),
            b.declared,
            "{}: model.toml geometry != Lean instance geometry",
            b.triple
        );
        // And the corpus header (b.window) is exactly the instance window.
        assert_eq!(
            (lo as u64, hi as u64),
            b.window,
            "{}: corpus ram header != Lean instance geometry",
            b.triple
        );
    }
}

// ---------------------------------------------------------------------------
// Minimal committed-corpus reader (the `expect` fields, for the Rust leg).
// ---------------------------------------------------------------------------

/// Parse the committed corpus's `expect` fields per vector id.
fn parse_committed(text: &str) -> Vec<(String, Tri, Interval)> {
    let json: serde_json::Value = serde_json::from_str(text).expect("corpus parses");
    let vectors = json["vectors"].as_array().expect("vectors array");
    vectors
        .iter()
        .map(|v| {
            let id = v["id"].as_str().unwrap_or("").to_string();
            let head = v["expect"]["head"].as_str().unwrap_or("");
            let top = v["expect"]["top"].as_str().unwrap_or("");
            (
                id,
                match head {
                    "discharged" => Tri::DefTrue,
                    "def-false" => Tri::DefFalse,
                    _ => Tri::Top,
                },
                parse_interval(top),
            )
        })
        .collect()
}

fn parse_interval(s: &str) -> Interval {
    match s {
        "<top>" => Interval::TOP,
        "<bottom>" => Interval::BOTTOM,
        _ => {
            let inner = s.trim_start_matches('[').trim_end_matches(']');
            let mut parts = inner.split(',');
            let lo: i64 = parts.next().unwrap_or("0").parse().expect("lo");
            let hi: i64 = parts.next().unwrap_or("0").parse().expect("hi");
            iv_range(lo, hi)
        }
    }
}
