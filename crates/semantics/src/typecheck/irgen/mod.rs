use crate::typecheck::context::{ContextKind, ContextStack, FrameParam};
use crate::typecheck::db::{
    enum_variant_value, is_iso_type, resource_is_isr_reachable, resource_ty, struct_field_ty,
    IsoDb, NominalDb, ResourceDb, SubtypeInfo,
};
use crate::typecheck::error::{ChecksMode, EscapeKind, TcError};
use crate::typecheck::irgen::compile::borrow::{mint_id, PlaceKey, LEDGER_CAP};
use crate::typecheck::mmio::mmio_type_width_bytes;
use crate::typecheck::mmio::{aperture_access_bits,
    access_can_read, access_can_write, field_mask_shift, resolve_mmio_place, MmioDb, MmioResolved,
};
use crate::typecheck::parse::{capture_balanced, capture_scoped_block, read_qualified_name};
use crate::typecheck::place::PlacePath;
use crate::typecheck::util::parse_u32_any;
use crate::typecheck::util::{
    align_up, apply_sig, array_elem_type, array_len, chan_elem_type, check_no_scoped_live,
    field_align, find_local, find_subtype, lookup, parse_i64_token, pop, push, region_ref_type,
    slice_span, slice_type_of_elem, type_compatible, type_size_bytes,
};
use crate::typecheck::value::{PlaceId, Value, PARAM_BASE, PLACE_NONE};
use crate::types::{TypeAtom, WordEntry, WordSig};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use core::mem::MaybeUninit;
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::parse::{AttrAst, DeclAst};
use frontend::span::Span;
use frontend::token::{Token, TokenKind};
use ir::{self as lir, CapSet, EffectSet, High, StackBound};
use verifier::model::{ExtractionCtx, Formula, Kind as OblKind, Oel, Provenance};

pub mod arena;
mod compile;
mod control;
mod locals;
mod observer;
mod prologue;
mod quotes;
mod types;

pub use observer::{NullObserver, StackcheckObserver, TypecheckObserver};
pub use types::intern_type;

#[allow(dead_code)]
struct QuoteSig {
    sig: WordSig,
    performs: EffectSet,
    requires: CapSet,
    bound: StackBound,
    body: Span,
    has_explicit_performs: bool,
}

struct DestructBind {
    name: TypeAtom,
    borrow: bool,
    mutable: bool,
    span: Span,
}

enum IndexOut {
    Value,
    Ptr(bool),
    Mmio,
}

struct IrWordGen<'a, 'r> {
    src: &'a [u8],
    env: &'a [WordEntry],
    subtypes: &'a [SubtypeInfo],
    mmio: &'a MmioDb,
    /// The compiled platform descriptor (P4): sources aperture identity/size for
    /// the word's aperture-use table. `None` for descriptor-less compiles.
    descriptor: Option<&'a codegen_core::compiled_desc::CompiledDescriptor>,
    resources: &'a ResourceDb,
    nominals: &'a NominalDb,
    iso: &'a IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    /// Verification-obligation extraction (slice P2): `Some` only for the
    /// final lowering of declared words — the fixpoint passes and quotation
    /// words compile with `None`. Quotation internal names (`_quot_*`) are
    /// per-word-local, so obligation ids keyed on them would collide across
    /// words; their sites are covered in P5 under the caller word.
    extraction: Option<&'a mut ExtractionCtx>,
    sig: WordSig,

    locals: [TypeAtom; 64],
    local_tys: [TypeAtom; 64],
    local_live: [bool; 64],
    local_scoped: [u16; 64],
    local_place: [PlaceId; 64],
    local_len: usize,

    next_scope: u16,
    scope_stack: [u16; 16],
    scope_sp: usize,

    ctx: ContextStack,

    /// Borrow ledger: tracks live borrowed places for exclusivity checking (S-3).
    ledger: [PlaceKey; LEDGER_CAP],
    ledger_len: u8,

    /// Accumulated stack-bound for the word being compiled (stack-bound §2).
    acc: StackBound,

    terminated: bool,
    word: lir::Word,
    /// Aperture-use table accumulated while emitting `MmioPlace`/`AddrOf::Mmio`
    /// ops (P4); assigned to `word.apertures` at finalization.
    word_apertures: FixedVec<lir::ApertureUse, 8>,
    extra_words: FixedVec<&'r lir::Word, { arena::QUOTE_WORD_CAP }>,
    arena: *mut arena::ArenaAllocator,
    quote_id: u32,
}

