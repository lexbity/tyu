use crate::typecheck::db::{
    enum_variant_value, is_iso_type, resource_sharing_class, resource_ty, struct_field_ty, IsoDb,
    NominalDb, ResourceDb, SubtypeInfo,
};
use crate::typecheck::error::{ChecksMode, TcError};
use crate::typecheck::mmio::mmio_type_width_bytes;
use crate::typecheck::mmio::{
    access_can_read, access_can_write, field_mask_shift, resolve_mmio_place, MmioDb, MmioResolved,
};
use crate::typecheck::parse::{
    capture_balanced, capture_scoped_block, parse_place, read_qualified_name,
};
use crate::typecheck::util::parse_u32_any;
use crate::typecheck::util::{
    align_up, apply_sig, array_elem_type, array_len, chan_elem_type, check_no_scoped_live,
    field_align, find_local, find_subtype, lookup, parse_i64_token, pop, push, region_ref_type,
    slice_span, slice_type_of_elem, type_compatible, type_size_bytes,
};
use crate::typecheck::value::Value;
use crate::types::{TypeAtom, WordEntry, WordSig};
use core::mem::MaybeUninit;
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::parse::DeclAst;
use frontend::span::Span;
use frontend::token::{Token, TokenKind};
use ir::{self as lir, CapSet, EffectSet, StackBound};

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

struct QuoteSig {
    sig: WordSig,
    performs: EffectSet,
    requires: CapSet,
    bound: StackBound,
    body: Span,
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
    resources: &'a ResourceDb,
    nominals: &'a NominalDb,
    iso: &'a IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    sig: WordSig,

    locals: [TypeAtom; 64],
    local_tys: [TypeAtom; 64],
    local_live: [bool; 64],
    local_scoped: [u16; 64],
    local_len: usize,

    next_scope: u16,
    scope_stack: [u16; 16],
    scope_sp: usize,

    locked_resource: Option<TypeAtom>,
    in_lock: bool,

