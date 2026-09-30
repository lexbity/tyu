//! The REPL session model (PLAN-WASM-1 §Q3/DEC-3..DEC-6, §7.2/§7.3): an
//! accumulating-module session with a persistent data stack and heap, and
//! **transactional** phrase execution — any phrase fault (trap / fuel /
//! internal error) rolls the stack+frames+heap back to the phrase-entry
//! checkpoint via the undo journal (§7.3), while already-emitted output stays
//! (§Q3 DEC-6). Definitions commit on compile success regardless of phrase
//! outcome ("definitions survive phrase fault").
//!
//! This is the first REPL surface entirely inside the crate: `repl-cli`
//! (P3) drives it the way the page (P5) will; `test-goldens/repl/` pins the
//! transcripts byte-exactly (G56).

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use frontend::span::Span;
use ir::TrapCode;

use crate::compile::{compile, Compiled, SourceTable};
use crate::diag::code;
use crate::engine::{Engine, ExecState, Outcome};
use crate::fuel::Fuel;
use crate::heap::RegionHeap;
use crate::journal::Effect;
use crate::periph::Disabled;
use crate::phrase::{self, EffectSig};
use crate::trace::Trace;
use crate::value::Slot;

/// Session limits (§5, NFR-4), each surfaced as its §7.5 diagnostic. The
/// data-stack/depth caps live in the engine (`crate::fuel`, L2).
const SOURCE_CAP: usize = 65_536; // session source (accumulated declarations)

/// Session options.
#[derive(Clone, Copy, Debug)]
pub struct SessionOpts {
    /// Default per-phrase fuel; `0` = unlimited on host/CI (E9151 on wasm).
    pub fuel: Fuel,
    /// L1 guard elision (§Q4); default off in the REPL.
    pub elide_guards: bool,
    /// The target identity the session compiles/runs at (DEC-8).
    pub spec: verifier::target::TargetSpec,
}

impl Default for SessionOpts {
    fn default() -> Self {
        SessionOpts {
            fuel: Fuel::new(crate::fuel::DEFAULT_FUEL),
            elide_guards: false,
            spec: crate::compile::X86_64_LINUX_SPEC,
        }
    }
}

/// A submit's outcome status.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Compiled (and, if a phrase was present, the phrase committed).
    Ok,
    /// The submission did not compile; the session is untouched.
    CompileError { diag: crate::Diag },
    /// The phrase ran and trapped (rollback applied; output retained).
    Trapped { code: TrapCode, span: Span },
    /// The phrase exhausted its fuel (rollback applied; output retained;
    /// raise `:fuel` and re-run).
    FuelExhausted,
    /// `testio.exit` terminated the run deliberately (session survives).
    Exited { code: i64 },
}

/// One stack cell for the UI (§6.2 `StackCell`: type name + hex value).
#[derive(Clone, Debug, PartialEq)]
pub struct StackCell {
    pub ty_name: Vec<u8>,
    pub value_hex: Vec<u8>,
}

/// The result of one `transactional_submit`.
#[derive(Clone, Debug)]
pub struct SubmitResult {
    pub status: Status,
    /// Byte range appended to `out_total` in this submit (incremental UI).
    pub output_new: Range<u32>,
    /// The persistent stack after the submit (bottom-first).
    pub stack: Vec<StackCell>,
    /// The last-good `--emit=ir` render (present iff the session has ever
    /// compiled; on compile failure it shows the last good IR — DEC-13).
    pub ir_text: Option<Vec<u8>>,
    /// The phrase's normalized `tyu.trace/1` trace (empty when no phrase).
    pub trace: Option<Trace>,
}

/// The REPL session (the data model of §5): the accumulated declarations
/// source, the last-good compiled artifact, the persistent stack/heap/output,
/// and the session options.
pub struct ReplSession {
    source: Vec<u8>,
    compiled: Option<Compiled>,
    stack: Vec<Slot>,
    heap: RegionHeap,
    out_total: Vec<u8>,
    fuel: Fuel,
    opts: SessionOpts,
}

