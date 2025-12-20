use crate::types::{WordEntry, WordSig, TypeAtom};
use crate::typecheck::error::{TcError, ChecksMode};
use crate::typecheck::value::Value;
use crate::typecheck::db::{SubtypeInfo, ResourceDb, NominalDb, IsoDb, resource_ty, struct_field_ty, enum_variant_value, is_iso_type};
use crate::typecheck::mmio::{MmioDb, resolve_mmio_place, MmioResolved, access_can_read, access_can_write, field_mask_shift};
use crate::typecheck::util::{
    apply_sig, array_elem_type, array_len, chan_elem_type, check_no_scoped_live, find_local, find_subtype, lookup,
    parse_i64_token, push, pop, region_ref_type, slice_span, slice_type_of_elem, type_compatible, type_size_bytes,
    field_align, align_up,
};
use crate::typecheck::mmio::mmio_type_width_bytes;
use crate::typecheck::util::parse_u32_any;
use crate::typecheck::parse::{parse_place, read_qualified_name, capture_balanced, capture_scoped_block};
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::parse::DeclAst;
use frontend::span::Span;
use frontend::token::TokenKind;
use ir as lir;

const QUOTE_WORD_CAP: usize = 16;
const WORD_ARENA_CAP: usize = 256;

struct WordArena {
    len: usize,
    words: [MaybeUninit<lir::Word>; WORD_ARENA_CAP],
}

impl WordArena {
    const fn new() -> Self {
        Self {
            len: 0,
            words: [const { MaybeUninit::uninit() }; WORD_ARENA_CAP],
        }
    }

    unsafe fn reset(&mut self) {
        for i in 0..self.len {
            self.words[i].assume_init_drop();
        }
        self.len = 0;
    }

    unsafe fn alloc(&mut self, word: lir::Word, span: Span) -> Result<&'static lir::Word, TcError> {
        if self.len >= WORD_ARENA_CAP {
            return Err(TcError { code: 3906, span });
        }
        let slot = self.words[self.len].write(word);
        self.len += 1;
        let ptr = slot as *const lir::Word;
        Ok(&*ptr)
    }
}

struct WordArenaCell(UnsafeCell<WordArena>);

// Single-threaded compiler usage; manual sync boundary around the arena.
unsafe impl Sync for WordArenaCell {}

static WORD_ARENA: WordArenaCell = WordArenaCell(UnsafeCell::new(WordArena::new()));

pub fn reset_word_arena() {
    unsafe { (*WORD_ARENA.0.get()).reset() };
}

fn arena_alloc_word(word: lir::Word, span: Span) -> Result<&'static lir::Word, TcError> {
    unsafe { (*WORD_ARENA.0.get()).alloc(word, span) }
}

struct QuoteSig {
    sig: WordSig,
    may_suspend: bool,
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

struct IrWordGen<'a, 'w> {
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

    terminated: bool,
    word: lir::Word,
    extra_words: &'w mut FixedVec<&'static lir::Word, QUOTE_WORD_CAP>,
    quote_id: &'w mut u32,
}

impl<'a, 'w> IrWordGen<'a, 'w> {
    fn enum_base_ty(&self, enum_ty: TypeAtom) -> Option<TypeAtom> {
        for e in self.nominals.enums.iter() {
            if e.name == enum_ty {
                return Some(e.base);
            }
        }
        None
    }

    fn prim_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
        let b = ty.as_bytes();
        if b.starts_with(b"Chan(") {
            return Some((64, false));
        }
        let (bits, signed) = match b {
            b"u8" => (8, false),
            b"u16" => (16, false),
            b"u32" => (32, false),
            b"u64" => (64, false),
            b"usize" => (64, false),
            b"i8" => (8, true),
            b"i16" => (16, true),
            b"i32" => (32, true),
            b"i64" => (64, true),
            b"isize" => (64, true),
            b"bool" => (8, false),
            b"ptr" | b"ptr_mut" | b"str" | b"mmio" => (64, false),
            _ => return None,
        };
        Some((bits, signed))
    }