    terminated: bool,
    word: lir::Word,
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
        resources: &'a ResourceDb,
        nominals: &'a NominalDb,
        iso: &'a IsoDb,
        checks: ChecksMode,
        allow_raw_casts: bool,
        arena: &mut arena::ArenaAllocator,
        sig: WordSig,
        name: lir::Atom,
    ) -> Result<Self, TcError> {
        let arena = arena as *mut arena::ArenaAllocator;
        let mut types: FixedVec<lir::Atom, 64> = FixedVec::new();
        let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
        let z = lir::AT_EMPTY;
        types.push(z).map_err(|_| TcError::TypeTableFull {
            span: Span::new(0, 0),
        })?;
        type_sizes.push(0).map_err(|_| TcError::TypeTableFull {
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
        types
            .push(lir::AT_BOOL)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(1).map_err(|_| TcError::TypeTableFull {
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
        types
            .push(lir::AT_PTR)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
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
        types
            .push(lir::AT_MMIO)
            .map_err(|_| TcError::TypeTableFull {
                span: Span::new(0, 0),
            })?;
        type_sizes.push(8).map_err(|_| TcError::TypeTableFull {
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
            resources,
            nominals,
            iso,
            checks,
            allow_raw_casts,
            sig,
            locals: [TypeAtom::EMPTY; 64],
            local_tys: [TypeAtom::EMPTY; 64],
            local_live: [false; 64],
            local_scoped: [0u16; 64],
            local_len: 0,
            next_scope: 1,
            scope_stack: [0u16; 16],
            scope_sp: 0,
            locked_resource: None,
            in_lock: false,
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
                blocks,
            },
            extra_words,
            arena,
            quote_id,
        })
    }

    fn block_mut(&mut self, id: lir::BlockId) -> Result<&mut lir::Block, TcError> {
        self.word
            .blocks
            .get_mut(id.0 as usize)
            .ok_or(TcError::BlockNotFound {
                span: Span::new(0, 0),
            })
    }

    fn new_block(
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

    fn emit_op(&mut self, cur: lir::BlockId, kind: lir::OpKind, span: Span) -> Result<(), TcError> {
        let op = lir::Op { kind, span };
        let b = self.block_mut(cur)?;
        b.ops.push(op).map_err(|_| TcError::OpTableFull { span })?;
        Ok(())
    }

    fn emit_subtype_range_trap(
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

    fn resolve_place_pointee_ty(
        &self,
        place_bytes: &[u8],
        place_abs: Span,
    ) -> Result<Option<TypeAtom>, TcError> {
        let mut segs: FixedVec<(TypeAtom, bool), 8> = FixedVec::new();
        let mut start = 0usize;
        for i in 0..=place_bytes.len() {
            if i == place_bytes.len() || place_bytes[i] == b'.' {
                if i == start {
                    return Err(TcError::PlaceSegmentEmpty { span: place_abs });
                }
                let seg = &place_bytes[start..i];
                let mut has_index = false;
                let (name_bytes, _) = if let Some(pos) = seg.iter().position(|&b| b == b'\'') {
                    has_index = true;
                    (&seg[..pos], &seg[pos + 1..])
                } else {
                    (seg, &[][..])
                };
                let atom = TypeAtom::new(name_bytes)
                    .ok_or(TcError::PlaceSegmentEmpty { span: place_abs })?;
                segs.push((atom, has_index))
                    .map_err(|_| TcError::PlaceSegmentEmpty { span: place_abs })?;
                start = i + 1;
            }
        }
        if segs.is_empty() {
            return Ok(None);
        }

        let (root, root_index) = *segs.get(0).expect("len > 0 checked above");
        let mut ty = if let Some(rty) = resource_ty(self.resources, root) {
            rty
        } else if let Some(idx) = find_local(&self.locals, self.local_len, root) {
            self.local_tys[idx]
        } else {
            return Ok(None);
        };
        if root_index {
            let Some(elem) = array_elem_type(ty) else {
                return Err(TcError::FieldNotFound { span: place_abs });
            };
            ty = elem;
        }

        for i in 1..segs.len() {
            let (field, has_index) = *segs.get(i).expect("i < segs.len() by loop guard");
            let Some(mut next) = struct_field_ty(self.nominals, ty, field) else {
                return Err(TcError::FieldNotFound { span: place_abs });
            };
            if has_index {
                let Some(elem) = array_elem_type(next) else {
                    return Err(TcError::FieldNotFound { span: place_abs });
                };
                next = elem;
            }
            ty = next;
        }

        Ok(Some(ty))
    }
}

impl<'a, 'r> IrWordGen<'a, 'r> {
    fn finish(self, span: Span) -> Result<IrWordOutput<'r>, TcError> {
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
    resources: &ResourceDb,
    nominals: &NominalDb,
    iso: &IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    sig: &WordSig,
    arena: &mut arena::ArenaAllocator,
    observer: &mut dyn TypecheckObserver,
) -> Result<IrWordOutput<'r>, TcError> {
    let name = lir_atom(slice_span(src, decl.name))?;
    let mut gen = IrWordGen::new(
        src,
        env,
        subtypes,
        mmio,
        resources,
        nominals,
        iso,
        checks,
        allow_raw_casts,
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
        .any(|a| slice_span(src, *a).starts_with(b"@interrupt("));
    if is_isr {
        gen.word.performs = EffectSet::from_bits(EffectSet::INTERRUPT);
    }

    let mut cur = lir::BlockId(0);
    cur = gen.emit_prologue(cur, &mut stack, &mut sp, decl.requires, observer)?;

    if let Some(body_span) = decl.body {
        // ISR body forbids suspend (and runs with ceiling = N_isr, checked in Phase 15).
        let allow_suspend = if is_isr {
            false
        } else {
            decl.effect_bits & 1 != 0
        };
        cur = gen.compile_span(
            cur,
            &mut stack,
            &mut sp,
            body_span,
            allow_suspend,
            true,
            observer,
        )?;
    }

    if !gen.check_no_scoped_live(&stack, sp) {
        return Err(TcError::BorrowEscape {
            span: decl.body.unwrap_or(decl.name),
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
        let got = match v {
            Value::Plain(t) => *t,
            Value::Scoped { ty, .. } => *ty,
            Value::Resource(_) => TypeAtom::RESOURCE,
            Value::Quot(_) => TypeAtom::QUOT,
            Value::MmioPlace(_) => TypeAtom::MMIO,
            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
        };
        if !type_compatible(got, sig.outputs[i], subtypes) {
            return Err(TcError::OutputTypeMismatch {
                span: decl.body.unwrap_or(decl.name),
            });
        }
    }

    cur = gen.emit_epilogue(cur, &mut stack, &mut sp, decl.ensures, observer)?;
    gen.emit_op(cur, lir::OpKind::Ret, decl.body.unwrap_or(decl.name))?;
    let span = decl.body.unwrap_or(decl.name);
    gen.finish(span)
}

pub fn lir_atom(bytes: &[u8]) -> Result<lir::Atom, TcError> {
    lir::Atom::new(bytes).ok_or(TcError::AtomTooLong {
        span: Span::new(0, 0),
    })
}