impl<'a, 'r> IrWordGen<'a, 'r> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        src: &'a [u8],
        env: &'a [WordEntry],
        subtypes: &'a [SubtypeInfo],
        mmio: &'a MmioDb,
        descriptor: Option<&'a codegen_core::compiled_desc::CompiledDescriptor>,
        resources: &'a ResourceDb,
        nominals: &'a NominalDb,
        iso: &'a IsoDb,
        checks: ChecksMode,
        allow_raw_casts: bool,
        extraction: Option<&'a mut ExtractionCtx>,
        arena: &mut arena::ArenaAllocator,
        sig: WordSig,
        name: lir::Atom,
    ) -> Result<Self, TcError> {
        let arena = arena as *mut arena::ArenaAllocator;
        let mut types: FixedVec<lir::Atom, 64> = FixedVec::new();
        let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
        let mut subtype_bases: FixedVec<lir::TypeId, 64> = FixedVec::new();
        let z = lir::AT_EMPTY;
        types.push(z).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        type_sizes.push(0).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_I64)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_BOOL)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(1).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_STR)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_PTR)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_PTR_MUT)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        types
            .push(lir::AT_MMIO)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        subtype_bases
            .push(lir::TY_EMPTY)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;

        let mut lir_sig = lir::Sig::empty();
        lir_sig.in_len = sig.in_len;
        lir_sig.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            let atom = lir_atom(sig.inputs[i].as_bytes())?;
            lir_sig.inputs[i] =
                intern_type(&mut types, &mut type_sizes, atom, nominals, Span::new(0, 0))?;
        }
        for i in 0..(sig.out_len as usize) {
            let atom = lir_atom(sig.outputs[i].as_bytes())?;
            lir_sig.outputs[i] =
                intern_type(&mut types, &mut type_sizes, atom, nominals, Span::new(0, 0))?;
        }

        let mut blocks: FixedVec<lir::Block, 16> = FixedVec::new();
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for i in 0..(lir_sig.in_len as usize) {
            entry_stack
                .push(lir_sig.inputs[i])
                .map_err(|_| TcError::TypeTableFull {
                    span: Span::new(0, 0),
                })?;
        }
        let entry_block = lir::Block {
            id: lir::BlockId(0),
            entry_stack,
            ops: FixedVec::new(),
        };
        blocks
            .push(entry_block)
            .map_err(|_| TcError::BlockTableFull {
                span: Span::new(0, 0),
            })?;

        let extra_words = FixedVec::new();
        let quote_id = 0u32;
        Ok(Self {
            src,
            env,
            subtypes,
            mmio,
            descriptor,
            resources,
            nominals,
            iso,
            checks,
            allow_raw_casts,
            extraction,
            sig,
            locals: [TypeAtom::EMPTY; 64],
            local_tys: [TypeAtom::EMPTY; 64],
            local_live: [false; 64],
            local_scoped: [0u16; 64],
            local_place: [PLACE_NONE; 64],
            local_len: 0,
            next_scope: 1,
            scope_stack: [0u16; 16],
            scope_sp: 0,
            ctx: ContextStack::new(),
            ledger: [PlaceKey {
                root: TypeAtom::EMPTY,
                full: TypeAtom::EMPTY,
                origin: Span::new(0, 0),
            }; LEDGER_CAP],
            ledger_len: 0,
            acc: StackBound::ID,
            terminated: false,
            word: lir::Word {
                name,
                sig: lir_sig,
                performs: EffectSet::empty(),
                requires: CapSet::empty(),
                bound: StackBound::ID,
                entry: lir::BlockId(0),
                types,
                type_sizes,
                type_classes: FixedVec::new(),
                apertures: FixedVec::new(),
                subtype_bases,
                blocks,
            },
            word_apertures: FixedVec::new(),
            extra_words,
            arena,
            quote_id,
        })
    }

    pub(super) fn block_mut(&mut self, id: lir::BlockId) -> Result<&mut lir::Block, TcError> {
        self.word
            .blocks
            .get_mut(id.0 as usize)
            .ok_or(TcError::BlockNotFound {
                span: Span::new(0, 0),
            })
    }

    pub(super) fn new_block(
        &mut self,
        stack: &[Value; 256],
        sp: usize,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let id = lir::BlockId(self.word.blocks.len() as u16);
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for (_, v) in stack.iter().enumerate().take(sp) {
            let tid = self.ty_id_of_value(*v, span)?;
            entry_stack
                .push(tid)
                .map_err(|_| TcError::TypeTableFull { span })?;
        }
        let b = lir::Block {
            id,
            entry_stack,
            ops: FixedVec::new(),
        };
        self.word
            .blocks
            .push(b)
            .map_err(|_| TcError::BlockTableFull { span })?;
        Ok(id)
    }

    pub(super) fn emit_op(
        &mut self,
        cur: lir::BlockId,
        kind: lir::OpKind,
        span: Span,
    ) -> Result<(), TcError> {
        // BUG-012: keep the stack-bound accumulator in lockstep with the
        // runtime data-stack delta of every op, so `branch_max`'s equal-net
        // invariant holds for all well-typed programs.  Ops that need caller
        // context (`Call`, control-flow targets) return `None` and their
        // caller accounts the delta.
        if let Some(delta) = Self::op_stack_delta(&kind) {
            self.acc = self.acc.compose(delta);
        }
        let op = lir::Op { kind, span };
        let b = self.block_mut(cur)?;
        b.ops.push(op).map_err(|_| TcError::OpTableFull { span })?;
        Ok(())
    }

    /// Stack delta of a single `OpKind`, matching the runtime data-stack
    /// effect (BUG-012).  `None` means the op does not change stack depth or
    /// the caller accounts it explicitly (`Call`, `Br`, `Ret`).
    pub(super) fn op_stack_delta(kind: &lir::OpKind) -> Option<StackBound> {
        const ONE: StackBound = StackBound {
            net: 1,
            high: High::Slots(1),
        };
        const ZERO: StackBound = StackBound {
            net: 0,
            high: High::Slots(0),
        };
        const NEG1: StackBound = StackBound {
            net: -1,
            high: High::Slots(0),
        };
        const NEG2: StackBound = StackBound {
            net: -2,
            high: High::Slots(0),
        };
        match kind {
            lir::OpKind::ConstI64(_) | lir::OpKind::ConstBool(_) | lir::OpKind::ConstStr(_) => {
                Some(ONE)
            }
            lir::OpKind::AddrOf { .. } | lir::OpKind::MmioPlace { .. } => Some(ONE),
            lir::OpKind::ScopedEnter { .. } => Some(ONE),
            lir::OpKind::TaskSpawn { .. } => Some(ONE),
            lir::OpKind::PtrAddConst { .. } => Some(ZERO),
            lir::OpKind::PtrAddIndex { .. } => Some(NEG1),
            lir::OpKind::Dup { .. } => Some(ONE),
            lir::OpKind::Drop { .. } => Some(NEG1),
            lir::OpKind::Swap { .. } => Some(ZERO),
            lir::OpKind::AddI64 | lir::OpKind::SubI64 | lir::OpKind::MulI64 => Some(NEG1),
            lir::OpKind::Cmp { .. } | lir::OpKind::AndBool | lir::OpKind::OrBool => Some(NEG1),
            lir::OpKind::NotBool
            | lir::OpKind::InterruptDisable
            | lir::OpKind::InterruptEnable => Some(ZERO),
            lir::OpKind::LocalSet { .. } => Some(NEG1),
            lir::OpKind::LocalGet { .. } => Some(ONE),
            lir::OpKind::Cast { .. } | lir::OpKind::Bitcast { .. } => Some(ZERO),
            lir::OpKind::Load { .. }
            | lir::OpKind::MmioVolLoad { .. }
            | lir::OpKind::MmioVolLoadField { .. } => Some(ZERO),
            lir::OpKind::Store { .. }
            | lir::OpKind::MmioVolStore { .. }
            | lir::OpKind::MmioVolStoreField { .. } => Some(NEG2),
            lir::OpKind::TrapIfFalse { .. } => Some(NEG1),
            // `BrIf` pops the condition, but the control-flow compilers
            // (`if`/`while`/`loop`) snapshot and reset `self.acc` around the
            // branch, so they account it themselves.  `Call` (caller composes
            // the callee's sig/bound), `Br`, `Ret` are handled by their callers.
            lir::OpKind::BrIf { .. } | lir::OpKind::Call { .. } | lir::OpKind::Br { .. } | lir::OpKind::Ret => {
                None
            }
        }
    }

    pub(super) fn emit_subtype_range_trap(
        &mut self,
        cur: lir::BlockId,
        value_slot: u16,
        value_ty: lir::TypeId,
        st: &SubtypeInfo,
        span: Span,
    ) -> Result<(), TcError> {
        self.emit_op(
            cur,
            lir::OpKind::LocalGet {
                slot: value_slot,
                ty: value_ty,
            },
            span,
        )?;
        self.emit_op(cur, lir::OpKind::ConstI64(st.min), span)?;
        self.emit_op(
            cur,
            lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Ge,
            },
            span,
        )?;
        self.emit_op(
            cur,
            lir::OpKind::TrapIfFalse {
                code: lir::TrapCode::SubtypeFail,
            },
            span,
        )?;

        self.emit_op(
            cur,
            lir::OpKind::LocalGet {
                slot: value_slot,
                ty: value_ty,
            },
            span,
        )?;
        self.emit_op(cur, lir::OpKind::ConstI64(st.max), span)?;
        self.emit_op(
            cur,
            lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Le,
            },
            span,
        )?;
        self.emit_op(
            cur,
            lir::OpKind::TrapIfFalse {
                code: lir::TrapCode::SubtypeFail,
            },
            span,
        )?;
        Ok(())
    }

    pub(super) fn resolve_place_pointee_ty(
        &self,
        place: &PlacePath,
        place_abs: Span,
    ) -> Result<Option<TypeAtom>, TcError> {
        // Resolve root type from place root identifier.
        let root_bytes = slice_span(self.src, place.root);
        let root_atom =
            TypeAtom::new(root_bytes).ok_or(TcError::PlaceParseFailed { span: place.root })?;

        let mut ty = if let Some(rty) = resource_ty(self.resources, root_atom) {
            rty
        } else if let Some(idx) = find_local(&self.locals, self.local_len, root_atom) {
            self.local_tys[idx]
        } else {
            return Ok(None);
        };

        // Walk each step, resolving through struct fields / array elements.
        for step in place.steps.iter() {
            match *step {
                crate::typecheck::place::Step::Field(field) => {
                    let Some(next) = struct_field_ty(self.nominals, ty, field) else {
                        return Err(TcError::FieldNotFound { span: place_abs });
                    };
                    ty = next;
                }
                crate::typecheck::place::Step::Index(_)
                | crate::typecheck::place::Step::DynamicIndex(_) => {
                    let Some(elem) = array_elem_type(ty) else {
                        return Err(TcError::FieldNotFound { span: place_abs });
                    };
                    ty = elem;
                }
            }
        }

        Ok(Some(ty))
    }
}

