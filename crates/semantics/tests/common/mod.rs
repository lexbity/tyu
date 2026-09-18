// Shared test harness for checker_unit tests.
// build_ir_word uses large stack frames (~80KB with Word + IrWordGen).
// Each test runs on an 8MB thread to avoid stack overflow on the default
// 2MB Rust test thread stack.

pub use frontend::fixed::FixedVec;
pub use semantics::typecheck::db::{IsoDb, NominalDb, ResourceDb, SubtypeInfo};
pub use semantics::typecheck::irgen::{arena, build_ir_word, NullObserver};
pub use semantics::typecheck::mmio::{AccessMode, MmioDb};
pub use semantics::typecheck::ChecksMode;
pub use semantics::types::{TypeAtom, WordEntry, WordSig};

use frontend::parse::{DeclAst, DeclKind};
use frontend::span::Span;
use ir::{CapSet, EffectSet, StackBound, Word};
use semantics::typecheck::mmio::MmioFieldInfo;

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
        has_explicit_performs: false,
    };
    (decl, src)
}

pub fn builtin_env() -> ([WordEntry; 256], usize) {
    let mut env = empty_env();
    let mut len = 0usize;
    for w in semantics::typecheck::builtin_words() {
        if len >= env.len() {
            break;
        }
        env[len] = *w;
        len += 1;
    }
    (env, len)
}

// ---------------------------------------------------------------------------
// Dbs — semantic database builder
// ---------------------------------------------------------------------------

pub struct Dbs {
    pub subtypes: Vec<SubtypeInfo>,
}

impl Dbs {
    pub fn new() -> Self {
        Dbs {
            subtypes: Vec::new(),
        }
    }

    pub fn with_subtype(mut self, name: &[u8], base: &[u8], min: i64, max: i64) -> Self {
        self.subtypes.push(SubtypeInfo {
            name: TypeAtom::new(name).unwrap(),
            base: TypeAtom::new(base).unwrap(),
            min,
            max,
        });
        self
    }
}

// ---------------------------------------------------------------------------
// check() — run the typechecker, assertions run inside the 8MB-stack thread.
// ---------------------------------------------------------------------------

/// Run the typechecker with given body/inputs/outputs/env/dbs/checks.
/// The `on_word` callback runs inside the big-stack thread with access to
/// the built IR `Word` (on success).  On error the error code is returned.
pub fn check(
    body: &str,
    inputs: &[&[u8]],
    outputs: &[&[u8]],
    env: &[WordEntry],
    dbs: &Dbs,
    checks: ChecksMode,
    on_result: impl FnOnce(Result<(), u32>) + Send + 'static,
) {
    let body_owned = body.to_string();
    let inputs_owned: Vec<Vec<u8>> = inputs.iter().map(|b| b.to_vec()).collect();
    let outputs_owned: Vec<Vec<u8>> = outputs.iter().map(|b| b.to_vec()).collect();
    let env_owned: Vec<WordEntry> = env.to_vec();
    let subtypes_owned: Vec<SubtypeInfo> = dbs.subtypes.clone();

    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&body_owned);
            let s = sig(
                &inputs_owned
                    .iter()
                    .map(|v| v.as_slice())
                    .collect::<Vec<_>>(),
                &outputs_owned
                    .iter()
                    .map(|v| v.as_slice())
                    .collect::<Vec<_>>(),
            );
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb {
                maps: FixedVec::new(),
                instances: FixedVec::new(),
                reg_meta: FixedVec::new(),
            };
            let resources = ResourceDb {
                items: FixedVec::new(),
            };
            let nominals = NominalDb {
                structs: FixedVec::new(),
                enums: FixedVec::new(),
            };
            let iso = IsoDb {
                types: FixedVec::new(),
            };
            let mut obs = NullObserver;
            match build_ir_word(
                &decl,
                &src,
                &env_owned,
                &subtypes_owned,
                &mmio,
                None,
                &resources,
                &nominals,
                &iso,
                checks,
                false,
                &s,
                &mut arena,
                &mut obs,
            ) {
                Ok(_out_words) => on_result(Ok(())),
                Err(e) => on_result(Err(e.code())),
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

pub fn check_ok(body: &str, inputs: &[&[u8]], outputs: &[&[u8]], env: &[WordEntry]) {
    let dbs = Dbs::new();
    check(
        body,
        inputs,
        outputs,
        env,
        &dbs,
        ChecksMode::All,
        |result| match result {
            Ok(()) => {}
            Err(code) => panic!("expected OK, got error code {code}"),
        },
    );
}

/// Like `check_ok` but also invokes `on_word` with the built IR word
/// so callers can assert IR shape (op kinds, block count, etc.).
/// The callback runs inside the 8MB-stack thread.
pub fn check_ok_with<F>(
    body: &str,
    inputs: &[&[u8]],
    outputs: &[&[u8]],
    env: &[WordEntry],
    on_word: F,
) where
    F: FnOnce(&Word) + Send + 'static,
{
    let body_owned = body.to_string();
    let inputs_owned: Vec<Vec<u8>> = inputs.iter().map(|b| b.to_vec()).collect();
    let outputs_owned: Vec<Vec<u8>> = outputs.iter().map(|b| b.to_vec()).collect();
    let env_owned: Vec<WordEntry> = env.to_vec();

    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let (decl, src) = make_decl(&body_owned);
            let s = sig(
                &inputs_owned
                    .iter()
                    .map(|v| v.as_slice())
                    .collect::<Vec<_>>(),
                &outputs_owned
                    .iter()
                    .map(|v| v.as_slice())
                    .collect::<Vec<_>>(),
            );
            let mut arena = arena::ArenaAllocator::new();
            let mmio = MmioDb {
                maps: FixedVec::new(),
                instances: FixedVec::new(),
                reg_meta: FixedVec::new(),
            };
            let resources = ResourceDb {
                items: FixedVec::new(),
            };
            let nominals = NominalDb {
                structs: FixedVec::new(),
                enums: FixedVec::new(),
            };
            let iso = IsoDb {
                types: FixedVec::new(),
            };
            let subtypes: &[SubtypeInfo] = &[];
            let mut obs = NullObserver;
            match build_ir_word(
                &decl,
                &src,
                &env_owned,
                subtypes,
                &mmio,
                None,
                &resources,
                &nominals,
                &iso,
                ChecksMode::All,
                false,
                &s,
                &mut arena,
                &mut obs,
            ) {
                Ok(out_words) => on_word(out_words.word),
                Err(e) => panic!(
                    "expected OK, got error code {} span={:?}",
                    e.code(),
                    e.span()
                ),
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

pub fn check_err(
    body: &str,
    inputs: &[&[u8]],
    outputs: &[&[u8]],
    env: &[WordEntry],
    expected_code: u32,
) {
    let dbs = Dbs::new();
    check(
        body,
        inputs,
        outputs,
        env,
        &dbs,
        ChecksMode::All,
        move |result| match result {
            Ok(_) => panic!("expected error {expected_code}, got Ok"),
            Err(code) => assert_eq!(code, expected_code),
        },
    );
}
