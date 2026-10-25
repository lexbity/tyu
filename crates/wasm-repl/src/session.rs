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

use alloc::boxed::Box;
use alloc::vec::Vec;
use codegen_core::compiled_desc::CompiledDescriptor;
use core::ops::Range;
use frontend::lex::Lexer;
use frontend::span::Span;
use frontend::token::{Token, TokenKind};
use ir::TrapCode;

use crate::compile::{compile, Compiled, SourceTable};
use crate::diag::code;
use crate::engine::{Engine, ExecState, Outcome, TrapDetail};
use crate::fuel::Fuel;
use crate::heap::RegionHeap;
use crate::journal::{Effect, Journal};
use crate::periph::{Disabled, Peripheral};
use crate::phrase::{self, EffectSig};
use crate::trace::{Trace, TraceHeader};
use crate::value::{format_hex, Slot};

/// Session limits (§5, NFR-4), each surfaced as its §7.5 diagnostic. The
/// data-stack/depth caps live in the engine (`crate::fuel`, L2).
const SOURCE_CAP: usize = 65_536; // session source (accumulated declarations)
/// The session = one module, and the IR format caps a module at 64 words
/// (`ir::Module`'s format contract). Native `build_ir` enforces it with
/// E3920; the session boundary pre-checks it with E9157 (§7.5).
const MODULE_WORD_CAP: usize = 64;

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
    /// The phrase ran and trapped (rollback applied; output retained). A
    /// contract failure (trap 20) carries the §Q6/DEC-22 `TrapDetail`; every
    /// other trap's detail is absent.
    Trapped {
        code: TrapCode,
        span: Span,
        detail: Option<TrapDetail>,
    },
    /// The phrase exhausted its fuel (rollback applied; output retained).
    /// The phrase is **suspended** at the exhaustion point: [`ReplSession::step`]
    /// resumes it with a fresh budget (E9150 carries the suspended op's span).
    FuelExhausted { diag: crate::Diag },
    /// An engine-internal fault (E9158, or E9156's runtime backstop): a
    /// state the verified IR cannot reach. Always a bug — rollback applied,
    /// the page fails loudly (§6.5).
    Internal { diag: crate::Diag },
    /// `testio.exit` terminated the run deliberately (session survives).
    Exited { code: i64 },
    /// A device-level failure (P4): the peripheral reported an unbound
    /// aperture (E9152), carrying the aperture id when known. Rollback
    /// applied; output before the fault retained.
    DeviceError {
        code: u32,
        span: Span,
        aperture: Option<u16>,
    },
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
    /// The device boundary (P4, §7.3): the sole nondeterminism source. The
    /// session defaults to [`Disabled`] (any MMIO access → E9152); a
    /// scripted/board device is attached with [`ReplSession::set_peripheral`].
    periph: Box<dyn Peripheral>,
    /// The compiled platform descriptor (P4): MMIO modules compile only when
    /// it is present (E3640 parity otherwise); its aperture table resolves
    /// MMIO addresses and drives the session's IR pane.
    descriptor: Option<Box<CompiledDescriptor>>,
    /// A fuel-suspended phrase (§Q6/§Q8): the at-exhaustion engine state plus
    /// the phrase-entry stack checkpoint the resumed rollback restores. Any
    /// `transactional_submit` or `reset` discards the suspension.
    suspended: Option<Suspended>,
    /// Is the session buffer itself a complete module (a `module`-form
    /// submission)? Fragment submissions accumulate into a synthesized
    /// `module repl-session;` wrapper; a module-form submission *replaces*
    /// the buffer verbatim (DEC-3's replace rule, applied to whole-module
    /// edits) and later content is spliced before its final `end;`.
    module_form: bool,
    /// The last submission's phrase trace (the `trace_events` ABI snapshot's
    /// source — the LAST submission only, DEC-20).
    last_trace: Option<Trace>,
    /// The last submission's status (the `last_status` ABI snapshot's source;
    /// the status bar re-renders without re-submitting).
    last_status: Option<Status>,
    /// The pooled phrase journal (D-W4): capacity persists across phrases,
    /// contents are cleared — the container comes from the session, never a
    /// fresh allocation per phrase (journal docs are now true).
    scratch_journal: Journal,
    /// The pooled diagnostic-output buffer (D-W4): same pooling discipline as
    /// the journal.
    out_pool: Vec<u8>,
    /// The last submit's anchor map (I-9, as-built): the engine's spans
    /// index the COMPILED module text (`Compiled.src`), not the submitted
    /// buffer — the session built that text, so the session is the one place
    /// that can map anchor offsets back onto the doc (`doc_span`). Fields
    /// describe the module text of the most recent submit attempt.
    anchor: AnchorMap,
}