/// Unified suspend blocker return — why suspension is blocked at this site.
pub(super) enum SuspendBlocker {
    Frame(Span),
    Undeclared,
    BorrowLive { frame_span: Span },
}

/// Source line/column of a span start, as *debug info only* (Q2 — spans never
/// participate in obligation identity). `(0, 0)` means "no source location"
/// for compiler-generated sites (prologue checks, epilogue checks).
pub(super) fn span_line_col(src: &[u8], span: Span) -> (u32, u32) {
    if span.start == 0 {
        return (0, 0);
    }
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    let end = core::cmp::min(span.start, src.len());
    for &b in &src[..end] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The `in.i` bound-variable name for input slot `i`. No `alloc::format!`:
/// its machinery carries an unwinding landing pad the hosted `-nodefaultlibs`
/// link cannot resolve (same rationale as `verifier::model::utf8_lossy`).
fn var_in_ref(i: usize) -> String {
    let mut s = String::with_capacity(8);
    s.push_str("in.");
    verifier::model::push_u32_decimal(&mut s, i as u32);
    s
}

/// The `out.i` bound-variable name for output slot `i`.
fn var_out_ref(i: usize) -> String {
    let mut s = String::with_capacity(9);
    s.push_str("out.");
    verifier::model::push_u32_decimal(&mut s, i as u32);
    s
}

impl<'a, 'r> IrWordGen<'a, 'r> {
    /// C1 site: record a `subtype-range` obligation for a subtype-typed word
    /// input at the callee-entry prologue (static-verification.md §7.1).
    /// Emitted independently of `--checks` (FR-1): the artifact is complete
    /// even when the runtime trap is not inserted.
    pub(super) fn record_subtype_param_obligation(&mut self, i: usize, st: &SubtypeInfo) {
        if let Some(ctx) = self.extraction.as_mut() {
            ctx.record(
                OblKind::SubtypeRange,
                Formula::InRange {
                    value: Oel::Var {
                        name: var_in_ref(i),
                    },
                    lo: st.min,
                    hi: st.max,
                },
                0,
                0,
                Provenance::Direct,
            );
        }
    }

    /// C2 site: record a `subtype-range` obligation for a subtype-typed return
    /// value at the epilogue.
    pub(super) fn record_subtype_return_obligation(&mut self, i: usize, st: &SubtypeInfo) {
        if let Some(ctx) = self.extraction.as_mut() {
            ctx.record(
                OblKind::SubtypeRange,
                Formula::InRange {
                    value: Oel::Var {
                        name: var_out_ref(i),
                    },
                    lo: st.min,
                    hi: st.max,
                },
                0,
                0,
                Provenance::Direct,
            );
        }
    }

    /// C3 site: record a `subtype-range` obligation for an `as T` narrowing
    /// cast (names.rs `compile_cast`). v1 provenance is `$top` — the cast
    /// operand's value flow is not tracked until P5's interval engine — so the
    /// record carries `Provenance::Opaque` and can be discharged by no one.
    pub(super) fn record_cast_obligation(
        &mut self,
        from_ty: TypeAtom,
        to_ty: TypeAtom,
        st: &SubtypeInfo,
        span: Span,
    ) {
        if let Some(ctx) = self.extraction.as_mut() {
            let (line, col) = span_line_col(self.src, span);
            ctx.record(
                OblKind::SubtypeRange,
                Formula::InRange {
                    value: Oel::Cast {
                        // No `String::from_utf8_lossy`: its toolchain build
                        // carries an unwinding landing pad the hosted
                        // `-nodefaultlibs` link cannot resolve.
                        from: verifier::model::utf8_lossy(from_ty.as_bytes()),
                        to: verifier::model::utf8_lossy(to_ty.as_bytes()),
                        arg: Box::new(Oel::Var {
                            name: "$top".to_string(),
                        }),
                    },
                    lo: st.min,
                    hi: st.max,
                },
                line,
                col,
                Provenance::Opaque,
            );
        }
    }

    pub(super) fn finish(mut self, span: Span) -> Result<IrWordOutput<'r>, TcError> {
        self.fill_subtype_bases(span)?;
        self.fill_type_classes(span)?;
        self.word.apertures = core::mem::replace(&mut self.word_apertures, FixedVec::new());
        self.word.bound = self.acc;
        let word = unsafe {
            let arena = &mut *self.arena;
            let w = arena.alloc(self.word, span)?;
            &*(w as *const lir::Word)
        };
        Ok(IrWordOutput {
            word,
            extra_words: self.extra_words,
        })
    }

    /// Populate `word.subtype_bases` so the IR verifier can accept a subtype
    /// value where its base type is declared (subsumption).  Runs once the
    /// word's type table is final.
    pub(super) fn fill_subtype_bases(&mut self, span: Span) -> Result<(), TcError> {
        let mut bases: FixedVec<lir::TypeId, 64> = FixedVec::new();
        let n = self.word.types.len();
        for i in 0..n {
            let atom_opt = self.word.types.iter().nth(i);
            let Some(atom) = atom_opt else {
                break;
            };
            let ty = TypeAtom::new(atom.as_bytes()).unwrap_or(TypeAtom::EMPTY);
            let base = match find_subtype(self.subtypes, ty) {
                Some(st) => self.ty_id_of_type(st.base, span)?,
                None => lir::TY_EMPTY,
            };
            bases
                .push(base)
                .map_err(|_| TcError::TypeTableFull { span })?;
        }
        self.word.subtype_bases = bases;
        Ok(())
    }

    /// Populate `word.type_classes` (decision D-13). Runs once the word's
    /// type table is final (after `fill_subtype_bases`, which may intern
    /// subtype-base types). This is the single site where classes are
    /// computed; backends consume the carried tags and never byte-match type
    /// names.
    pub(super) fn fill_type_classes(&mut self, span: Span) -> Result<(), TcError> {
        let mut classes: FixedVec<lir::TypeClass, 64> = FixedVec::new();
        let n = self.word.types.len();
        for i in 0..n {
            let atom_opt = self.word.types.iter().nth(i);
            let Some(atom) = atom_opt else {
                break;
            };
            let ty = TypeAtom::new(atom.as_bytes()).unwrap_or(TypeAtom::EMPTY);
            classes
                .push(ty.class())
                .map_err(|_| TcError::TypeTableFull { span })?;
        }
        self.word.type_classes = classes;
        Ok(())
    }

    /// Record a aperture use for the word being compiled (P4). Adds or updates
    /// the entry in `word_apertures` from the descriptor's aperture identity, and
    /// ORs the register's fused access bits. A bus aperture with no absolute
    /// base is unbindable (E3648).
    pub(super) fn record_aperture(
        &mut self,
        aperture: u16,
        access: super::mmio::AccessMode,
        span: Span,
    ) -> Result<(), TcError> {
        let Some(descriptor) = self.descriptor else {
            return Ok(());
        };
        let Some(spec) = descriptor.apertures().iter().find(|w| w.id == aperture) else {
            return Ok(());
        };
        if spec.kind == codegen_core::MmioApertureKind::Bus && spec.base.is_none() {
            return Err(TcError::MmioApertureUnbindable { span });
        }
        let bits = aperture_access_bits(access);
        for wu in self.word_apertures.iter_mut() {
            if wu.id == aperture {
                wu.access_mask |= bits;
                return Ok(());
            }
        }
        let _ = self.word_apertures.push(lir::ApertureUse {
            id: aperture,
            name: spec.name,
            kind: match spec.kind {
                codegen_core::MmioApertureKind::Bus => lir::ApertureKind::Bus,
                codegen_core::MmioApertureKind::Emulated => lir::ApertureKind::Emulated,
            },
            base: spec.base,
            size: spec.size,
            access_mask: bits,
            bind: match spec.reloc_isa {
                Some(codegen_core::RelocIsa::ArmThumbLdrLiteral) => lir::BindKind::ArmThumbLdrLiteral,
                Some(codegen_core::RelocIsa::RiscVHi20Lo12) => lir::BindKind::RiscVHi20Lo12,
                None => lir::BindKind::None,
            },
        });
        Ok(())
    }

    /// Unified suspend blocker — checks the three rejection dimensions
    /// (frame forbid, undeclared word, read-borrow liveness).
    ///
    /// Returns `Some` with the reason suspension is blocked.  The suspend
    /// sites compile_env_word and compile_call_quote call this and map every
    /// `Some` to `E5001 SuspendForbidden` with the blocker span in the
    /// diagnostic.  compile_task_run inlines the frame-forbid and liveness
    /// checks only: it deliberately skips the undeclared-capability check
    /// because its Handler frame discharges SUSPEND from the body.
    pub(super) fn suspend_blocker(
        &self,
        stack: &[Value; 256],
        sp: usize,
    ) -> Option<SuspendBlocker> {
        // 1. Any frame unconditionally forbids SUSPEND (Lock, MutBorrow, Isr)
        if let Some(s) = self
            .ctx
            .forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND))
        {
            return Some(SuspendBlocker::Frame(s));
        }
        // 2. No SUSPENDABLE capability granted (word didn't declare performs {suspend})
        if !self.ctx.ambient_grants.contains(CapSet::SUSPENDABLE) {
            return Some(SuspendBlocker::Undeclared);
        }
        // 3. Conditional-suspend frames: forbid SUSPEND while the borrow's
        //    scope is still live (driven by MATRIX[kind].conditional_suspend).
        for i in 0..self.ctx.depth() as usize {
            let f = &self.ctx.frames()[i];
            if !crate::typecheck::context::row(f.kind).conditional_suspend {
                continue;
            }
            if let Some(s) = f.scope() {
                if self.stack_has_scope(stack, sp, s) || self.local_has_scope_live(s) {
                    return Some(SuspendBlocker::BorrowLive { frame_span: f.span });
                }
            }
        }
        None
    }
}

