// Shared test harness for checker_unit tests.
// build_ir_word uses large stack frames (~80KB with Word + IrWordGen).
// Each test runs on an 8MB thread to avoid stack overflow on the default
// 2MB Rust test thread stack.

pub use frontend::fixed::FixedVec;
pub use semantics::typecheck::db::{IsoDb, NominalDb, ResourceDb, SubtypeInfo};
pub use semantics::typecheck::irgen::{arena, build_ir_word, NullObserver};
pub use semantics::typecheck::mmio::MmioDb;
pub use semantics::typecheck::ChecksMode;
pub use semantics::types::{TypeAtom, WordEntry, WordSig};

use frontend::parse::{DeclAst, DeclKind};
use frontend::span::Span;
use ir::{CapSet, EffectSet, StackBound};

pub fn ta(bytes: &[u8]) -> TypeAtom {
    TypeAtom::new(bytes).unwrap()
}

pub fn sig(inp: &[&[u8]], out: &[&[u8]]) -> WordSig {
    let mut s = WordSig::empty();
    s.in_len = inp.len() as u8;
    s.out_len = out.len() as u8;
    for (i, &b) in inp.iter().enumerate() {
        s.inputs[i] = ta(b);
    }
    for (i, &b) in out.iter().enumerate() {
        s.outputs[i] = ta(b);
    }
    s
}

pub fn entry(name: &[u8], inp: &[&[u8]], out: &[&[u8]]) -> WordEntry {
    WordEntry {
        name: ta(name),
        sig: sig(inp, out),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }
}

pub fn empty_env() -> [WordEntry; 256] {
    [WordEntry {
        name: ta(b""),
        sig: WordSig::empty(),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
    }; 256]
}

pub fn make_decl(body: &str) -> (DeclAst, Vec<u8>) {
    let mut src = Vec::new();
    src.extend_from_slice(b"main");
    let name_span = Span::new(0, src.len());
    src.push(b'\n');
    let body_start = src.len();
    src.extend_from_slice(body.as_bytes());
    let body_span = Span::new(body_start, src.len());
    let decl = DeclAst {
        kind: DeclKind::Word,
        name: name_span,
        sig: None,
        attrs: FixedVec::new(),
        body: Some(body_span),
        requires: None,
        ensures: None,
        cap_set: None,
        effect_bits: 0,
        effect_net: 0,
        effect_high: 0,
    };
    (decl, src)
}

pub fn builtin_env() -> ([WordEntry; 256], usize) {
    let mut env = empty_env();
    let mut len = 0usize;
    macro_rules! add {
        ($name:expr, $inp:expr, $out:expr) => {{
            env[len] = entry($name, $inp, $out);
            len += 1;
        }};
    }
    add!(b"dup", &[b"i64"], &[b"i64", b"i64"]);
    add!(b"drop", &[b"i64"], &[]);
    add!(b"swap", &[b"i64", b"i64"], &[b"i64", b"i64"]);
    add!(b"+", &[b"i64", b"i64"], &[b"i64"]);
    add!(b"-", &[b"i64", b"i64"], &[b"i64"]);
    add!(b"*", &[b"i64", b"i64"], &[b"i64"]);
    add!(b">", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"<", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"==", &[b"i64", b"i64"], &[b"bool"]);
    add!(b"and", &[b"bool", b"bool"], &[b"bool"]);
    add!(b"or", &[b"bool", b"bool"], &[b"bool"]);
    add!(b"not", &[b"bool"], &[b"bool"]);
    (env, len)
}

// ---------------------------------------------------------------------------
// Stack-heavy checkers, each runs on its own 8MB thread
// ---------------------------------------------------------------------------

struct CheckOkInput {
    body: String,
    inputs: Vec<Vec<u8>>,
    outputs: Vec<Vec<u8>>,
    env: Vec<WordEntry>,
}

pub fn check_ok(body: &str, inputs: &[&[u8]], outputs: &[&[u8]], env: &[WordEntry]) {
    let inp = CheckOkInput {
        body: body.to_string(),
        inputs: inputs.iter().map(|b| b.to_vec()).collect(),
        outputs: outputs.iter().map(|b| b.to_vec()).collect(),
        env: env.to_vec(),
    };
    let handle = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&inp.body);
            let s = sig(
                &inp.inputs.iter().map(|v| v.as_slice()).collect::<Vec<_>>(),
                &inp.outputs.iter().map(|v| v.as_slice()).collect::<Vec<_>>(),
            );
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb { maps: FixedVec::new(), instances: FixedVec::new() };
            let resources = ResourceDb { items: FixedVec::new() };
            let nominals = NominalDb { structs: FixedVec::new(), enums: FixedVec::new() };
            let iso = IsoDb { types: FixedVec::new() };
            let subtypes: &[SubtypeInfo] = &[];
            let mut obs = NullObserver;
            let result = build_ir_word(
                &decl, &src, &inp.env, subtypes, &mmio, &resources,
                &nominals, &iso, ChecksMode::All, false, &s,
                &mut arena, &mut obs,
            );
            if let Err(e) = result {
                panic!(
                    "expected OK, got error code {} span={:?}",
                    e.code(),
                    e.span()
                );
            }
        })
        .unwrap();
    handle.join().unwrap();
}

struct CheckErrInput {
    body: String,
    inputs: Vec<Vec<u8>>,
    outputs: Vec<Vec<u8>>,
    env: Vec<WordEntry>,
    expected_code: u32,
}

pub fn check_err(
    body: &str,
    inputs: &[&[u8]],
    outputs: &[&[u8]],
    env: &[WordEntry],
    expected_code: u32,
) {
    let inp = CheckErrInput {
        body: body.to_string(),
        inputs: inputs.iter().map(|b| b.to_vec()).collect(),
        outputs: outputs.iter().map(|b| b.to_vec()).collect(),
        env: env.to_vec(),
        expected_code,
    };
    let handle = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&inp.body);
            let s = sig(
                &inp.inputs.iter().map(|v| v.as_slice()).collect::<Vec<_>>(),
                &inp.outputs.iter().map(|v| v.as_slice()).collect::<Vec<_>>(),
            );
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb { maps: FixedVec::new(), instances: FixedVec::new() };
            let resources = ResourceDb { items: FixedVec::new() };
            let nominals = NominalDb { structs: FixedVec::new(), enums: FixedVec::new() };
            let iso = IsoDb { types: FixedVec::new() };
            let subtypes: &[SubtypeInfo] = &[];
            let mut obs = NullObserver;
            let err = match build_ir_word(
                &decl, &src, &inp.env, subtypes, &mmio, &resources,
                &nominals, &iso, ChecksMode::All, false, &s,
                &mut arena, &mut obs,
            ) {
                Ok(_) => panic!("expected error for body: {:?}", inp.body),
                Err(e) => e,
            };
            assert_eq!(
                err.code(),
                inp.expected_code,
                "error code mismatch for body: {:?}",
                inp.body,
            );
        })
        .unwrap();
    handle.join().unwrap();
}