/// The anchor→doc mapping of one submit attempt (see `ReplSession::anchor`).
#[derive(Clone, Debug, Default)]
struct AnchorMap {
    /// The submitted buffer (the I-9 doc).
    doc: Vec<u8>,
    /// Anchor offset of doc[0] (the `module repl-session;\n` prefix width
    /// for fragment sessions, 0 for module-form without an insertion).
    base: usize,
    /// The synthetic phrase word's placement, when a phrase was spliced in:
    /// (anchor offset of the synthetic, anchor offset of the phrase INSIDE
    /// the synthetic, doc offset of the phrase, phrase byte length).
    synthetic: Option<(usize, usize, usize, usize)>,
    /// The synthetic's total anchor width (0 when none) — offsets past it
    /// shift left by this much.
    synthetic_len: usize,
}

impl AnchorMap {
    /// Map an anchor-relative span onto the submitted buffer. `None` for
    /// offsets that live in session machinery (the prefix, the synthetic
    /// word's own tokens) — never a guess (I-8: no JS-side reconstruction).
    fn doc_span(&self, span: Span) -> Option<Span> {
        let map_off = |off: usize| -> Option<usize> {
            if off < self.base {
                return None; // the module prefix
            }
            let d = off - self.base;
            if d < self.doc.len() {
                return Some(d);
            }
            if let Some((at, phrase_at, phrase_doc, phrase_len)) = self.synthetic {
                if off < at {
                    return None;
                }
                if off >= phrase_at && off <= phrase_at + phrase_len {
                    return Some(phrase_doc + (off - phrase_at));
                }
                if off >= at + self.synthetic_len {
                    return Some(off - self.synthetic_len);
                }
                return None; // the synthetic word's own name/sig tokens
            }
            // Past the doc with no synthetic: the trailing `end;`/insertions
            // region — machinery, unless it is exactly the doc end.
            if d == self.doc.len() {
                return Some(d);
            }
            None
        };
        let start = map_off(span.start)?;
        let end = map_off(span.end)?;
        Some(Span::new(start, end.max(start)))
    }
}

/// A fuel-suspended phrase (the plan's `SubmitResult.snapshot`, held
/// session-side so the wasm ABI's `repl_step` resumes it without shipping
/// engine internals across the boundary).
struct Suspended {
    /// The engine state at exhaustion (journal intact; `cursor` at the op
    /// that did not run).
    state: ExecState,
    /// The phrase-entry data stack (the resumed rollback's checkpoint).
    entry_stack: Vec<Slot>,
}