pub struct IrWordOutput<'r> {
    pub word: &'r lir::Word,
    pub extra_words: FixedVec<&'r lir::Word, { arena::QUOTE_WORD_CAP }>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_ir_word<'r>(
    decl: &DeclAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    descriptor: Option<&codegen_core::compiled_desc::CompiledDescriptor>,
    resources: &ResourceDb,
    nominals: &NominalDb,
    iso: &IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    sig: &WordSig,
    arena: &mut arena::ArenaAllocator,
    extraction: Option<&mut ExtractionCtx>,
    observer: &mut dyn TypecheckObserver,
) -> Result<IrWordOutput<'r>, TcError> {
    let name = lir_atom(slice_span(src, decl.name))?;
    let mut gen = IrWordGen::new(
        src,
        env,
        subtypes,
        mmio,
        descriptor,
        resources,
        nominals,
        iso,
        checks,
        allow_raw_casts,
        extraction,
        arena,
        *sig,
        name,
    )?;

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::EMPTY); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.in_len as usize) {
        stack[sp] = Value::Plain(sig.inputs[i]);
        sp += 1;
    }

    // Detect @interrupt(VEC) attribute — ISR bodies run under interrupt context.
    let is_isr = decl
        .attrs
        .iter()
        .any(|a| matches!(a, AttrAst::Interrupt { .. }));
    if is_isr {
        gen.word.performs = EffectSet::from_bits(EffectSet::INTERRUPT);
    }

    // Determine stack ceiling: ISR bodies get a smaller budget (N_isr), main gets N_main.
    let ceiling: High = if is_isr {
        High::Slots(32) // N_isr — limited budget for interrupt handlers
    } else {
        High::Top // No ceiling for ordinary words (checked at entry)
    };

    let mut cur = lir::BlockId(0);
    cur = gen.emit_prologue(cur, &mut stack, &mut sp, decl.requires, observer)?;

    // S-11: Parse compile-time capability set from `requires {caps}`.
    if let Some(cap_span) = decl.cap_set {
        let cap_bytes = &src[cap_span.start + 1..cap_span.end - 1]; // strip braces
        let mut caps = CapSet::empty();
        let mut cap_start = 0usize;
        while cap_start < cap_bytes.len() {
            // Skip whitespace and commas.
            while cap_start < cap_bytes.len()
                && (cap_bytes[cap_start] == b' '
                    || cap_bytes[cap_start] == b','
                    || cap_bytes[cap_start] == b'\n')
            {
                cap_start += 1;
            }
            if cap_start >= cap_bytes.len() {
                break;
            }
            // Find end of this capability name.
            let mut cap_end = cap_start;
            while cap_end < cap_bytes.len()
                && cap_bytes[cap_end] != b','
                && cap_bytes[cap_end] != b' '
                && cap_bytes[cap_end] != b'\n'
            {
                cap_end += 1;
            }
            let name = &cap_bytes[cap_start..cap_end];
            // Map known capability names to CapSet bits.
            if name == b"suspendable" {
                caps = caps.union(CapSet::from_bits(CapSet::SUSPENDABLE));
            } else if name.starts_with(b"write(") && name.ends_with(b")") {
                caps = caps.union(CapSet::from_bits(CapSet::WRITE));
            } else {
                // Unknown capability — list known names.
                let span = Span::new(cap_span.start + 1 + cap_start, cap_span.start + 1 + cap_end);
                return Err(TcError::Internal { span });
                // TODO: proper unknown-capability error with known list
            }
            cap_start = cap_end;
        }
        gen.word.requires = caps;
    }

    // S-8: Seed pointer-typed parameters with synthetic PlaceIds.
    // The prologue pushes inputs as Value::Plain; we replace PTR/PTR_MUT
    // params with Value::Ptr carrying a synthetic param PlaceId so that
    // borrow-checking tracks aliasing through pointer parameters (D-6).
    let stack_start = 0usize;
    for i in 0..(sig.in_len as usize) {
        if sig.inputs[i] == TypeAtom::PTR || sig.inputs[i] == TypeAtom::PTR_MUT {
            let mutable = sig.inputs[i] == TypeAtom::PTR_MUT;
            // Build a synthetic root name for the param ledger entry.
            let mut root_buf = [0u8; 12];
            root_buf[..7].copy_from_slice(b"_param_");
            let mut v = i as u32;
            let mut pos = 7;
            loop {
                root_buf[pos] = b'0' + (v % 10) as u8;
                pos += 1;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            let root_atom = TypeAtom::new(&root_buf[..pos]).unwrap_or(TypeAtom::EMPTY);
            let param_id = PlaceId(PARAM_BASE + i as u16);
            // Add to ledger so the borrow checker knows about this param.
            let _ = mint_id(
                &mut gen.ledger,
                &mut gen.ledger_len,
                root_atom,
                TypeAtom::EMPTY,
                Span::new(0, 0),
            );
            let idx = stack_start + i;
            if idx < sp {
                stack[idx] = Value::Ptr {
                    ty: TypeAtom::EMPTY,
                    mutable,
                    place: param_id,
                };
            }
        }
    }

    // S3: seed context stack for this word.
    gen.ctx.reset();
    if is_isr {
        gen.ctx.push(
            ContextKind::Isr,
            FrameParam::None,
            decl.body.unwrap_or(decl.name),
        )?;
    }
    // S9: push Bounded frame when the stack ceiling is finite.
    if ceiling != High::Top {
        gen.ctx.push(
            ContextKind::Bounded,
            FrameParam::None,
            decl.body.unwrap_or(decl.name),
        )?;
    }
    if (decl.effect_bits & 1) != 0 {
        gen.ctx.push(
            ContextKind::WordBody,
            FrameParam::None,
            decl.body.unwrap_or(decl.name),
        )?;
    }

    if let Some(body_span) = decl.body {
        cur = gen.compile_span(cur, &mut stack, &mut sp, body_span, true, observer)?;
    }

    // Pop context frames back to base depth before finish.
    while gen.ctx.depth() > 0 {
        gen.ctx.pop();
    }

    // S8: E5005 — declared performs must cover computed performs.
    if decl.has_explicit_performs {
        let declared = EffectSet::from_bits(decl.effect_bits);
        let missing = gen.word.performs.minus(declared);
        if !missing.is_empty() {
            return Err(TcError::EffectNotDeclared {
                span: decl.body.unwrap_or(decl.name),
            });
        }
    }

    if !gen.check_no_scoped_live(&stack, sp) {
        return Err(TcError::BorrowEscape {
            span: decl.body.unwrap_or(decl.name),
            kind: EscapeKind::AtClose,
        });
    }

    if gen.terminated {
        let span = decl.body.unwrap_or(decl.name);
        return gen.finish(span);
    }

    if sp != sig.out_len as usize {
        return Err(TcError::OutputCountMismatch {
            span: decl.body.unwrap_or(decl.name),
        });
    }
    for (i, v) in stack.iter().enumerate().take(sig.out_len as usize) {
        let got = v.to_type_atom();
        if !type_compatible(got, sig.outputs[i], subtypes) {
            return Err(TcError::OutputTypeMismatch {
                span: decl.body.unwrap_or(decl.name),
            });
        }
    }

    cur = gen.emit_epilogue(cur, &mut stack, &mut sp, decl.ensures, observer)?;
    gen.emit_op(cur, lir::OpKind::Ret, decl.body.unwrap_or(decl.name))?;
    let span = decl.body.unwrap_or(decl.name);
    let result = gen.finish(span)?;

    // C3: check word's high against the ceiling.
    let word_high = result.word.bound.high;
    if word_high.exceeds(ceiling) {
        if is_isr {
            return Err(TcError::IsrStack {
                span: decl.body.unwrap_or(decl.name),
            });
        } else if ceiling == High::Top {
            // On a bounded profile, Top at entry is an error (5100).
            // S9: this branch is reachable only when a bounded profile is
            // implemented for non-ISR words.  For v1 the ceiling is Top for
            // all non-ISR words and Top.exceeds(Top) is false, so this line
            // is unreachable in practice.
            return Err(TcError::StackUnbounded {
                span: decl.body.unwrap_or(decl.name),
            });
        } else {
            return Err(TcError::StackExceedsBudget {
                span: decl.body.unwrap_or(decl.name),
            });
        }
    }

    Ok(result)
}

pub fn lir_atom(bytes: &[u8]) -> Result<lir::Atom, TcError> {
    lir::Atom::new(bytes).ok_or(TcError::AtomTooLong {
        span: Span::new(0, 0),
    })
}