    fn ty_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
        if let Some(p) = self.prim_bits_signed(ty) {
            return Some(p);
        }
        if let Some(base) = self.enum_base_ty(ty) {
            return self.prim_bits_signed(base);
        }
        if let Some(st) = find_subtype(self.subtypes, ty) {
            return self.prim_bits_signed(st.base);
        }
        None
    }

    fn check_raw_cast_allowed(&self, from_ty: TypeAtom, to_ty: TypeAtom) -> bool {
        let from = from_ty.as_bytes();
        let to = to_ty.as_bytes();
        let is_ptrish = |t: &[u8]| matches!(t, b"ptr" | b"ptr_mut");
        let is_intish = |t: &[u8]| matches!(
            t,
            b"u8" | b"u16" | b"u32" | b"u64" | b"usize" | b"i8" | b"i16" | b"i32" | b"i64" | b"isize"
        );

        if is_ptrish(from) && is_intish(to) {
            return self.allow_raw_casts;
        }
        if is_intish(from) && is_ptrish(to) {
            return self.allow_raw_casts;
        }
        if is_ptrish(from) && is_ptrish(to) && from != to {
            return self.allow_raw_casts;
        }
        true
    }

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
        extra_words: &'w mut FixedVec<&'static lir::Word, QUOTE_WORD_CAP>,
        quote_id: &'w mut u32,
        sig: WordSig,
        name: lir::Atom,
    ) -> Result<Self, TcError> {
        let mut types: FixedVec<lir::Atom, 64> = FixedVec::new();
        let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
        let z = lir::Atom::new(b"").unwrap();
        let _ = types.push(z).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(0).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"i64").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(8).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"bool").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(1).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"str").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(8).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"ptr").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(8).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"ptr_mut").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(8).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"mmio").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = type_sizes.push(8).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;

        let mut lir_sig = lir::Sig::empty();
        lir_sig.in_len = sig.in_len;
        lir_sig.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            let atom = lir_atom_lossy(sig.inputs[i].as_bytes());
            lir_sig.inputs[i] = intern_type(&mut types, &mut type_sizes, atom, nominals, Span::new(0, 0))?;
        }
        for i in 0..(sig.out_len as usize) {
            let atom = lir_atom_lossy(sig.outputs[i].as_bytes());
            lir_sig.outputs[i] = intern_type(&mut types, &mut type_sizes, atom, nominals, Span::new(0, 0))?;
        }

        let mut blocks: FixedVec<lir::Block, 16> = FixedVec::new();
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for i in 0..(lir_sig.in_len as usize) {
            let _ = entry_stack.push(lir_sig.inputs[i]).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        }
        let entry_block = lir::Block { id: lir::BlockId(0), entry_stack, ops: FixedVec::new() };
        let _ = blocks.push(entry_block).map_err(|_| TcError { code: 3903, span: Span::new(0, 0) })?;

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
            locals: [TypeAtom::new(b"").unwrap(); 64],
            local_tys: [TypeAtom::new(b"").unwrap(); 64],
            local_live: [false; 64],
            local_scoped: [0u16; 64],
            local_len: 0,
            next_scope: 1,
            scope_stack: [0u16; 16],
            scope_sp: 0,
            locked_resource: None,
            terminated: false,
            word: lir::Word {
                name,
                sig: lir_sig,
                entry: lir::BlockId(0),
                types,
                type_sizes,
                blocks,
            },
            extra_words,
            quote_id,
        })
    }

    fn block_mut(&mut self, id: lir::BlockId) -> Result<&mut lir::Block, TcError> {
        self.word
            .blocks
            .get_mut(id.0 as usize)
            .ok_or(TcError { code: 3904, span: Span::new(0, 0) })
    }

    fn new_block(&mut self, stack: &[Value; 256], sp: usize, span: Span) -> Result<lir::BlockId, TcError> {
        let id = lir::BlockId(self.word.blocks.len() as u16);
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for i in 0..sp {
            let tid = self.ty_id_of_value(stack[i], span)?;
            let _ = entry_stack.push(tid).map_err(|_| TcError { code: 3902, span })?;
        }
        let b = lir::Block {
            id,
            entry_stack,
            ops: FixedVec::new(),
        };
        let _ = self.word.blocks.push(b).map_err(|_| TcError { code: 3903, span })?;
        Ok(id)
    }

    fn emit_op(&mut self, cur: lir::BlockId, kind: lir::OpKind, span: Span) -> Result<(), TcError> {
        let op = lir::Op { kind, span };
        let b = self.block_mut(cur)?;
        let _ = b.ops.push(op).map_err(|_| TcError { code: 3905, span })?;
        Ok(())
    }

    fn check_no_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        check_no_scoped_live(stack, sp)
    }

    fn any_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        if !check_no_scoped_live(stack, sp) {
            return true;
        }
        for i in 0..self.local_len {
            if self.local_live[i] && self.local_scoped[i] != 0 {
                return true;
            }
        }
        false
    }

    fn check_no_scoped_live_all(&self, stack: &[Value; 256], sp: usize) -> bool {
        !self.any_scoped_live(stack, sp)
    }

    fn enter_scope(&mut self) -> Option<u16> {
        if self.scope_sp >= self.scope_stack.len() {
            return None;
        }
        let id = self.next_scope;
        self.next_scope = self.next_scope.wrapping_add(1);
        self.scope_stack[self.scope_sp] = id;
        self.scope_sp += 1;
        Some(id)
    }

    fn leave_scope(&mut self, id: u16) {
        if self.scope_sp == 0 {
            return;
        }
        let top = self.scope_stack[self.scope_sp - 1];
        if top == id {
            self.scope_sp -= 1;
        }
    }

    fn stack_has_scope(&self, stack: &[Value; 256], sp: usize, scope: u16) -> bool {
        for i in 0..sp {
            if let Value::Scoped { scope: s, .. } = stack[i] {
                if s == scope {
                    return true;
                }
            }
        }
        false
    }

    fn invalidate_scope_locals(&mut self, scope: u16) {
        for i in 0..self.local_len {
            if self.local_scoped[i] == scope {
                self.local_scoped[i] = 0;
                self.local_live[i] = false;
            }
        }
    }

    fn local_slot(&self, idx: usize) -> u16 {
        self.sig.in_len as u16 + idx as u16
    }

    fn temp_base_slot(&self) -> u16 {
        (self.sig.in_len as u16)
            .wrapping_add(self.local_len as u16)
            .wrapping_add(1)
    }

    fn emit_subtype_range_trap(
        &mut self,
        cur: lir::BlockId,
        value_slot: u16,
        value_ty: lir::TypeId,
        st: &SubtypeInfo,
        span: Span,
    ) -> Result<(), TcError> {
        self.emit_op(cur, lir::OpKind::LocalGet { slot: value_slot, ty: value_ty }, span)?;
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

        self.emit_op(cur, lir::OpKind::LocalGet { slot: value_slot, ty: value_ty }, span)?;
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

    fn ty_id_of_type(&mut self, ty: TypeAtom, span: Span) -> Result<lir::TypeId, TcError> {
        intern_type(&mut self.word.types, &mut self.word.type_sizes, lir_atom_lossy(ty.as_bytes()), self.nominals, span)
    }

    fn ty_id_of_value(&mut self, v: Value, span: Span) -> Result<lir::TypeId, TcError> {
        match v {
            Value::Plain(t) => self.ty_id_of_type(t, span),
            Value::Scoped { ty, .. } => self.ty_id_of_type(ty, span),
            Value::Resource(_) => intern_type(&mut self.word.types, &mut self.word.type_sizes, lir_atom_lossy(b"resource"), self.nominals, span),
            Value::Quot(_) => intern_type(&mut self.word.types, &mut self.word.type_sizes, lir_atom_lossy(b"quot"), self.nominals, span),
            Value::MmioPlace(_) => Ok(lir::TY_MMIO),
            Value::Ptr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::Ptr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
            Value::MmioPtr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::MmioPtr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
        }
    }

    fn resolve_place_pointee_ty(&self, place_bytes: &[u8], place_abs: Span) -> Result<Option<TypeAtom>, TcError> {
        // Split `a.b.c` into atoms.
        let mut segs: FixedVec<(TypeAtom, bool), 8> = FixedVec::new();
        let mut start = 0usize;
        for i in 0..=place_bytes.len() {
            if i == place_bytes.len() || place_bytes[i] == b'.' {
                if i == start {
                    return Err(TcError { code: 3715, span: place_abs });
                }
                let seg = &place_bytes[start..i];
                let mut has_index = false;
                let (name_bytes, _) = if let Some(pos) = seg.iter().position(|&b| b == b'\'') {
                    has_index = true;
                    (&seg[..pos], &seg[pos + 1..])
                } else {
                    (seg, &[][..])
                };
                let atom = TypeAtom::new(name_bytes).ok_or(TcError { code: 3715, span: place_abs })?;
                segs.push((atom, has_index)).map_err(|_| TcError { code: 3715, span: place_abs })?;
                start = i + 1;
            }
        }
        if segs.len() == 0 {
            return Ok(None);
        }

        let (root, root_index) = *segs.get(0).unwrap();
        let mut ty = if let Some(rty) = resource_ty(self.resources, root) {
            rty
        } else if let Some(idx) = find_local(&self.locals, self.local_len, root) {
            self.local_tys[idx]
        } else {
            return Ok(None);
        };
        if root_index {
            let Some(elem) = array_elem_type(ty) else {
                return Err(TcError { code: 3716, span: place_abs });
            };
            ty = elem;
        }

        for i in 1..segs.len() {
            let (field, has_index) = *segs.get(i).unwrap();
            let Some(mut next) = struct_field_ty(self.nominals, ty, field) else {
                return Err(TcError { code: 3716, span: place_abs });
            };
            if has_index {
                let Some(elem) = array_elem_type(next) else {
                    return Err(TcError { code: 3716, span: place_abs });
                };
                next = elem;
            }
            ty = next;
        }

        Ok(Some(ty))
    }

    fn lir_sig_for_entry(&mut self, sig: &WordSig, span: Span) -> Result<lir::Sig, TcError> {
        let mut out = lir::Sig::empty();
        out.in_len = sig.in_len;
        out.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            out.inputs[i] = self.ty_id_of_type(sig.inputs[i], span)?;
        }
        for i in 0..(sig.out_len as usize) {
            out.outputs[i] = self.ty_id_of_type(sig.outputs[i], span)?;
        }
        Ok(out)
    }

    fn effect_has_suspend(bytes: &[u8]) -> bool {
        if bytes.len() < 4 {
            return false;
        }
        let needle = b"suspend";
        bytes.windows(needle.len()).any(|w| w == needle)
    }

    fn parse_quote_sig(&self, quot_span: Span) -> Result<QuoteSig, TcError> {
        if quot_span.end <= quot_span.start + 2 {
            return Err(TcError { code: 3760, span: quot_span });
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        let slice = &self.src[inner.start..inner.end];
        let mut lex = Lexer::new(slice);
        let first = lex.next();
        if first.kind != TokenKind::PunctLParen {
            return Err(TcError { code: 3760, span: quot_span });
        }
        let sig = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, first.span.start)
            .map_err(|code| TcError { code, span: quot_span })?;
        let sig_span = Span::new(inner.start + sig.start, inner.start + sig.end);
        let sig = crate::typecheck::parse::parse_word_sig(self.src, sig_span)
            .map_err(|e| TcError { code: e.code, span: e.span })?;

        let mut may_suspend = false;
        let next = lex.next();
        let next = if next.kind == TokenKind::EffectSet {
            let tok_bytes = &slice[next.span.start..next.span.end];
            may_suspend = Self::effect_has_suspend(tok_bytes);
            lex.next()
        } else {
            next
        };
        let body_start = if next.kind == TokenKind::Eof {
            inner.end
        } else {
            inner.start + next.span.start
        };
        let body = Span::new(body_start, inner.end);
        Ok(QuoteSig { sig, may_suspend, body })
    }

    fn quote_word_name(&mut self) -> lir::Atom {
        let id = *self.quote_id;
        *self.quote_id = self.quote_id.wrapping_add(1);
        let mut buf = [0u8; 16];
        let mut i = 0usize;
        buf[i] = b'_';
        i += 1;
        buf[i] = b'_';
        i += 1;
        buf[i] = b'q';
        i += 1;
        buf[i] = b'u';
        i += 1;
        buf[i] = b'o';
        i += 1;
        buf[i] = b't';
        i += 1;
        buf[i] = b'_';
        i += 1;
        let mut v = id;
        for _ in 0..8 {
            let digit = (v & 0xf) as u8;
            let b = if digit < 10 { b'0' + digit } else { b'a' + (digit - 10) };
            buf[i] = b;
            i += 1;
            v >>= 4;
        }
        lir::Atom::new(&buf[..i]).unwrap_or(lir::Atom::new(b"__quot").unwrap())
    }

    fn build_quote_word(&mut self, quot_span: Span) -> Result<(lir::Atom, WordSig, bool), TcError> {
        let parsed = self.parse_quote_sig(quot_span)?;
        let name = self.quote_word_name();
        let sig = parsed.sig;

        let word = {
            let extra_words = &mut *self.extra_words;
            let quote_id = &mut *self.quote_id;
            let mut qgen = IrWordGen::new(
                self.src,
                self.env,
                self.subtypes,
                self.mmio,
                self.resources,
                self.nominals,
                self.iso,
                self.checks,
                self.allow_raw_casts,
                extra_words,
                quote_id,
                sig,
                name,
            )?;

            let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
            let mut sp: usize = 0;
            for i in 0..(sig.in_len as usize) {
                stack[sp] = Value::Plain(sig.inputs[i]);
                sp += 1;
            }
            let cur = lir::BlockId(0);
            let cur = qgen.emit_prologue(cur, &mut stack, &mut sp, None)?;
            let _ = qgen.compile_span(cur, &mut stack, &mut sp, parsed.body, parsed.may_suspend, false)?;
            if !qgen.check_no_scoped_live(&stack, sp) {
                return Err(TcError { code: 3504, span: quot_span });
            }
            if !qgen.terminated {
                if sp != sig.out_len as usize {
                    return Err(TcError { code: 3220, span: quot_span });
                }
                for i in 0..(sig.out_len as usize) {
                    let got = match stack[i] {
                        Value::Plain(t) => t,
                        Value::Scoped { ty, .. } => ty,
                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    };
                    if !type_compatible(got, sig.outputs[i], self.subtypes) {
                        return Err(TcError { code: 3221, span: quot_span });
                    }
                }
            }
            qgen.word
        };

        let word = arena_alloc_word(word, quot_span)?;
        let _ = self
            .extra_words
            .push(word)
            .map_err(|_| TcError { code: 3905, span: quot_span })?;
        Ok((name, sig, parsed.may_suspend))
    }

    fn emit_prologue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        requires: Option<Span>,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;
        let n = self.sig.in_len as usize;
        // Move params into implicit slots 0..n, so checks can access them without disturbing stack order.
        for i in (0..n).rev() {
            let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(0, 0) })?;
            let _ = v;
            self.emit_op(cur, lir::OpKind::LocalSet { slot: i as u16, ty: self.word.sig.inputs[i] }, Span::new(0, 0))?;
        }

        if self.checks == ChecksMode::All {
            for i in 0..n {
                if let Some(st) = find_subtype(self.subtypes, self.sig.inputs[i]) {
                    self.emit_subtype_range_trap(cur, i as u16, self.word.sig.inputs[i], &st, Span::new(0, 0))?;
                }
            }
        }

        let mut params_on_stack = false;
        if self.checks != ChecksMode::Off && (self.checks == ChecksMode::Contracts || self.checks == ChecksMode::All) {
            if let Some(req) = requires {
                // Reload params onto stack for predicate evaluation.
                for i in 0..n {
                    self.emit_op(cur, lir::OpKind::LocalGet { slot: i as u16, ty: self.word.sig.inputs[i] }, req)?;
                    push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
                }
                cur = self.compile_quote_span(cur, stack, sp, req, false, false)?;
                // Predicate must leave an extra bool, preserving inputs.
                if *sp != n + 1 {
                    return Err(TcError { code: 3310, span: req });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
                    return Err(TcError { code: 3311, span: req });
                }
                for i in 0..n {
                    if stack[i] != Value::Plain(self.sig.inputs[i]) {
                        return Err(TcError { code: 3312, span: req });
                    }
                }
                // Consume bool and keep inputs.
                let _ = pop(stack, sp);
                self.emit_op(cur, lir::OpKind::TrapIfFalse { code: lir::TrapCode::ContractFail }, req)?;
                params_on_stack = true;
            }
        }

        // Reload params for word body execution.
        if !params_on_stack {
            for i in 0..n {
                self.emit_op(cur, lir::OpKind::LocalGet { slot: i as u16, ty: self.word.sig.inputs[i] }, Span::new(0, 0))?;
                push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
            }
        }
        Ok(cur)
    }

    fn emit_epilogue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        ensures: Option<Span>,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;

        if self.checks != ChecksMode::Off && (self.checks == ChecksMode::Contracts || self.checks == ChecksMode::All) {
            if let Some(ens) = ensures {
                let n = self.sig.out_len as usize;
                let base_sp = *sp;
                // Evaluate predicate with outputs on stack.
                cur = self.compile_quote_span(cur, stack, sp, ens, false, false)?;
                if *sp != base_sp + 1 {
                    return Err(TcError { code: 3320, span: ens });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
                    return Err(TcError { code: 3321, span: ens });
                }
                for i in 0..n {
                    if stack[i] != Value::Plain(self.sig.outputs[i]) {
                        return Err(TcError { code: 3322, span: ens });
                    }
                }
                let _ = pop(stack, sp);
                self.emit_op(cur, lir::OpKind::TrapIfFalse { code: lir::TrapCode::ContractFail }, ens)?;
            }
        }

        if self.checks == ChecksMode::All {
            // Check subtype returns without disturbing order.
            let n = self.sig.out_len as usize;
            let base_stack = *stack;
            let base_sp = *sp;
            let tmp_base = self.temp_base_slot();
            for i in (0..n).rev() {
                let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(0, 0) })?;
                let _ = v;
                self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp_base + i as u16, ty: self.word.sig.outputs[i] }, Span::new(0, 0))?;
            }
            for i in 0..n {
                if let Some(st) = find_subtype(self.subtypes, self.sig.outputs[i]) {
                    self.emit_subtype_range_trap(cur, tmp_base + i as u16, self.word.sig.outputs[i], &st, Span::new(0, 0))?;
                }
            }
            for i in 0..n {
                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet { slot: tmp_base + i as u16, ty: self.word.sig.outputs[i] },
                    Span::new(0, 0),
                )?;
                stack[i] = base_stack[i];
            }
            *sp = base_sp;
        }

        Ok(cur)
    }

    fn compile_quote_span(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        quot_span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        // Expect brackets at ends; just slice inside.
        if quot_span.end <= quot_span.start + 2 {
            return Ok(cur);
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals)
    }

    fn compile_span(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        let slice = &self.src[span.start..span.end];
        let mut lex = Lexer::new(slice);
        let mut terminated = false;

        loop {
            let tok = lex.next();
            if tok.kind == TokenKind::Eof {
                break;
            }
            if terminated {
                continue;
            }

            match tok.kind {
                TokenKind::Number => {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                    let num = parse_i64_token(&slice[tok.span.start..tok.span.end]).unwrap_or(0);
                    self.emit_op(cur, lir::OpKind::ConstI64(num), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::String => {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"str").unwrap()))?;
                    self.emit_op(cur, lir::OpKind::ConstStr(Span::new(span.start + tok.span.start, span.start + tok.span.end)), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::PunctArrowBind => {
                    if !allow_locals {
                        return Err(TcError { code: 3281, span });
                    }
                    let next = lex.next();
                    if next.kind == TokenKind::PunctLBrace {
                        let mut binds: FixedVec<DestructBind, 32> = FixedVec::new();
                        let mut saw_borrow = false;
                        loop {
                            let b = lex.next();
                            match b.kind {
                                TokenKind::PunctRBrace => break,
                                TokenKind::PunctComma => continue,
                                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                                    saw_borrow = true;
                                    let name_tok = lex.next();
                                    if name_tok.kind != TokenKind::Ident {
                                        return Err(TcError { code: 3201, span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end) });
                                    }
                                    let lname = TypeAtom::new(&slice[name_tok.span.start..name_tok.span.end]).ok_or(TcError {
                                        code: 3203,
                                        span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end),
                                    })?;
                                    binds
                                        .push(DestructBind {
                                            name: lname,
                                            borrow: true,
                                            mutable: b.kind == TokenKind::PunctAmpBang,
                                            span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end),
                                        })
                                        .map_err(|_| TcError { code: 3205, span })?;
                                }
                                TokenKind::Ident => {
                                    if saw_borrow {
                                        return Err(TcError { code: 3701, span: Span::new(span.start + b.span.start, span.start + b.span.end) });
                                    }
                                    let lname = TypeAtom::new(&slice[b.span.start..b.span.end]).ok_or(TcError {
                                        code: 3203,
                                        span: Span::new(span.start + b.span.start, span.start + b.span.end),
                                    })?;
                                    binds
                                        .push(DestructBind {
                                            name: lname,
                                            borrow: false,
                                            mutable: false,
                                            span: Span::new(span.start + b.span.start, span.start + b.span.end),
                                        })
                                        .map_err(|_| TcError { code: 3205, span })?;
                                }
                                _ => {
                                    return Err(TcError { code: 3702, span: Span::new(span.start + b.span.start, span.start + b.span.end) });
                                }
                            }
                        }
                        if binds.len() == 0 {
                            return Err(TcError { code: 3703, span });
                        }
                        let base = *stack.get(*sp - 1).ok_or(TcError { code: 3202, span })?;
                        let has_borrow = binds.iter().any(|b| b.borrow);
                        let (struct_ty, base_mut, _base_is_value) = match base {
                            Value::Ptr { ty, mutable } => (ty, mutable, false),
                            Value::Plain(t) if !has_borrow => (t, false, true),
                            _ => return Err(TcError { code: 3704, span }),
                        };
                        let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
                            return Err(TcError { code: 3716, span });
                        };
                        if binds.len() != sinfo.fields.len() {
                            return Err(TcError { code: 3705, span });
                        }
                        let base_tid = if base_mut { lir::TY_PTR_MUT } else { lir::TY_PTR };
                        let tmp = self.temp_base_slot();
                        self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: base_tid }, Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                        let _ = pop(stack, sp);

                        let mut offset: u32 = 0;
                        for (idx, field) in sinfo.fields.iter().enumerate() {
                            let bind = binds.get(idx).unwrap();
                            if bind.borrow && bind.mutable && !base_mut {
                                return Err(TcError { code: 3501, span: bind.span });
                            }
                            let fsize = type_size_bytes(field.ty, self.nominals).ok_or(TcError { code: 3718, span: bind.span })?;
                            let falign = field_align(fsize);
                            offset = align_up(offset, falign);
                            let field_offset = offset;
                            offset = offset.checked_add(fsize).ok_or(TcError { code: 3718, span: bind.span })?;

                            self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: base_tid }, bind.span)?;
                            push(stack, sp, Value::Ptr { ty: struct_ty, mutable: base_mut })?;
                            self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset: field_offset }, bind.span)?;

                            if bind.borrow {
                                let ptr_ty = if bind.mutable { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                                if find_local(&self.locals, self.local_len, bind.name).is_some() {
                                    return Err(TcError { code: 3204, span: bind.span });
                                }
                                let slot = self.local_slot(self.local_len);
                                self.locals[self.local_len] = bind.name;
                                self.local_tys[self.local_len] = ptr_ty;
                                self.local_live[self.local_len] = true;
                                self.local_scoped[self.local_len] = 0u16;
                                self.local_len += 1;
                                let tid = if bind.mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                                let _ = pop(stack, sp);
                                self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, bind.span)?;
                            } else {
                                let tid = self.ty_id_of_type(field.ty, bind.span)?;
                                self.emit_op(cur, lir::OpKind::Load { ty: tid }, bind.span)?;
                                let _ = pop(stack, sp);
                                push(stack, sp, Value::Plain(field.ty))?;
                                if find_local(&self.locals, self.local_len, bind.name).is_some() {
                                    return Err(TcError { code: 3204, span: bind.span });
                                }
                                let slot = self.local_slot(self.local_len);
                                self.locals[self.local_len] = bind.name;
                                self.local_tys[self.local_len] = field.ty;
                                self.local_live[self.local_len] = true;
                                self.local_scoped[self.local_len] = 0u16;
                                self.local_len += 1;
                                let _ = pop(stack, sp);
                                self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, bind.span)?;
                            }
                        }
                    } else {
                        let name = next;
                        if name.kind != TokenKind::Ident {
                            return Err(TcError { code: 3201, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                        }
                        let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                        if v == Value::Plain(TypeAtom::new(b"scoped").unwrap()) {
                            return Err(TcError { code: 3504, span });
                        }
	                        let ty = match v {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
                        let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(TcError {
                            code: 3203,
                            span: Span::new(span.start + name.span.start, span.start + name.span.end),
                        })?;
                        if find_local(&self.locals, self.local_len, lname).is_some() {
                            return Err(TcError { code: 3204, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                        }
                        if self.local_len >= self.locals.len() {
                            return Err(TcError { code: 3205, span });
                        }
                        let slot = self.local_slot(self.local_len);
                        self.locals[self.local_len] = lname;
                        self.local_tys[self.local_len] = ty;
                        self.local_live[self.local_len] = true;
                        self.local_scoped[self.local_len] = match v {
                            Value::Scoped { scope, .. } => scope,
                            _ => 0u16,
                        };
                        self.local_len += 1;
                        let tid = self.ty_id_of_type(ty, span)?;
                        self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                    }
                }
                TokenKind::PunctLBracket => {
                    let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                        .map_err(|code| TcError { code, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                    let q_span = Span::new(span.start + q.start, span.start + q.end);
                    push(stack, sp, Value::Quot(q_span))?;
                }
                TokenKind::PunctArrow => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let field_tok = lex.next();
                    if field_tok.kind != TokenKind::Ident {
                        return Err(TcError { code: 3716, span: op_span });
                    }
                    let field_atom = TypeAtom::new(&slice[field_tok.span.start..field_tok.span.end])
                        .ok_or(TcError { code: 3716, span: op_span })?;
                    let base = *stack.get(*sp - 1).ok_or(TcError { code: 3202, span: op_span })?;
                    let (struct_ty, mutable) = match base {
                        Value::Ptr { ty, mutable } => (ty, mutable),
                        _ => return Err(TcError { code: 3716, span: op_span }),
                    };
                    let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
                        return Err(TcError { code: 3716, span: op_span });
                    };
                    let mut offset: u32 = 0;
                    let mut found: Option<TypeAtom> = None;
                    for field in sinfo.fields.iter() {
                        let fsize = type_size_bytes(field.ty, self.nominals).ok_or(TcError { code: 3718, span: op_span })?;
                        let falign = field_align(fsize);
                        offset = align_up(offset, falign);
                        if field.name == field_atom {
                            found = Some(field.ty);
                            break;
                        }
                        offset = offset.checked_add(fsize).ok_or(TcError { code: 3718, span: op_span })?;
                    }
                    let Some(field_ty) = found else {
                        return Err(TcError { code: 3716, span: op_span });
                    };
                    let base_tid = if mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                    self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset }, op_span)?;
                    stack[*sp - 1] = Value::Ptr { ty: field_ty, mutable };
                }
                TokenKind::PunctApostrophe => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let mut const_idx: Option<u32> = None;
                    let mut is_dynamic = false;

                    let next = lex.next();
                    match next.kind {
                        TokenKind::Number => {
                            const_idx = parse_u32_any(&slice[next.span.start..next.span.end]);
                            if const_idx.is_none() {
                                return Err(TcError { code: 3519, span: op_span });
                            }
                        }
                        TokenKind::PunctLParen => {
                            let par = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, next.span.start)
                                .map_err(|code| TcError { code, span: op_span })?;
                            let inner = Span::new(span.start + par.start + 1, span.start + par.end - 1);
                            cur = self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals)?;
                            is_dynamic = true;
                        }
                        _ => {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                    }

                    if is_dynamic {
                        if *sp == 0 {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                        if stack[*sp - 1] != Value::Plain(TypeAtom::new(b"i64").unwrap()) {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                    }

                    let base_pos = if is_dynamic { *sp - 2 } else { *sp - 1 };
                    if base_pos >= *sp {
                        return Err(TcError { code: 3519, span: op_span });
                    }

                    let base = stack[base_pos];
                    let (elem_ty, scale, out_kind, base_tid) = match base {
                        Value::Plain(t) => {
                            let Some(elem) = array_elem_type(t) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            let len = array_len(t).ok_or(TcError { code: 3519, span: op_span })?;
                            if let Some(idx) = const_idx {
                                if idx >= len {
                                    return Err(TcError { code: 3518, span: op_span });
                                }
                            }
                            let size = type_size_bytes(elem, self.nominals).ok_or(TcError { code: 3519, span: op_span })?;
                            (elem, size, IndexOut::Value, lir::TY_PTR)
                        }
                        Value::Ptr { ty, mutable } => {
                            let Some(elem) = array_elem_type(ty) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            let len = array_len(ty).ok_or(TcError { code: 3519, span: op_span })?;
                            if let Some(idx) = const_idx {
                                if idx >= len {
                                    return Err(TcError { code: 3518, span: op_span });
                                }
                            }
                            let size = type_size_bytes(elem, self.nominals).ok_or(TcError { code: 3519, span: op_span })?;
                            let tid = if mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                            (elem, size, IndexOut::Ptr(mutable), tid)
                        }
                        Value::MmioPlace(MmioResolved::Reg(reg)) => {
                            let Some(width) = mmio_type_width_bytes(reg.reg_ty.as_bytes()) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            if let Some(idx) = const_idx {
                                if let Some(len) = reg.array_len {
                                    if idx >= len {
                                        return Err(TcError { code: 3604, span: op_span });
                                    }
                                }
                            }
                            (reg.reg_ty, width, IndexOut::Mmio, lir::TY_MMIO)
                        }
                        Value::MmioPlace(MmioResolved::Field(_)) => {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                        _ => return Err(TcError { code: 3519, span: op_span }),
                    };

                    match out_kind {
                        IndexOut::Value => {
                            stack[base_pos] = Value::Ptr { ty: elem_ty, mutable: false };
                        }
                        IndexOut::Ptr(mutable) => {
                            stack[base_pos] = Value::Ptr { ty: elem_ty, mutable };
                        }
                        IndexOut::Mmio => {
                            // Keep MMIO metadata; address arithmetic happens on stack.
                        }
                    }

                    if is_dynamic {
                        self.emit_op(cur, lir::OpKind::PtrAddIndex { ty: base_tid, scale }, op_span)?;
                        *sp = (*sp).saturating_sub(1);
                    } else {
                        let offset = const_idx.unwrap().saturating_mul(scale);
                        self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset }, op_span)?;
                    }

                    if matches!(out_kind, IndexOut::Value) {
                        let tid = self.ty_id_of_type(elem_ty, op_span)?;
                        self.emit_op(cur, lir::OpKind::Load { ty: tid }, op_span)?;
                        stack[base_pos] = Value::Plain(elem_ty);
                    }
                }
	                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
	                    let mut_tok = tok.kind == TokenKind::PunctAmpBang;
	                    let place = parse_place(&mut lex, slice).ok_or(TcError { code: 3500, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
	                    let place_bytes = &slice[place.full.start..place.full.end];
	                    let place_abs = Span::new(span.start + place.full.start, span.start + place.full.end);
	                    let root_atom = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap_or(TypeAtom::new(b"").unwrap());
	                    let mut const_addr: Option<u64> = None;

	                    if let Some(res) = resolve_mmio_place(self.mmio, self.src, place_bytes, place_abs)? {
	                        match res {
	                            MmioResolved::Reg(reg) => {
	                                if mut_tok && !access_can_write(reg.access) {
	                                    return Err(TcError { code: 3609, span: place_abs });
	                                }
	                                const_addr = Some(reg.addr);
	                                push(stack, sp, Value::MmioPtr { reg, mutable: mut_tok })?;
	                            }
	                            MmioResolved::Field(_) => return Err(TcError { code: 3608, span: place_abs }),
	                        }
	                    } else {
                        if resource_ty(self.resources, root_atom).is_some() {
                            // Resources are only borrowable inside their own lock scope.
                            if self.locked_resource != Some(root_atom) {
                                return Err(TcError { code: 3515, span: place_abs });
                            }
                        } else if mut_tok {
                            let root = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap();
                            if find_local(&self.locals, self.local_len, root).is_some() {
                                return Err(TcError { code: 3501, span: place.root_abs(span.start) });
                            }
                        }
                        if let Some(pointee) = self.resolve_place_pointee_ty(place_bytes, place_abs)? {
                            push(stack, sp, Value::Ptr { ty: pointee, mutable: mut_tok })?;
                        } else {
                            let ty = if mut_tok { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                            push(stack, sp, Value::Plain(ty))?;
                        }
                    }

	                    self.emit_op(
	                        cur,
	                        lir::OpKind::AddrOf {
	                            place: lir_atom_lossy(place_bytes),
	                            mutable: mut_tok,
	                            const_addr,
	                        },
	                        place_abs,
	                    )?;
	                }
                TokenKind::PunctPipeGreater => {
                    // `|>` send: ( Chan(T) T -- )
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let val = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;
                    let ch = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;

                    let val_ty = match val {
                        Value::Plain(t) => t,
                        Value::Scoped { ty, .. } => ty,
                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    };
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3731, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3731, span: op_span })?;
                    if !type_compatible(val_ty, elem, self.subtypes) {
                        return Err(TcError { code: 3732, span: op_span });
                    }

                    let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
                    let val_tid = self.ty_id_of_type(val_ty, op_span)?;
                    let mut sig = lir::Sig::empty();
                    sig.in_len = 2;
                    sig.out_len = 0;
                    sig.inputs[0] = ch_tid;
                    sig.inputs[1] = val_tid;
                    self.emit_op(
                        cur,
                        lir::OpKind::Call {
                            name: lir_atom_lossy(b"platform.channel.send"),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctLessPipe => {
                    // `<|` receive: ( Chan(T) -- T )
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let ch = pop(stack, sp).ok_or(TcError { code: 3733, span: op_span })?;
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3734, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3734, span: op_span })?;
                    push(stack, sp, Value::Plain(elem))?;

                    let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
                    let elem_tid = self.ty_id_of_type(elem, op_span)?;
                    let mut sig = lir::Sig::empty();
                    sig.in_len = 1;
                    sig.out_len = 1;
                    sig.inputs[0] = ch_tid;
                    sig.outputs[0] = elem_tid;
                    self.emit_op(
                        cur,
                        lir::OpKind::Call {
                            name: lir_atom_lossy(b"platform.channel.recv"),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                    let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                    if *sp == 0 {
                        return Err(TcError { code: 3505, span });
                    }
                    let top = stack[*sp - 1];
	                    let top_ty = match top {
	                        Value::Plain(t) => t,
	                        Value::Scoped { ty, .. } => ty,
	                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                    };

                    if let Some(elem) = array_elem_type(top_ty) {
                        // Milestone 13: Arrays -> scoped slices.
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let slice_ty = slice_type_of_elem(elem, mut_scope).ok_or(TcError { code: 3513, span })?;
                        let len = array_len(top_ty).ok_or(TcError { code: 3513, span })?;
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: slice_ty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(slice_ty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid, len },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else if top_ty == TypeAtom::new(b"Region").unwrap() {
                        // Milestone 5: Regions -> scoped borrowed region handles.
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let rty = region_ref_type(mut_scope);
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: rty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(rty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid, len: 0 },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else {
                        // `&[` / `&![` only apply to Arrays and Regions in v1.
                        return Err(TcError { code: 3515, span });
                    }
                }
                TokenKind::Ident | TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                    let mut qbuf = [0u8; 64];
                    let (name, name_span) = if tok.kind == TokenKind::Ident {
                        let (len, used, s) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                        let bytes = if used { &qbuf[..len] } else { &slice[tok.span.start..tok.span.end] };
                        (bytes, s)
                    } else {
                        (&slice[tok.span.start..tok.span.end], tok.span)
                    };
                    let name_abs = Span::new(span.start + name_span.start, span.start + name_span.end);

                    if name == b"true" || name == b"false" {
                        push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                        self.emit_op(cur, lir::OpKind::ConstBool(name == b"true"), name_abs)?;
                        continue;
                    }

                    if tok.kind == TokenKind::Ident {
                        if let Some(atom) = TypeAtom::new(name) {
                            if resource_ty(self.resources, atom).is_some() {
                                push(stack, sp, Value::Resource(atom))?;
                                continue;
                            }
                        }
                    }

                    if tok.kind == TokenKind::Ident {
                        // Enum tag literal: `Enum.Variant`
                        if let Some(dot) = name.iter().position(|&b| b == b'.') {
                            if dot + 1 < name.len() && !name[dot + 1..].iter().any(|&b| b == b'.') {
                                let enum_part = &name[..dot];
                                let var_part = &name[dot + 1..];
                                if let (Some(enum_ty), Some(var)) = (TypeAtom::new(enum_part), TypeAtom::new(var_part)) {
                                    if let Some(v) = enum_variant_value(self.nominals, enum_ty, var) {
                                        // Lower as: `ConstI64(v); Cast i64 -> EnumTy`
                                        self.emit_op(cur, lir::OpKind::ConstI64(v), name_abs)?;
                                        push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                                        let to = self.ty_id_of_type(enum_ty, name_abs)?;
                                        self.emit_op(cur, lir::OpKind::Cast { from: lir::TY_I64, to }, name_abs)?;
                                        let _ = pop(stack, sp);
                                        push(stack, sp, Value::Plain(enum_ty))?;
                                        continue;
                                    }
                                    // If the enum exists but the variant doesn't, prefer a stable enum diagnostic.
                                    for e in self.nominals.enums.iter() {
                                        if e.name == enum_ty {
                                            return Err(TcError { code: 3725, span: name_abs });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                        let is_load = name[0] == b'@';
                        let typed = name.len() > 1;
                        let ty_atom = if typed {
                            Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: name_abs })?)
                        } else {
                            None
                        };

                        if is_load {
                            let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                            match addr {
                                Value::MmioPtr { reg, .. } => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
	                                Value::MmioPlace(MmioResolved::Field(field)) => {
	                                    if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
	                                        return Err(TcError { code: 3610, span: name_abs });
	                                    }
	                                    if typed {
	                                        return Err(TcError { code: 3634, span: name_abs });
	                                    }
	                                    let (mask, shift) = field_mask_shift(&field.field);
	                                    push(stack, sp, Value::Plain(field.field.ty))?;
	                                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
	                                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
	                                    self.emit_op(
	                                        cur,
	                                        lir::OpKind::MmioVolLoadField {
	                                            reg_ty: reg_tid,
	                                            field_ty: tid,
	                                            place: lir_atom_lossy(slice_span(self.src, field.place_span)),
	                                            mask,
	                                            shift,
	                                        },
	                                        name_abs,
	                                    )?;
	                                    continue;
	                                }
                                Value::Ptr { ty, .. } => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.unwrap();
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    push(stack, sp, Value::Plain(ty))?;
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                Value::Plain(t) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::new(b"ptr").unwrap() && t != TypeAtom::new(b"ptr_mut").unwrap() {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.unwrap();
                                    push(stack, sp, Value::Plain(ty_atom))?;
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        } else {
                            let v = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
                            let addr = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
	                            let vty = match v {
	                                Value::Plain(t) => t,
	                                Value::Scoped { ty, .. } => ty,
	                                Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                                Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                                Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                                Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                                Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            };
                            match (addr, v) {
                                (Value::MmioPtr { reg, mutable: false }, _) => {
                                    let _ = reg;
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                (Value::MmioPtr { reg, mutable: true }, Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
	                                (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(_)) => {
	                                    if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
	                                        return Err(TcError { code: 3609, span: name_abs });
	                                    }
	                                    if typed {
	                                        return Err(TcError { code: 3634, span: name_abs });
	                                    }
	                                    if vty != field.field.ty {
	                                        return Err(TcError { code: 3231, span: name_abs });
	                                    }
	                                    let (mask, shift) = field_mask_shift(&field.field);
	                                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
	                                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
	                                    self.emit_op(
	                                        cur,
	                                        lir::OpKind::MmioVolStoreField {
	                                            reg_ty: reg_tid,
	                                            field_ty: tid,
	                                            place: lir_atom_lossy(slice_span(self.src, field.place_span)),
	                                            mask,
	                                            shift,
	                                        },
	                                        name_abs,
	                                    )?;
	                                    continue;
	                                }
                                (Value::Ptr { ty, mutable }, Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if !mutable {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.unwrap();
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    if vty != ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                (Value::Plain(t), Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::new(b"ptr_mut").unwrap() {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.unwrap();
                                    if vty != ty_atom {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        }
                    }

	                    if tok.kind == TokenKind::Ident {
	                        if let Some(res) = resolve_mmio_place(self.mmio, self.src, name, name_abs)? {
	                            let addr = match res {
	                                MmioResolved::Reg(reg) => reg.addr,
	                                MmioResolved::Field(field) => field.addr,
	                            };
	                            push(stack, sp, Value::MmioPlace(res))?;
	                            self.emit_op(
	                                cur,
	                                lir::OpKind::MmioPlace {
	                                    place: lir_atom_lossy(name),
	                                    addr,
	                                },
	                                name_abs,
	                            )?;
	                            continue;
	                        }
	                    }

	                    if name == b"dup" {
	                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
	                        let top_ty = match top {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        if is_iso_type(self.iso, top_ty) {
	                            return Err(TcError { code: 3742, span: name_abs });
	                        }
	                        push(stack, sp, top)?;
	                        push(stack, sp, top)?;
	                        let tid = self.ty_id_of_value(top, name_abs)?;
	                        self.emit_op(cur, lir::OpKind::Dup { ty: tid }, name_abs)?;
	                        continue;
	                    }
	                    if name == b"drop" {
	                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
	                        let top_ty = match top {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        if is_iso_type(self.iso, top_ty) {
	                            return Err(TcError { code: 3743, span: name_abs });
	                        }
	                        let tid = self.ty_id_of_value(top, name_abs)?;
	                        self.emit_op(cur, lir::OpKind::Drop { ty: tid }, name_abs)?;
	                        continue;
	                    }
                    if name == b"swap" {
                        let b = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        let a = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        push(stack, sp, b)?;
                        push(stack, sp, a)?;
                        let a_id = self.ty_id_of_value(a, name_abs)?;
                        let b_id = self.ty_id_of_value(b, name_abs)?;
                        self.emit_op(cur, lir::OpKind::Swap { a: a_id, b: b_id }, name_abs)?;
                        continue;
                    }

                    if name == b"as" || name == b"as?" || name == b"bitcast" {
                        let first = lex.next();
                        let start = first.span.start;
                        let (to_ty, next) = crate::typecheck::parse::parse_type_expr(slice, start).ok_or(TcError {
                            code: 3295,
                            span: Span::new(span.start + first.span.start, span.start + first.span.end),
                        })?;
                        lex.set_pos(next);
                        let v = pop(stack, sp).ok_or(TcError { code: 3297, span })?;
		                        let from_ty = match v {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        let subtype = find_subtype(self.subtypes, to_ty);
	                        if let Some(st) = subtype {
	                            if !type_compatible(from_ty, st.base, self.subtypes) {
	                                return Err(TcError { code: if name == b"as?" { 3298 } else { 3300 }, span });
	                            }
	                        }

	                        if name == b"bitcast" {
	                            // Disallow bitcasting directly to/from subtypes (use `as`/`as?` + checks instead).
	                            if subtype.is_some() {
	                                return Err(TcError { code: 3302, span: name_abs });
	                            }
	                            let Some((from_bits, _)) = self.ty_bits_signed(from_ty) else {
	                                return Err(TcError { code: 3303, span: name_abs });
	                            };
	                            let Some((to_bits, _)) = self.ty_bits_signed(to_ty) else {
	                                return Err(TcError { code: 3303, span: name_abs });
	                            };
	                            if from_bits != to_bits {
	                                return Err(TcError { code: 3304, span: name_abs });
	                            }
	                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
	                                return Err(TcError { code: 3305, span: name_abs });
	                            }
	                        } else {
	                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
	                                return Err(TcError { code: 3305, span: name_abs });
	                            }
	                        }

	                        // Model `as`/`as?` as an explicit type cast + optional subtype check.
	                        let from_id = self.ty_id_of_type(from_ty, name_abs)?;
	                        let to_id = self.ty_id_of_type(to_ty, name_abs)?;
	                        if name == b"bitcast" {
	                            self.emit_op(cur, lir::OpKind::Bitcast { from: from_id, to: to_id }, name_abs)?;
	                        } else {
	                            self.emit_op(cur, lir::OpKind::Cast { from: from_id, to: to_id }, name_abs)?;
	                        }
                        if name == b"as?" {
                            push(stack, sp, Value::Plain(to_ty))?;
                            if let Some(st) = subtype {
                                let tmp = self.temp_base_slot();
                                self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;

                                // Stack effect: `( to -- to ok )`
                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.min), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Ge,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.max), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Le,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::AndBool, name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                            } else {
                                self.emit_op(cur, lir::OpKind::ConstBool(true), name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                            }
                            continue;
                        }
                        // `as` / `bitcast`
                        push(stack, sp, Value::Plain(to_ty))?;
                        if name == b"as" && subtype.is_some() && self.checks == ChecksMode::All {
                            let tmp = self.temp_base_slot();
                            let st = subtype.unwrap();
                            self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                            self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;
                            self.emit_subtype_range_trap(cur, tmp, to_id, &st, name_abs)?;
                        }
                        continue;
                    }

                    if name == b"if" {
                        cur = self.compile_if(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"while" {
                        cur = self.compile_while(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"loop" {
                        cur = self.compile_loop(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"return" {
                        let want = self.sig.out_len as usize;
                        if *sp != want {
                            return Err(TcError { code: 3230, span });
                        }
                        if self.any_scoped_live(stack, *sp) {
                            return Err(TcError { code: 3511, span });
                        }
	                        for i in 0..want {
	                            let got = match stack[i] {
	                                Value::Plain(t) => t,
	                                Value::Scoped { ty, .. } => ty,
	                                Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                                Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                                Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                                Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                                Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            };
                            if !type_compatible(got, self.sig.outputs[i], self.subtypes) {
                                return Err(TcError { code: 3231, span });
                            }
                        }
                        self.emit_op(cur, lir::OpKind::Ret, name_abs)?;
                        terminated = true;
                        self.terminated = true;
                        continue;
                    }
                    if name == b"lock" {
                        let mut probe = lex;
                        let next = probe.next();
                        if next.kind == TokenKind::PunctLBracket {
                            lex = probe;
                            let _block = capture_scoped_block(&mut lex, slice, next.span)
                                .map_err(|code| TcError { code, span: name_abs })?;
                            let full_span = Span::new(span.start + next.span.start, span.start + lex.pos());
                            push(stack, sp, Value::Quot(full_span))?;
                        }
                        cur = self.compile_lock(cur, stack, sp, name_abs)?;
                        continue;
                    }
                    if name == b"call" {
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3758, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3758, span: name_abs }),
                        };
                        let (qname, qsig, may_suspend) = self.build_quote_word(body_span)?;
                        if may_suspend && !allow_suspend {
                            return Err(TcError { code: 3503, span: name_abs });
                        }

                        let entry = WordEntry {
                            name: TypeAtom::new(b"call").unwrap(),
                            sig: qsig,
                            may_suspend,
                        };
                        apply_sig(stack, sp, &entry, name_abs, self.subtypes)?;
                        let call_sig = self.lir_sig_for_entry(&qsig, name_abs)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::Call {
                                name: qname,
                                sig: call_sig,
                                may_suspend,
                            },
                            name_abs,
                        )?;
                        continue;
                    }
                    if name == b"platform.task.spawn" {
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3754, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3754, span: name_abs }),
                        };
                        let (qname, qsig, _may_suspend) = self.build_quote_word(body_span)?;
                        if qsig.in_len != 0 || qsig.out_len != 0 {
                            return Err(TcError { code: 3756, span: name_abs });
                        }

                        let task_ty = TypeAtom::new(b"Task").ok_or(TcError { code: 3757, span: name_abs })?;
                        let task_tid = self.ty_id_of_type(task_ty, name_abs)?;
                        push(stack, sp, Value::Plain(task_ty))?;
                        self.emit_op(
                            cur,
                            lir::OpKind::TaskSpawn {
                                name: qname,
                                task_ty: task_tid,
                            },
                            name_abs,
                        )?;
                        continue;
                    }
                    if name == b"platform.task.run" {
                        if !self.check_no_scoped_live_all(stack, *sp) {
                            return Err(TcError { code: 3502, span: name_abs });
                        }
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3750, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3751, span: name_abs }),
                        };
                        let base_stack = *stack;
                        let base_sp = *sp;
                        cur = self.compile_quote_span(cur, stack, sp, body_span, true, false)?;
                        if *sp != base_sp {
                            return Err(TcError { code: 3752, span: name_abs });
                        }
                        for i in 0..base_sp {
                            if stack[i] != base_stack[i] {
                                return Err(TcError { code: 3753, span: name_abs });
                            }
                        }
                        continue;
                    }

                    if let Some(idx) = find_local(&self.locals, self.local_len, TypeAtom::new(name).unwrap_or(TypeAtom::new(b"").unwrap())) {
                        if !self.local_live[idx] {
                            return Err(TcError { code: 3514, span: name_abs });
                        }
	                        if is_iso_type(self.iso, self.local_tys[idx]) {
	                            self.local_live[idx] = false;
	                        }
	                        if self.local_scoped[idx] != 0 {
	                            push(
	                                stack,
	                                sp,
                                Value::Scoped {
                                    ty: self.local_tys[idx],
                                    scope: self.local_scoped[idx],
                                },
                            )?;
                        } else {
                            push(stack, sp, Value::Plain(self.local_tys[idx]))?;
                        }
                        let tid = self.ty_id_of_type(self.local_tys[idx], name_abs)?;
                        self.emit_op(cur, lir::OpKind::LocalGet { slot: self.local_slot(idx), ty: tid }, name_abs)?;
                        continue;
                    }

                    let entry = lookup(self.env, name).ok_or(TcError { code: 3210, span: name_abs })?;
                    if entry.may_suspend && !allow_suspend {
                        return Err(TcError { code: 3503, span: name_abs });
                    }
                    if entry.may_suspend && !self.check_no_scoped_live_all(stack, *sp) {
                        return Err(TcError { code: 3502, span: name_abs });
                    }
                    apply_sig(stack, sp, entry, name_abs, self.subtypes)?;

                    let builtin = match name {
                        b"+" => Some(lir::OpKind::AddI64),
                        b"-" => Some(lir::OpKind::SubI64),
                        b"*" => Some(lir::OpKind::MulI64),
                        b">" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Gt }),
                        b"<" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Lt }),
                        b">=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ge }),
                        b"<=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Le }),
                        b"==" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Eq }),
                        b"!=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ne }),
                        b"and" => Some(lir::OpKind::AndBool),
                        b"or" => Some(lir::OpKind::OrBool),
                        b"not" => Some(lir::OpKind::NotBool),
                        _ => None,
                    };
                    if let Some(kind) = builtin {
                        self.emit_op(cur, kind, name_abs)?;
                    } else {
                        let call_sig = self.lir_sig_for_entry(&entry.sig, name_abs)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::Call {
                                name: lir_atom_lossy(name),
                                sig: call_sig,
                                may_suspend: entry.may_suspend,
                            },
                            name_abs,
                        )?;
                    }
                }
                _ => {
                    // ignore other punctuation in MVP
                }
            }
        }

        Ok(cur)
    }

    fn compile_if(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span })?;
        let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span })?;
        let cond = pop(stack, sp).ok_or(TcError { code: 3242, span })?;
        if cond != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
            return Err(TcError { code: 3243, span });
        }
        let then_span = match then_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3244, span }),
        };
        let else_span = match else_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3245, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let then_blk = self.new_block(&base_stack, base_sp, span)?;
        let else_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::BrIf { then_tgt: then_blk, else_tgt: else_blk }, span)?;

        let mut then_stack = base_stack;
        let mut then_sp = base_sp;
        let then_end = self.compile_quote_span(then_blk, &mut then_stack, &mut then_sp, then_span, allow_suspend, false)?;

        let mut else_stack = base_stack;
        let mut else_sp = base_sp;
        let else_end = self.compile_quote_span(else_blk, &mut else_stack, &mut else_sp, else_span, allow_suspend, false)?;

        if then_sp != else_sp {
            return Err(TcError { code: 3246, span });
        }
        for i in 0..then_sp {
            if then_stack[i] != else_stack[i] {
                return Err(TcError { code: 3247, span });
            }
        }

        let join_blk = self.new_block(&then_stack, then_sp, span)?;
        self.emit_op(then_end, lir::OpKind::Br { target: join_blk }, span)?;
        self.emit_op(else_end, lir::OpKind::Br { target: join_blk }, span)?;

        for i in 0..then_sp {
            stack[i] = then_stack[i];
        }
        *sp = then_sp;
        Ok(join_blk)
    }

    fn compile_while(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span })?;
        let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3252, span }),
        };
        let cond_span = match cond_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3253, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let header = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: header }, span)?;

        let mut cond_stack = base_stack;
        let mut cond_sp = base_sp;
        let cond_end = self.compile_quote_span(header, &mut cond_stack, &mut cond_sp, cond_span, allow_suspend, false)?;
        if cond_sp != base_sp + 1 {
            return Err(TcError { code: 3254, span });
        }
        if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
            return Err(TcError { code: 3255, span });
        }
        for i in 0..base_sp {
            if cond_stack[i] != base_stack[i] {
                return Err(TcError { code: 3256, span });
            }
        }
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cond_end, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3257, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3258, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: header }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    fn compile_loop(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3261, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let check_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: check_blk }, span)?;
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;

        self.emit_op(check_blk, lir::OpKind::ConstBool(true), span)?;
        self.emit_op(check_blk, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3262, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3263, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: check_blk }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    fn compile_lock(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        if self.locked_resource.is_some() {
            return Err(TcError { code: 3517, span });
        }
        let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3271, span }),
        };
        let locked = if *sp > 0 {
            match stack[*sp - 1] {
                Value::Resource(name) => {
                    let _ = pop(stack, sp);
                    Some(name)
                }
                _ => None,
            }
        } else {
            None
        };
        self.locked_resource = locked;
        let base_stack = *stack;
        let base_sp = *sp;
        let end = self.compile_quote_span(cur, stack, sp, body_span, false, true)?;
        if *sp != base_sp {
            return Err(TcError { code: 3272, span });
        }
        for i in 0..base_sp {
            if stack[i] != base_stack[i] {
                return Err(TcError { code: 3273, span });
            }
        }
        self.locked_resource = None;
        Ok(end)
    }
}