/// The phrase-entry checkpoint (§7.3): the stack before the phrase (the
/// rollback restores it exactly) and the fuel at entry.
struct Checkpoint {
    stack: Vec<Slot>,
    fuel: Fuel,
}

impl ReplSession {
    pub fn new(opts: SessionOpts) -> ReplSession {
        ReplSession {
            source: Vec::new(),
            compiled: None,
            stack: Vec::new(),
            heap: RegionHeap::new(),
            out_total: Vec::new(),
            fuel: opts.fuel,
            opts,
        }
    }

    /// The accumulated declarations source (the session buffer).
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// The last-good compiled artifact (None before the first success).
    pub fn compiled(&self) -> Option<&Compiled> {
        self.compiled.as_ref()
    }

    /// The full accumulated output bytes.
    pub fn out_total(&self) -> &[u8] {
        &self.out_total
    }

    /// Raise/lower the per-phrase fuel budget (`0` = unlimited on host/CI).
    pub fn set_fuel(&mut self, ops: u64) {
        self.fuel = Fuel::new(if ops == 0 { u64::MAX } else { ops });
    }

    /// Toggle L1 guard elision (§Q4); default off in the REPL.
    pub fn set_elide_guards(&mut self, on: bool) {
        self.opts.elide_guards = on;
    }

    /// The current default fuel budget.
    pub fn fuel(&self) -> Fuel {
        self.fuel
    }

    /// Abandon the session: back to Fresh (§4.2).
    pub fn reset(&mut self) {
        self.source.clear();
        self.compiled = None;
        self.stack.clear();
        self.heap = RegionHeap::new();
        self.out_total.clear();
        self.fuel = self.opts.fuel;
    }