/// The phrase-entry checkpoint (§7.3): the stack before the phrase (the
/// rollback restores it exactly) and the fuel at entry.
pub(crate) struct Checkpoint {
    pub(crate) stack: Vec<Slot>,
    pub(crate) fuel: Fuel,
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
            periph: Box::new(Disabled),
            descriptor: None,
            suspended: None,
            module_form: false,
            last_trace: None,
            last_status: None,
            scratch_journal: Journal::new(),
            out_pool: Vec::new(),
            anchor: AnchorMap::default(),
        }
    }

    /// Attach the device boundary (P4): a scripted/board device, or
    /// [`Disabled`] to keep every MMIO access an E9152.
    pub fn set_peripheral(&mut self, periph: Box<dyn Peripheral>) {
        self.periph = periph;
    }

    /// Attach (or clear) the compiled platform descriptor (P4): without it
    /// MMIO modules are compile-rejected E3640; with it, the aperture table
    /// resolves MMIO accesses and the descriptor is passed to the compiler
    /// exactly as `langc --platform=...` does natively.
    pub fn set_descriptor(&mut self, descriptor: Option<Box<CompiledDescriptor>>) {
        self.descriptor = descriptor;
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

    /// The last-good `--emit=ir` render (DEC-13): the IR pane's source.
    pub fn last_ir_text(&self) -> Option<Vec<u8>> {
        self.compiled.as_ref().map(|c| {
            let mut r = Vec::new();
            crate::render_ir(&mut VecSink(&mut r), &c.module);
            r
        })
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
        self.suspended = None;
        self.module_form = false;
        self.last_trace = None;
        self.last_status = None;
    }

    /// The last submission's phrase trace (the `trace_events` snapshot's
    /// source; `None` when the last submission had no phrase — DEC-20).
    pub fn last_trace(&self) -> Option<&Trace> {
        self.last_trace.as_ref()
    }

    /// The last submission's status (the `last_status` snapshot's source).
    pub fn last_status(&self) -> Option<&Status> {
        self.last_status.as_ref()
    }

    /// The persistent region heap (the `regions` snapshot's source).
    pub fn heap(&self) -> &RegionHeap {
        &self.heap
    }

    /// Map an anchor-relative span (engine spans index the compiled module
    /// text) onto the last submitted buffer (I-9). `None` for machinery
    /// offsets — never a guess.
    pub fn doc_span(&self, span: Span) -> Option<Span> {
        self.anchor.doc_span(span)
    }

    /// The live engine state of a fuel-suspended phrase, if any (the
    /// `frames_snapshot` ABI's live-frames source — post-`step` or
    /// post-suspension; a completed or rolled-back phrase has no frames, §Q8).
    pub fn suspended_state(&self) -> Option<&ExecState> {
        self.suspended.as_ref().map(|s| &s.state)
    }

    /// Execute a submission per §7.2/§7.3 (DEC-3..DEC-6).
    pub fn transactional_submit(
        &mut self,
        submission: &[u8],
        search: &[&dyn SourceTable],
    ) -> SubmitResult {
        let split = split_submission(submission);
        if !split.has_content {
            // Comment-only / empty submission: no-op re-render (DEC-3). The
            // suspension (if any) survives: the session state did not change.
            return self.result_for(Status::Ok, None, 0..0, None);
        }
        // Any real submission invalidates a fuel suspension (§Q8): the
        // suspended phrase ran against the previous compiled artifact.
        self.suspended = None;

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

        // The buffer shape AFTER this submit commits. A module-form
        // submission REPLACES the buffer (DEC-3 replace rule, applied to
        // whole-module edits); fragment definitions accumulate. Nothing is
        // committed until the compile succeeds (transaction #1 is
        // all-or-nothing).
        let committed_module_form = split.module_form || self.module_form;
        let committed_source: Vec<u8> = if split.module_form {
            split.decls.clone().unwrap_or_default()
        } else {
            match &split.decls {
                Some(d) => {
                    if self.module_form {
                        splice_before_final_end(&self.source, d)
                    } else {
                        let mut t = self.source.clone();
                        t.extend_from_slice(d);
                        t
                    }
                }
                None => self.source.clone(),
            }
        };

        // E9157 (§5/§7.5): the session word cap. The session = one module =
        // the IR format's 64-word cap, enforced natively by `build_ir` (E3920);
        // the session boundary rejects the offending definition BEFORE the
        // compile, at the definition that would exceed the cap — the REPL
        // renumbers nothing (native E3920 remains the backstop for
        // lowering-synthesized extra words).
        let current_words = if split.module_form {
            0
        } else {
            self.compiled
                .as_ref()
                .map(|c| c.module.words.len())
                .unwrap_or(0)
        };
        if !split.word_def_spans.is_empty()
            && current_words + split.word_def_spans.len() > MODULE_WORD_CAP
        {
            let over = current_words + split.word_def_spans.len() - MODULE_WORD_CAP;
            let span = split
                .word_def_spans
                .get(split.word_def_spans.len() - over)
                .copied()
                .unwrap_or(Span::UNKNOWN);
            return self.result_for(
                Status::CompileError {
                    diag: crate::Diag::from_bytes(
                        code::E9157_WORD_CAP,
                        span,
                        b"session word cap reached (64 words per module)",
                    ),
                },
                None,
                0..0,
                None,
            );
        }

        let entry: &[u8] = b"repl-phrase";
        // The compiled platform descriptor (P4): when present, MMIO modules
        // compile (E3640 parity otherwise) and the aperture table resolves
        // MMIO accesses.
        let desc = self.descriptor.as_deref();

        // The phrase's exact signature must be declared (the checker
        // verifies, never infers): compute it with the §7.2 effect pass. When
        // a phrase names a word defined in THIS submission, the lookup needs
        // the decls compiled first — compile the decls-only module (phase A),
        // then the full module (phase B). A decls-only failure is a plain
        // CompileError (definitions are all-or-nothing, DEC-3).
        let mut phrase_sig: Option<phrase::EffectSig> = None;
        if let Some(phrase) = split.phrase.as_deref() {
            if !phrase.is_empty() {
                let decls_src = assemble_module(&committed_source, committed_module_form, None);
                let lookup_env: Option<Compiled> = if split.decls.is_some() || split.module_form {
                    // Phase A: validate + build the lookup dictionary.
                    let built = match compile(decls_src.as_slice(), search, self.opts.spec, desc) {
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
                // The ambient inputs are the ACTUAL persistent stack (the W1
                // reconciliation — see `phrase_effect`): the synthetic phrase
                // word's declared inputs are what the runtime stack really
                // holds, so the checker verifies the phrase against reality.
                let ambient: Vec<Vec<u8>> = self
                    .stack
                    .iter()
                    .rev()
                    .map(|s| self.type_name(s.ty))
                    .collect();
                phrase_sig = phrase::phrase_effect(phrase, &ambient, |name| match &lookup_env {
                    Some(c) => lookup_word_in(c, name).or_else(|| self.lookup_word(name)),
                    None => self.lookup_word(name),
                });
            }
        }

        // Phase B: the full module (decls + the synthetic `repl-phrase` word
        // when a phrase is present).
        let synthetic = split.phrase.as_deref().filter(|p| !p.is_empty()).map(|p| {
            let mut t = alloc::string::String::from(": repl-phrase ");
            if let Some(sig) = &phrase_sig {
                t.push_str(&sig_text(sig));
            }
            t.push(' ');
            t.push_str(core::str::from_utf8(p).unwrap_or(""));
            t.push_str(" ;\n");
            t
        });
        let module_text = assemble_module(
            &committed_source,
            committed_module_form,
            synthetic.as_deref(),
        );
        // The anchor map for THIS attempt (the engine's spans will index
        // `module_text`; `doc_span` maps them back onto the submitted
        // buffer). Phrase geometry: the synthetic embeds the phrase bytes
        // verbatim before its trailing `" ;\n"`.
        self.anchor = {
            let doc = submission.to_vec();
            let mut map = AnchorMap {
                doc: doc.clone(),
                base: if committed_module_form { 0 } else { "module repl-session;
".len() },
                synthetic: None,
                synthetic_len: 0,
            };
            if let (Some(syn), Some(phrase)) = (synthetic.as_deref(), split.phrase.as_deref()) {
                let phrase_len = phrase.len();
                let phrase_in_doc = if committed_module_form {
                    // Module form: the buffer's phrase region is everything
                    // after the module's final `end;`.
                    doc.len().saturating_sub(phrase_len)
                } else {
                    committed_source.len()
                };
                let synthetic_at = if committed_module_form {
                    // Spliced before the final top-level `end;` — its exact
                    // offset is the doc's `end` token position, which the
                    // assembler found; recompute the same way it did.
                    splice_offset(&doc).unwrap_or(0)
                } else {
                    map.base + committed_source.len()
                };
                map.synthetic_len = syn.len();
                map.synthetic = Some((
                    synthetic_at,
                    synthetic_at + syn.len() - phrase_len - " ;
".len(),
                    phrase_in_doc,
                    phrase_len,
                ));
            }
            map
        };

        let built = match compile(&module_text, search, self.opts.spec, desc) {
            Ok(b) => b,
            Err(diag) => {
                // Session untouched (DEC-3: compile errors are not edits).
                return self.result_for(Status::CompileError { diag }, None, 0..0, None);
            }
        };

        // Commit the definitions (transaction #1 success). The trace header
        // pins the SHA-256 of the exact buffer this submit produced (FR-35).
        let committed_hash = crate::sysroot::sha256(&committed_source);
        self.source = committed_source;
        self.module_form = committed_module_form;

        let mut rendered = Vec::new();
        crate::render_ir(&mut VecSink(&mut rendered), &built.module);
        let ir_now = Some(rendered);
        self.compiled = Some(built.clone());

        if split.phrase.is_none() || split.phrase.as_deref().is_some_and(|p| p.is_empty()) {
            // Definitions only: no run (DEC-3); stack unchanged. The trace
            // snapshot covers the LAST submission only (DEC-20) — this
            // submission had no phrase.
            self.last_trace = None;
            return self.result_for(Status::Ok, ir_now, 0..0, None);
        }

        // -- transaction #2: the phrase (rollback via the undo journal) ------
        let out_start = self.out_total.len() as u32;
        let mut state = ExecState::in_session(
            core::mem::take(&mut self.stack),
            core::mem::take(&mut self.heap),
            self.fuel,
            self.opts.elide_guards,
            core::mem::take(&mut self.scratch_journal),
            core::mem::take(&mut self.out_pool),
        );
        state.intern_compiled(&built);
        let snap = Checkpoint {
            stack: state.stack.clone(),
            fuel: state.fuel,
        };
        let entry_fuel = state.fuel.remaining();
        let outcome = Engine::new(&built).run(entry, &mut state, &mut *self.periph);

        // The phrase trace is derived from the journal + outcome (§7.4) — a
        // walk over the journal, never a clone (D-W4). The header pins the
        // FR-35 facts: schema version, target, exact submitted bytes, budget.
        let header = TraceHeader {
            v: crate::trace::TRACE_VERSION,
            spec: alloc::string::String::from(self.opts.spec.triple),
            source_hash: hex_32(&committed_hash),
            fuel: entry_fuel,
        };
        let trace = Trace::from_phrase(header, &state.journal, &outcome);
        self.last_trace = Some(trace.clone());

        let status = self.settle(outcome, &mut state, &snap, out_start);
        let out_end = self.out_total.len() as u32;
        self.reclaim_phrase_buffers(&mut state);

        self.result_for(status, ir_now, out_start..out_end, Some(trace))
    }

    /// Resume the fuel-suspended phrase with `ops` more ops of budget (§Q6:
    /// "the phrase can be resumed with more fuel — this is also the stepping
    /// primitive", §Q8). `ops == 0` means unlimited on host/CI (the wasm
    /// boundary rejects it with E9151). With no suspension: E9155.
    pub fn step(&mut self, ops: u64) -> SubmitResult {
        let Some(susp) = self.suspended.take() else {
            return self.result_for(
                Status::CompileError {
                    diag: crate::Diag::from_bytes(
                        code::E9155_PHRASE,
                        Span::UNKNOWN,
                        b"no suspended phrase to step (step resumes a fuel-exhausted phrase)",
                    ),
                },
                None,
                0..0,
                None,
            );
        };
        let Some(built) = self.compiled.clone() else {
            return self.result_for(
                Status::Internal {
                    diag: crate::Diag::from_bytes(
                        code::E9158_INTERNAL,
                        Span::UNKNOWN,
                        b"suspension without a compiled module",
                    ),
                },
                None,
                0..0,
                None,
            );
        };

        let out_start = self.out_total.len() as u32;
        let mut state = susp.state;
        state.fuel = Fuel::new(if ops == 0 { u64::MAX } else { ops });
        let snap = Checkpoint {
            stack: susp.entry_stack,
            fuel: state.fuel,
        };
        let entry_fuel = state.fuel.remaining();
        let outcome = Engine::new(&built).resume(&mut state, &mut *self.periph);

        // The resumed run's journal continues the suspended phrase's journal,
        // so the trace covers the WHOLE phrase (pre- and post-suspension).
        // The header's `fuel` is the resumed budget (the trace covers the
        // whole phrase; the budget shown is the step's).
        let header = TraceHeader {
            v: crate::trace::TRACE_VERSION,
            spec: alloc::string::String::from(self.opts.spec.triple),
            source_hash: hex_32(&crate::sysroot::sha256(&self.source)),
            fuel: entry_fuel,
        };
        let trace = Trace::from_phrase(header, &state.journal, &outcome);
        self.last_trace = Some(trace.clone());

        let status = self.settle(outcome, &mut state, &snap, out_start);
        let out_end = self.out_total.len() as u32;
        self.reclaim_phrase_buffers(&mut state);

        self.result_for(status, None, out_start..out_end, Some(trace))
    }

    /// Return the phrase-run buffers to the session pool (D-W4): the journal
    /// and the diagnostic-output `Vec` keep their capacity across phrases;
    /// contents are cleared.
    fn reclaim_phrase_buffers(&mut self, state: &mut ExecState) {
        state.journal.clear();
        self.scratch_journal = core::mem::take(&mut state.journal);
        self.out_pool = core::mem::take(&mut state.out);
    }

    /// Commit (or roll back and suspend) a phrase outcome — shared by
    /// `transactional_submit` and `step` (§7.3). `out_start` is where this
    /// segment's output begins in `out_total` (the DEC-6 retention rule:
    /// a faulted phrase's already-emitted output stays).
    fn settle(
        &mut self,
        outcome: Outcome,
        state: &mut ExecState,
        snap: &Checkpoint,
        out_start: u32,
    ) -> Status {
        let (status, mut out) = match outcome {
            Outcome::Ok { out } => {
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
            Outcome::Trap {
                code,
                span,
                detail,
                out,
            } => {
                rollback(state, snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (Status::Trapped { code, span, detail }, out)
            }
            Outcome::Fuel { span, out } => {
                // Suspend at the exhaustion point FIRST (the at-fault state,
                // journal intact — the step primitive's resume point), then
                // roll the session surface back to the phrase entry (§7.3).
                self.suspended = Some(Suspended {
                    state: state.clone(),
                    entry_stack: snap.stack.clone(),
                });
                rollback(state, snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (
                    Status::FuelExhausted {
                        diag: crate::Diag::from_bytes(
                            code::E9150_FUEL,
                            span,
                            b"phrase fuel exhausted (the phrase is suspended; step to resume)",
                        ),
                    },
                    out,
                )
            }
            Outcome::Device {
                code,
                span,
                aperture,
                out,
            } => {
                // A device-level failure (P4, e.g. E9152 unbound aperture):
                // the same transactional treatment as a trap — state rolls
                // back, output before the fault stays (DEC-6).
                rollback(state, snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (
                    Status::DeviceError {
                        code,
                        span,
                        aperture,
                    },
                    out,
                )
            }
            Outcome::Internal { fault, out } => {
                rollback(state, snap);
                self.stack = core::mem::take(&mut state.stack);
                self.heap = core::mem::take(&mut state.heap);
                (
                    Status::Internal {
                        diag: crate::Diag::from_bytes(fault.code, fault.span, fault.message),
                    },
                    out,
                )
            }
        };
        self.out_total.extend_from_slice(&out);
        // Return the output buffer's capacity to the pool (D-W4).
        out.clear();
        self.out_pool = out;
        // DEC-6: a faulted phrase's output is retained even as stack/heap
        // roll back — BUT never under the G56 selftest corruption.
        if matches!(
            status,
            Status::Trapped { .. } | Status::DeviceError { .. } | Status::Internal { .. }
        ) && !selftest_retain_trap_output()
        {
            self.out_total.truncate(out_start as usize);
        }
        status
    }

    /// Package a status into a `SubmitResult` (the last-good IR render is
    /// attached on demand — DEC-13). The status is retained as the
    /// `last_status` snapshot (the status bar re-renders without re-submit).
    fn result_for(
        &mut self,
        status: Status,
        ir_now: Option<Vec<u8>>,
        output_new: Range<u32>,
        trace: Option<Trace>,
    ) -> SubmitResult {
        self.last_status = Some(status.clone());
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

impl Status {
    /// The status bar's outcome word (§7.4): one canonical rendering shared
    /// by the transcript (G56), the debug dump (G57) and the wasm ABI — one
    /// renderer per fact (I-11).
    pub fn word(&self) -> alloc::string::String {
        match self {
            Status::Ok => "ok".into(),
            Status::CompileError { diag } => alloc::format!("compile-error E{}", diag.code),
            Status::Trapped { code, .. } => {
                alloc::format!("trapped {}", ir::trap_code_u32(*code))
            }
            Status::FuelExhausted { .. } => "fuel E9150".into(),
            Status::Internal { diag } => alloc::format!("internal E{}", diag.code),
            Status::Exited { code } => alloc::format!("exited {code}"),
            Status::DeviceError { code, aperture, .. } => match aperture {
                Some(ap) => alloc::format!("device-error E{code} ap{ap}"),
                None => alloc::format!("device-error E{code}"),
            },
        }
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
    /// lookup against the decls-only compile). `None` when the module does not
    /// define the name — the caller falls through to the builtin tables; no
    /// pre-scan double lookup, no `expect`, no silent default (D-W6).
    fn lookup_word_in(c: &Compiled, name: &[u8]) -> Option<EffectSig> {
        let w = c
            .module
            .words
            .iter()
            .find(|w| w.name.as_bytes() == name)?;
        let sig = &w.sig;
        let ty_name = |id: ir::TypeId| -> Vec<u8> {
            w.types
                .get(id.0 as usize)
                .map(|a| a.as_bytes().to_vec())
                .unwrap_or_else(|| b"?".to_vec())
        };
        let inputs = (0..sig.in_len as usize)
            .map(|i| ty_name(sig.inputs[i]))
            .collect();
        let outputs = (0..sig.out_len as usize)
            .map(|i| ty_name(sig.outputs[i]))
            .collect();
        Some(EffectSig { inputs, outputs })
    }

/// Render an effect signature as canonical `( in -- out )` text — the exact
/// form `parse_word_sig` accepts, including the empty `( -- )` shape.
fn sig_text(sig: &EffectSig) -> alloc::string::String {
    let names = |ts: &[Vec<u8>]| -> alloc::string::String {
        ts.iter()
            .map(|t| core::str::from_utf8(t).unwrap_or("?"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    alloc::format!("( {} -- {} )", names(&sig.inputs), names(&sig.outputs))
}

/// A SHA-256 digest as 64 lowercase hex bytes (the trace header's
/// `source_hash`, FR-35).
fn hex_32(d: &[u8; 32]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(64);
    for b in d {
        s.push_str(&alloc::format!("{b:02x}"));
    }
    s
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

pub(crate) fn rollback(state: &mut ExecState, snap: &Checkpoint) {
    for e in state.journal.iter().rev() {
        match e {
            Effect::Store { addr, width, old } => state.heap.compensate_store(*addr, *width, *old),
            Effect::RegionCreate { handle, old_cursor } => {
                state.heap.compensate_region_create(*handle, *old_cursor)
            }
            Effect::Intern { addr, old_cursor } => state.heap.compensate_intern(*addr, *old_cursor),
            Effect::RegionAlloc { handle, old_off } | Effect::RegionReset { handle, old_off } => {
                state.heap.compensate_cursor(*handle, *old_off);
            }
            Effect::RegionDestroy { handle, saved } => {
                state.heap.compensate_region_destroy(*handle, *saved);
            }
            Effect::ScopedEnter { addr, old_cursor } => {
                state.heap.compensate_scoped(*addr, *old_cursor)
            }
            // Trace-only effects are never undone (§Q3 DEC-6).
            Effect::Output { .. } | Effect::Call { .. } | Effect::Periph { .. } => {}
        }
    }
    state.journal.clear();
    state.frames.clear();
    // The per-run resource-place cache is phrase-scoped: a rollback reclaims
    // the interned reservations, so cached addresses would be stale.
    state.resources.clear();
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
    /// The submission is a complete `module ... end;` (its declarations
    /// region *is* the module text).
    module_form: bool,
    /// Bytes to append to (or, module-form, to become) the session source.
    decls: Option<Vec<u8>>,
    /// The phrase bytes (None = no phrase).
    phrase: Option<Vec<u8>>,
    /// The spans of the top-level word-definition openers (`:`), in order —
    /// the E9157 word-cap pre-check counts definitions with them.
    word_def_spans: Vec<Span>,
}

/// The module-level statement openers and their terminators, per the
/// production parser's dispatch (`frontend::parse`): `;`-terminated forms
/// and `end;`-terminated forms. Word definitions (`:` ... `;`) are tracked
/// separately because the phrase is defined *against* them (§7.2).
fn is_decl_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::KwImport
            | TokenKind::KwExport
            | TokenKind::KwType
            | TokenKind::KwSubtype
            | TokenKind::KwStruct
            | TokenKind::KwEnum
            | TokenKind::KwConst
            | TokenKind::KwResource
            | TokenKind::KwRegisterMap
            | TokenKind::KwOwned
            | TokenKind::KwIso
    )
}

fn decl_is_end_terminated(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::KwStruct | TokenKind::KwEnum | TokenKind::KwRegisterMap
    )
}

fn depth_delta(k: TokenKind) -> i32 {
    match k {
        TokenKind::PunctLParen | TokenKind::PunctLBracket | TokenKind::PunctLBrace => 1,
        TokenKind::PunctRParen | TokenKind::PunctRBracket | TokenKind::PunctRBrace => -1,
        _ => 0,
    }
}

/// Lex the whole submission (the lexer skips comments and whitespace; strings
/// are single tokens, so punctuation inside them cannot skew depth).
fn lex_all(src: &[u8]) -> Vec<Token> {
    let mut lex = Lexer::new(src);
    let mut toks = Vec::new();
    loop {
        let t = lex.next();
        if t.kind == TokenKind::Eof {
            return toks;
        }
        toks.push(t);
    }
}

/// Byte offset of the token that starts the top-level run at `i` (skipping
/// leading whitespace/comments is the lexer's job, so this is exact).
fn run_start(src: &[u8], toks: &[Token], i: usize) -> usize {
    if i < toks.len() {
        toks[i].span.start
    } else {
        src.len()
    }
}

fn split_submission(src: &[u8]) -> Split {
    let toks = lex_all(src);
    if toks.is_empty() {
        return Split {
            has_content: false,
            module_form: false,
            decls: None,
            phrase: None,
            word_def_spans: Vec::new(),
        };
    }

    // -- module form: `module ... end;` (optionally + trailing phrase) -------
    if toks[0].kind == TokenKind::KwModule {
        // The module's closing `end;` is the LAST top-level `end;` (every
        // inner form's own `end;` is consumed as part of that form).
        let mut depth = 0i32;
        let mut last_end_semi = None; // token index of the `;` after the last top-level `end;`
        let mut i = 0usize;
        while i < toks.len() {
            let k = toks[i].kind;
            if depth == 0 && k == TokenKind::KwEnd {
                if let Some(semi) = toks.get(i + 1) {
                    if semi.kind == TokenKind::PunctSemi {
                        last_end_semi = Some(i + 1);
                    }
                }
            }
            depth += depth_delta(k);
            i += 1;
        }
        let (decls_end, phrase) = match last_end_semi {
            Some(si) => {
                let phrase_start = run_start(src, &toks, si + 1);
                (toks[si].span.end, trim_bytes(&src[phrase_start..]))
            }
            None => (src.len(), &[][..]),
        };
        let word_def_spans = toks
            .iter()
            .filter(|t| t.kind == TokenKind::PunctColon)
            .map(|t| t.span)
            .collect();
        return Split {
            has_content: true,
            module_form: true,
            decls: Some(src[..decls_end].to_vec()),
            phrase: if phrase.is_empty() {
                None
            } else {
                Some(phrase.to_vec())
            },
            word_def_spans,
        };
    }

    // -- fragment form: top-level declarations, then at most one phrase ------
    let mut depth = 0i32;
    let mut word_def_spans: Vec<Span> = Vec::new();
    let mut i = 0usize;
    let mut phrase_start: Option<usize> = None;
    while i < toks.len() {
        let k = toks[i].kind;
        if depth == 0 {
            match k {
                TokenKind::PunctColon => {
                    // A word definition runs to the next top-level `;`.
                    word_def_spans.push(toks[i].span);
                    i += 1;
                    while i < toks.len() {
                        depth += depth_delta(toks[i].kind);
                        if depth == 0 && toks[i].kind == TokenKind::PunctSemi {
                            break;
                        }
                        i += 1;
                    }
                }
                k if is_decl_keyword(k) => {
                    // A declaration statement: `;`-terminated, or `end;`-
                    // terminated for the block forms (struct/enum/register-map).
                    let end_term = decl_is_end_terminated(k);
                    i += 1;
                    while i < toks.len() {
                        depth += depth_delta(toks[i].kind);
                        let hit = if end_term {
                            depth == 0
                                && toks[i].kind == TokenKind::KwEnd
                                && toks.get(i + 1).map(|t| t.kind) == Some(TokenKind::PunctSemi)
                        } else {
                            depth == 0 && toks[i].kind == TokenKind::PunctSemi
                        };
                        if hit {
                            if end_term {
                                i += 1; // consume the `end` so the `;` below ends the statement
                            }
                            break;
                        }
                        i += 1;
                    }
                }
                TokenKind::Ident if src[toks[i].span.start] == b'@' => {
                    // An attribute (`@interrupt(3)`): production-parse surface
                    // that attaches to the next definition — never a phrase.
                }
                _ => {
                    phrase_start = Some(i);
                    break;
                }
            }
        }
        depth += depth_delta(k);
        i += 1;
    }

    match phrase_start {
        Some(pi) => {
            let ps = run_start(src, &toks, pi);
            let phrase = trim_bytes(&src[ps..]);
            Split {
                has_content: true,
                module_form: false,
                decls: Some(src[..ps].to_vec()),
                phrase: if phrase.is_empty() {
                    None
                } else {
                    Some(phrase.to_vec())
                },
                word_def_spans,
            }
        }
        None => Split {
            has_content: true,
            module_form: false,
            decls: Some(src.to_vec()),
            phrase: None,
            word_def_spans,
        },
    }
}

/// Assemble the compile unit for a session buffer: a module-form buffer
/// compiles as itself (insertion spliced before its final `end;`); a
/// fragment buffer compiles inside the synthesized `module repl-session;`
/// wrapper (§7.2 — the accommodation lives in the session builder, never the
/// production parser).
fn assemble_module(source: &[u8], module_form: bool, insertion: Option<&str>) -> Vec<u8> {
    if module_form {
        match insertion.filter(|s| !s.is_empty()) {
            Some(ins) => splice_before_final_end_str(source, ins),
            None => source.to_vec(),
        }
    } else {
        let mut t = alloc::string::String::with_capacity(source.len() + 64);
        t.push_str("module repl-session;\n");
        t.push_str(core::str::from_utf8(source).unwrap_or(""));
        if let Some(ins) = insertion {
            t.push_str(ins);
        }
        t.push_str("end;\n");
        t.into_bytes()
    }
}

/// The byte offset of a module buffer's final top-level `end;` token (the
/// assembler's splice point; recomputed for the anchor map).
fn splice_offset(source: &[u8]) -> Option<usize> {
    let toks = lex_all(source);
    let mut depth = 0i32;
    let mut at = None;
    for t in &toks {
        if depth == 0
            && t.kind == TokenKind::KwEnd
            && toks.get(
                toks.iter().position(|x| x == t).unwrap_or(usize::MAX) + 1,
            )
            .map(|n| n.kind)
            == Some(TokenKind::PunctSemi)
        {
            at = Some(t.span.start);
        }
        depth += depth_delta(t.kind);
    }
    at
}

/// Splice `insertion` (bytes) before a module buffer's final top-level
/// `end;` — the session's insertion point for new content inside a
/// module-form buffer. A buffer without a final top-level `end;` is
/// malformed and compiles as-is (the parser reports it).
fn splice_before_final_end(source: &[u8], insertion: &[u8]) -> Vec<u8> {
    splice_before_final_end_str(source, core::str::from_utf8(insertion).unwrap_or(""))
}

fn splice_before_final_end_str(source: &[u8], insertion: &str) -> Vec<u8> {
    let toks = lex_all(source);
    let mut depth = 0i32;
    let mut splice_at: Option<usize> = None;
    for (i, t) in toks.iter().enumerate() {
        if depth == 0
            && t.kind == TokenKind::KwEnd
            && toks.get(i + 1).map(|n| n.kind) == Some(TokenKind::PunctSemi)
        {
            splice_at = Some(t.span.start);
        }
        depth += depth_delta(t.kind);
    }
    match splice_at {
        Some(at) => {
            let mut out = Vec::with_capacity(source.len() + insertion.len());
            out.extend_from_slice(&source[..at]);
            out.extend_from_slice(insertion.as_bytes());
            out.extend_from_slice(&source[at..]);
            out
        }
        None => source.to_vec(),
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    // `is_ascii_whitespace` already covers `\n` (the redundant arm is gone,
    // D-W6).
    let mut s = 0usize;
    let mut e = b.len();
    while s < e && b[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    &b[s..e]
}

/// The IR-render output sink (D-W7): one implementation in the lib — the
/// session, the ABI, and `repl-cli` all render through this type.
pub struct VecSink<'a>(pub &'a mut Vec<u8>);
impl frontend::parse::Output for VecSink<'_> {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}