pub struct IrWordOutput {
    pub word: &'static lir::Word,
    pub extra_words: FixedVec<&'static lir::Word, QUOTE_WORD_CAP>,
}

pub fn build_ir_word(
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
) -> Result<IrWordOutput, TcError> {
    let name = lir_atom_lossy(slice_span(src, decl.name));
    let mut extra_words: FixedVec<&'static lir::Word, QUOTE_WORD_CAP> = FixedVec::new();
    let mut quote_id = 0u32;
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
        &mut extra_words,
        &mut quote_id,
        *sig,
        name,
    )?;

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.in_len as usize) {
        stack[sp] = Value::Plain(sig.inputs[i]);
        sp += 1;
    }

    let mut cur = lir::BlockId(0);
    cur = gen.emit_prologue(cur, &mut stack, &mut sp, decl.requires)?;

    if let Some(body_span) = decl.body {
        cur = gen.compile_span(cur, &mut stack, &mut sp, body_span, decl.effect_suspend, true)?;
    }

    if !gen.check_no_scoped_live(&stack, sp) {
        return Err(TcError { code: 3504, span: decl.body.unwrap_or(decl.name) });
    }

    if gen.terminated {
        let span = decl.body.unwrap_or(decl.name);
        let word = arena_alloc_word(gen.word, span)?;
        return Ok(IrWordOutput {
            word,
            extra_words,
        });
    }

    if sp != sig.out_len as usize {
        return Err(TcError { code: 3220, span: decl.body.unwrap_or(decl.name) });
    }
	    for i in 0..(sig.out_len as usize) {
	        let got = match stack[i] {
	            Value::Plain(t) => t,
	            Value::Scoped { ty, .. } => ty,
	            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	        };
	        if !type_compatible(got, sig.outputs[i], subtypes) {
	            return Err(TcError { code: 3221, span: decl.body.unwrap_or(decl.name) });
	        }
	    }

    cur = gen.emit_epilogue(cur, &mut stack, &mut sp, decl.ensures)?;
    gen.emit_op(cur, lir::OpKind::Ret, decl.body.unwrap_or(decl.name))?;
    let span = decl.body.unwrap_or(decl.name);
    let word = arena_alloc_word(gen.word, span)?;
    Ok(IrWordOutput { word, extra_words })
}

pub fn lir_atom_lossy(bytes: &[u8]) -> lir::Atom {
    lir::Atom::new(bytes).unwrap_or(lir::Atom::new(b"?").unwrap())
}

fn intern_type(
    types: &mut FixedVec<lir::Atom, 64>,
    type_sizes: &mut FixedVec<u32, 64>,
    atom: lir::Atom,
    nominals: &NominalDb,
    span: Span,
) -> Result<lir::TypeId, TcError> {
    for (i, a) in types.iter().enumerate() {
        if *a == atom {
            return Ok(lir::TypeId(i as u8));
        }
    }
    let idx = types.len();
    if idx > u8::MAX as usize {
        return Err(TcError { code: 3907, span });
    }
    let size = TypeAtom::new(atom.as_bytes()).and_then(|ty| type_size_bytes(ty, nominals)).unwrap_or(0);
    types.push(atom).map_err(|_| TcError { code: 3902, span })?;
    type_sizes.push(size).map_err(|_| TcError { code: 3902, span })?;
    Ok(lir::TypeId(idx as u8))
}