    /// Execute a submission per §7.2/§7.3 (DEC-3..DEC-6).
    pub fn transactional_submit(
        &mut self,
        submission: &[u8],
        search: &[&dyn SourceTable],
    ) -> SubmitResult {
        let split = split_submission(submission);
        if !split.has_content {
            // Comment-only / empty submission: no-op re-render (DEC-3).
            return self.result_for(Status::Ok, None, 0..0, None);
        }

        // -- transaction #1: compile (all-or-nothing) -------------------------
        let candidate_len = self.source.len() + split.decls.as_ref().map_or(0, |d| d.len());
        if candidate_len > SOURCE_CAP {
            return self.result_for(
                Status::CompileError {
                    diag: crate::Diag::from_bytes(
                        code::E9154_SOURCE_CAP,
                        Span::UNKNOWN,
                        b"session source exceeds the 64 KiB cap",
                    ),
                },
                None,
                0..0,
                None,
            );
        }

        let entry: &[u8] = b"repl-phrase";
        let decls_src: Vec<u8> = {
            let mut t = alloc::string::String::with_capacity(candidate_len + 16);
            t.push_str("module repl-session;\n");
            t.push_str(core::str::from_utf8(&self.source).unwrap_or(""));
            if let Some(decls) = split.decls.as_deref() {
                t.push_str(core::str::from_utf8(decls).unwrap_or(""));
            }
            t.push_str("end;\n");
            t.into_bytes()
        };

        // The phrase's exact signature must be declared (the checker
        // verifies, never infers): compute it with the §7.2 effect pass. When
        // a phrase names a word defined in THIS submission, the lookup needs
        // the decls compiled first — compile the decls-only module (phase A),
        // then the full module (phase B). A decls-only failure is a plain
        // CompileError (definitions are all-or-nothing, DEC-3).
        let mut phrase_sig: Option<phrase::EffectSig> = None;
        if let Some(phrase) = split.phrase.as_deref() {
            if !phrase.is_empty() {
                let lookup_env: Option<Compiled> = if split.decls.is_some() {
                    // Phase A: validate + build the lookup dictionary.
                    let built = match compile(decls_src.as_slice(), search, self.opts.spec) {
                        Ok(b) => Some(b),
                        Err(diag) => {
                            return self.result_for(
                                Status::CompileError { diag },
                                None,
                                0..0,
                                None,
                            );
                        }
                    };
                    built
                } else {
                    self.compiled.clone()
                };
                phrase_sig = phrase::phrase_effect(phrase, |name| match &lookup_env {
                    Some(c) if c.module.words.iter().any(|w| w.name.as_bytes() == name) => {
                        Some(lookup_word_in(c, name))
                    }
                    _ => self.lookup_word(name),
                });
            }
        }

        let mut module_text = alloc::string::String::with_capacity(candidate_len + 64);
        module_text.push_str("module repl-session;\n");
        module_text.push_str(core::str::from_utf8(&self.source).unwrap_or(""));
        if let Some(decls) = split.decls.as_deref() {
            module_text.push_str(core::str::from_utf8(decls).unwrap_or(""));
        }
        let mut phrase_present = false;
        if let Some(phrase) = split.phrase.as_deref() {
            if !phrase.is_empty() {
                phrase_present = true;
                module_text.push_str(": repl-phrase ");
                if let Some(sig) = &phrase_sig {
                    module_text.push_str(&sig_text(sig));
                }
                module_text.push(' ');
                module_text.push_str(core::str::from_utf8(phrase).unwrap_or(""));
                module_text.push(' ');
                module_text.push(';');
                module_text.push('\n');
            }
        }
        module_text.push_str("end;\n");

        let built = match compile(module_text.as_bytes(), search, self.opts.spec) {
            Ok(b) => b,
            Err(diag) => {
                // Session untouched (DEC-3: compile errors are not edits).
                return self.result_for(Status::CompileError { diag }, None, 0..0, None);
            }
        };

        // Commit the definitions (transaction #1 success).
        if let Some(decls) = split.decls.as_deref() {
            self.source.extend_from_slice(decls);
        }
        let mut rendered = Vec::new();
        crate::render_ir(&mut VecSink(&mut rendered), &built.module);
        let ir_now = Some(rendered);
        self.compiled = Some(built.clone());

        if !phrase_present {
            // Definitions only: no run (DEC-3); stack unchanged.
            return self.result_for(Status::Ok, ir_now, 0..0, None);
        }

        // -- transaction #2: the phrase (rollback via the undo journal) ------
        let out_start = self.out_total.len() as u32;
        let mut state = ExecState::in_session(
            core::mem::take(&mut self.stack),
            core::mem::take(&mut self.heap),
            self.fuel,
            self.opts.elide_guards,
        );
        state.intern_compiled(&built);
        let snap = Checkpoint {
            stack: state.stack.clone(),
            fuel: state.fuel,
        };
        let mut periph = Disabled;
        let outcome = Engine::new(&built).run(entry, &mut state, &mut periph);

        // The phrase trace is derived from the journal + outcome (§7.4).
        let journal: Vec<Effect> = state.journal.iter().cloned().collect();
        let trace = Some(Trace::from_phrase(&journal, &outcome));

        let (status, phrase_out) = match outcome {
            Outcome::Ok { out, exit_stack: _ } => {
                // Commit: stack/heap persist; output retained (§7.3).
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (Status::Ok, out)
            }
            Outcome::Exited { code, out } => {
                // A deliberate end; the session survives. Commit the state
                // (the program chose to exit — not a fault) and surface the
                // code in Status.
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (Status::Exited { code }, out)
            }
            Outcome::Trap { code, span, out } => {
                rollback(&mut state, &snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (Status::Trapped { code, span }, out)
            }
            Outcome::Fuel { out } => {
                rollback(&mut state, &snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (Status::FuelExhausted, out)
            }
        };
        self.out_total.extend_from_slice(&phrase_out);
        // DEC-6: a trapped phrase's output is retained even as stack/heap roll
        // back — BUT never under the G56 selftest corruption.
        if matches!(status, Status::Trapped { .. }) && !selftest_retain_trap_output() {
            self.out_total.truncate(out_start as usize);
        }
        let out_end = self.out_total.len() as u32;

        self.result_for(status, ir_now, out_start..out_end, trace)
    }

    fn result_for(
        &self,
        status: Status,
        ir_now: Option<Vec<u8>>,
        output_new: Range<u32>,
        trace: Option<Trace>,
    ) -> SubmitResult {
        let ir_text = ir_now.or_else(|| {
            self.compiled.as_ref().map(|c| {
                let mut r = Vec::new();
                crate::render_ir(&mut VecSink(&mut r), &c.module);
                r
            })
        });
        SubmitResult {
            status,
            output_new,
            stack: self.stack_cells(),
            ir_text,
            trace,
        }
    }

    /// The persistent stack as UI cells (§6.2 `StackCell`: `{ty_name,
    /// value_hex}`), bottom-first.
    pub fn stack_cells(&self) -> Vec<StackCell> {
        self.stack
            .iter()
            .map(|s| StackCell {
                ty_name: self.type_name(s.ty),
                value_hex: format_hex(s.val.as_u64()),
            })
            .collect()
    }

    /// Map a slot `TypeId` to its name via the current compiled words’
    /// type tables (best-effort: the first word that names it).
    fn type_name(&self, ty: ir::TypeId) -> Vec<u8> {
        if let Some(c) = &self.compiled {
            for w in c.module.words.iter() {
                if let Some(a) = w.types.get(ty.0 as usize) {
                    return a.as_bytes().to_vec();
                }
            }
        }
        // Built-ins not present in any types table.
        match ty {
            ir::TY_I64 => b"i64".to_vec(),
            ir::TY_BOOL => b"bool".to_vec(),
            ir::TY_STR => b"str".to_vec(),
            ir::TY_PTR => b"ptr".to_vec(),
            ir::TY_PTR_MUT => b"ptr_mut".to_vec(),
            _ => b"?".to_vec(),
        }
    }

    /// Resolve a phrase word's declared effect: session words, then the
    /// interpreter builtins, then the semantics builtin table.
    fn lookup_word(&self, name: &[u8]) -> Option<EffectSig> {
        if let Some(c) = &self.compiled {
            for w in c.module.words.iter() {
                if w.name.as_bytes() == name {
                    let sig = &w.sig;
                    let inputs = (0..sig.in_len as usize)
                        .map(|i| self.type_name(sig.inputs[i]))
                        .collect();
                    let outputs = (0..sig.out_len as usize)
                        .map(|i| self.type_name(sig.outputs[i]))
                        .collect();
                    return Some(EffectSig { inputs, outputs });
                }
            }
        }
        if let Some(sig) = builtin_word_effect(name) {
            return Some(sig);
        }
        for e in semantics::typecheck::builtin_words() {
            if e.name.as_bytes() == name {
                let inputs = (0..e.sig.in_len as usize)
                    .map(|i| e.sig.inputs[i].as_bytes().to_vec())
                    .collect();
                let outputs = (0..e.sig.out_len as usize)
                    .map(|i| e.sig.outputs[i].as_bytes().to_vec())
                    .collect();
                return Some(EffectSig { inputs, outputs });
            }
        }
        None
    }
}

/// The declared effects of the interpreter builtins (§6.3 + region ABI).
fn builtin_word_effect(name: &[u8]) -> Option<EffectSig> {
    fn s(inputs: &[&[u8]], outputs: &[&[u8]]) -> EffectSig {
        EffectSig {
            inputs: inputs.iter().map(|t| t.to_vec()).collect(),
            outputs: outputs.iter().map(|t| t.to_vec()).collect(),
        }
    }
    Some(match name {
        b"testio.write-byte" => s(&[b"i64"], &[]),
        b"testio.write-str" => s(&[b"str"], &[]),
        b"testio.exit" => s(&[b"i64"], &[]),
        b"platform.mem.region-create" => s(&[b"usize"], &[b"Region"]),
        b"platform.mem.region-alloc" => s(&[b"Region", b"usize"], &[b"ptr_mut"]),
        b"platform.mem.region-reset" => s(&[b"Region"], &[]),
        b"platform.mem.region-destroy" => s(&[b"Region"], &[]),
        _ => return None,
    })
}

/// Resolve a word's declared effect from a `Compiled` module (the phrase's
/// lookup against the decls-only compile).
fn lookup_word_in(c: &Compiled, name: &[u8]) -> EffectSig {
    let w = c
        .module
        .words
        .iter()
        .find(|w| w.name.as_bytes() == name)
        .expect("lookup_word_in called only for a known word");
    let sig = &w.sig;
    let ty_name = |id: ir::TypeId| -> Vec<u8> {
        w.types
            .get(id.0 as usize)
            .map(|a| a.as_bytes().to_vec())
            .unwrap_or_else(|| b"i64".to_vec())
    };
    let inputs = (0..sig.in_len as usize)
        .map(|i| ty_name(sig.inputs[i]))
        .collect();
    let outputs = (0..sig.out_len as usize)
        .map(|i| ty_name(sig.outputs[i]))
        .collect();
    EffectSig { inputs, outputs }
}

/// Render an effect signature as canonical `( in -- out )` text.
fn sig_text(sig: &EffectSig) -> alloc::string::String {
    let mut out = alloc::string::String::from("( ");
    for t in &sig.inputs {
        out.push_str(core::str::from_utf8(t).unwrap_or("?"));
        out.push(' ');
    }
    out.push_str("-- ");
    if sig.outputs.is_empty() {
        out.pop();
    } else {
        for t in &sig.outputs {
            out.push_str(core::str::from_utf8(t).unwrap_or("?"));
            out.push(' ');
        }
    }
    out.push(')');
    out
}

fn format_hex(v: u64) -> Vec<u8> {
    let mut out = vec![b'0', b'x'];
    let mut digits = [0u8; 16];
    for (i, d) in digits.iter_mut().enumerate() {
        let nib = ((v >> (4 * (15 - i))) & 0xF) as u8;
        *d = if nib < 10 {
            b'0' + nib
        } else {
            b'a' + (nib - 10)
        };
    }
    out.extend_from_slice(&digits);
    out
}

// ---------------------------------------------------------------------------
// §7.3 journal reversal: walk the journal in reverse and apply each
// reversible effect's compensation; restore the stack/fuel checkpoint.
// ---------------------------------------------------------------------------

/// G56 red-path instrument (PLAN-WASM-1 AC-9): under `wasm-selftest` the
/// DEC-6 output-retention asymmetry is corrupted — a trapped phrase's output
/// is DISCARDED instead of retained — so `ci/wasm.sh --selftest` observes the
/// session goldens red. Default builds retain the output (§Q3).
#[cfg(feature = "wasm-selftest")]
fn selftest_retain_trap_output() -> bool {
    false
}

/// See the feature-gated twin above (default: output retained).
#[cfg(not(feature = "wasm-selftest"))]
fn selftest_retain_trap_output() -> bool {
    true
}

fn rollback(state: &mut ExecState, snap: &Checkpoint) {
    for e in state.journal.iter().rev() {
        match e {
            Effect::Store { addr, width, old } => state.heap.compensate_store(*addr, *width, *old),
            Effect::RegionCreate { handle } => state.heap.compensate_region_create(*handle),
            Effect::RegionAlloc { handle, old_off } | Effect::RegionReset { handle, old_off } => {
                state.heap.compensate_cursor(*handle, *old_off);
            }
            Effect::RegionDestroy { handle, saved } => {
                state.heap.compensate_region_destroy(*handle, *saved);
            }
            Effect::ScopedEnter { addr } => state.heap.compensate_scoped(*addr),
            // Trace-only effects are never undone (§Q3 DEC-6).
            Effect::Output { .. } | Effect::Call { .. } => {}
        }
    }
    state.journal.clear();
    state.frames.clear();
    state.stack = snap.stack.clone();
    state.fuel = snap.fuel;
}

// ---------------------------------------------------------------------------
// §7.2 submission splitter
// ---------------------------------------------------------------------------

/// The split of a submission into its declarative region and its trailing
/// phrase.
struct Split {
    /// Any non-comment, non-whitespace content?
    has_content: bool,
    /// Bytes to append to the session source (None = none).
    decls: Option<Vec<u8>>,
    /// The phrase bytes (None = no phrase).
    phrase: Option<Vec<u8>>,
}

/// The module-level keywords that mark a *declarations-only* submission when
/// it has no `:`-definitions (§7.2 — the accommodation lives in the session
/// builder, never the production parser).
const DECL_KEYWORDS: &[&str] = &[
    "import",
    "export",
    "type",
    "subtype",
    "struct",
    "enum",
    "const",
    "resource",
    "register-map",
    "isr",
];

/// Split a submission per §7.2: tokenize; track `:`-definitions at top level;
/// the phrase is the maximal top-level token run after the last definition’s
/// closing `;`. With no definition, a keyword-leading submission is
/// declarations-only; anything else is a phrase.
fn split_submission(src: &[u8]) -> Split {
    let mut i = 0usize;
    let mut depth: u32 = 0;
    let mut in_definition = false;
    let mut last_def_semi_end: Option<usize> = None;
    let mut has_content = false;
    let mut first_keyword: Option<bool> = None; // is the first token a DECL_KEYWORD?

    while i < src.len() {
        let c = src[i];
        // Comments: `#` to end-of-line.
        if c == b'#' {
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        has_content = true;
        match c {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => in_definition = true,
            b';' if depth == 0 && in_definition => {
                last_def_semi_end = Some(i + 1);
                in_definition = false;
            }
            _ if depth == 0 && first_keyword.is_none() => {
                first_keyword = Some(word_is_decl_keyword(src, i));
            }
            _ => {}
        }
        i += 1;
    }

    let Some(def_end) = last_def_semi_end else {
        // No `:`-definition: keyword-leading ⇒ declarations-only; else the
        // whole submission is the phrase (phrase-only edits, §7.2).
        if !has_content {
            return Split {
                has_content: false,
                decls: None,
                phrase: None,
            };
        }
        if first_keyword.unwrap_or(false) {
            return Split {
                has_content: true,
                decls: Some(src.to_vec()),
                phrase: None,
            };
        }
        return Split {
            has_content: true,
            decls: None,
            phrase: Some(trim_bytes(src).to_vec()),
        };
    };

    // Definitions present: the declarations region ends at the last
    // definition’s `;`; everything after is the phrase.
    let decls = src[..def_end].to_vec();
    let phrase = trim_bytes(&src[def_end..]);
    Split {
        has_content: true,
        decls: Some(decls),
        phrase: if phrase.is_empty() {
            None
        } else {
            Some(phrase.to_vec())
        },
    }
}

/// Is the identifier starting at `i` one of the DECL_KEYWORDS?
fn word_is_decl_keyword(src: &[u8], i: usize) -> bool {
    let mut j = i;
    while j < src.len() && (src[j].is_ascii_alphanumeric() || src[j] == b'-') {
        j += 1;
    }
    let w = core::str::from_utf8(&src[i..j]).unwrap_or("");
    DECL_KEYWORDS.contains(&w)
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let mut s = 0usize;
    let mut e = b.len();
    while s < e && (b[s].is_ascii_whitespace() || b[s] == b'\n') {
        s += 1;
    }
    while e > s && (b[e - 1].is_ascii_whitespace() || b[e - 1] == b'\n') {
        e -= 1;
    }
    &b[s..e]
}

struct VecSink<'a>(&'a mut Vec<u8>);
impl frontend::parse::Output for VecSink<'_> {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}
