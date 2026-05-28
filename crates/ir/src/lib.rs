#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use frontend::{fixed::FixedVec, span::Span, parse::Output};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Atom {
    len: u8,
    bytes: [u8; 32],
}

impl Atom {
    pub const fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 32 {
            return None;
        }
        let mut out = [0u8; 32];
        let mut i = 0usize;
        while i < bytes.len() {
            out[i] = bytes[i];
            i += 1;
        }
        Some(Self {
            len: bytes.len() as u8,
            bytes: out,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

// Static Atom constants for built-in types.
// These are used throughout the compiler pipeline to avoid repeated
// Atom::new(b"...").expect() calls. All fit in the 32-byte limit.
pub const AT_EMPTY: Atom = match Atom::new(b"") { Some(a) => a, None => unreachable!() };
pub const AT_I64: Atom = match Atom::new(b"i64") { Some(a) => a, None => unreachable!() };
pub const AT_BOOL: Atom = match Atom::new(b"bool") { Some(a) => a, None => unreachable!() };
pub const AT_STR: Atom = match Atom::new(b"str") { Some(a) => a, None => unreachable!() };
pub const AT_PTR: Atom = match Atom::new(b"ptr") { Some(a) => a, None => unreachable!() };
pub const AT_PTR_MUT: Atom = match Atom::new(b"ptr_mut") { Some(a) => a, None => unreachable!() };
pub const AT_MMIO: Atom = match Atom::new(b"mmio") { Some(a) => a, None => unreachable!() };
pub const AT_QUOT: Atom = match Atom::new(b"quot") { Some(a) => a, None => unreachable!() };
pub const AT_RESOURCE: Atom = match Atom::new(b"resource") { Some(a) => a, None => unreachable!() };

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeId(pub u8);

pub const TY_EMPTY: TypeId = TypeId(0);
pub const TY_I64: TypeId = TypeId(1);
pub const TY_BOOL: TypeId = TypeId(2);
pub const TY_STR: TypeId = TypeId(3);
pub const TY_PTR: TypeId = TypeId(4);
pub const TY_PTR_MUT: TypeId = TypeId(5);
pub const TY_MMIO: TypeId = TypeId(6);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sig {
    pub in_len: u8,
    pub out_len: u8,
    pub inputs: [TypeId; 8],
    pub outputs: [TypeId; 8],
}

impl Sig {
    pub const fn empty() -> Self {
        const Z: TypeId = TY_EMPTY;
        Self {
            in_len: 0,
            out_len: 0,
            inputs: [Z; 8],
            outputs: [Z; 8],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrapCode {
    ContractFail,
    SubtypeFail,
    AssertFail,
    StackOverflow,
    Unreachable,
}

pub const fn trap_code_u32(code: TrapCode) -> u32 {
    match code {
        TrapCode::ContractFail => 20,
        TrapCode::SubtypeFail => 21,
        TrapCode::AssertFail => 22,
        TrapCode::Unreachable => 23,
        TrapCode::StackOverflow => 10,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockId(pub u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpKind {
    ConstI64(i64),
    ConstBool(bool),
    ConstStr(Span),

    AddrOf { place: Atom, mutable: bool, const_addr: Option<u64> },
    MmioPlace { place: Atom, addr: u64 },
    ScopedEnter { ty: TypeId, len: u32 },
    TaskSpawn { name: Atom, task_ty: TypeId },
    PtrAddConst { ty: TypeId, offset: u32 },
    PtrAddIndex { ty: TypeId, scale: u32 },

    Dup { ty: TypeId },
    Drop { ty: TypeId },
    Swap { a: TypeId, b: TypeId },

    AddI64,
    SubI64,
    MulI64,
    Cmp { out: TypeId, kind: CmpKind },
    AndBool,
    OrBool,
    NotBool,

    LocalSet { slot: u16, ty: TypeId },
    LocalGet { slot: u16, ty: TypeId },

    Cast { from: TypeId, to: TypeId },
    Bitcast { from: TypeId, to: TypeId },

    Call { name: Atom, sig: Sig, may_suspend: bool },

    Load { ty: TypeId },
    Store { ty: TypeId },

    MmioVolLoad { ty: TypeId, place: Atom },
    MmioVolStore { ty: TypeId, place: Atom },
    MmioVolLoadField { reg_ty: TypeId, field_ty: TypeId, place: Atom, mask: u64, shift: u8 },
    MmioVolStoreField { reg_ty: TypeId, field_ty: TypeId, place: Atom, mask: u64, shift: u8 },

    // Produces `bool` while preserving the value (so `trap_if_false` can consume the bool).
    // Stack effect: `( ty -- ty bool )`
    CheckSubtype { ty: TypeId },
    TrapIfFalse { code: TrapCode },

    Br { target: BlockId },
    BrIf { then_tgt: BlockId, else_tgt: BlockId },
    Ret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CmpKind {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Op {
    pub kind: OpKind,
    pub span: Span,
}

pub struct Block {
    pub id: BlockId,
    pub entry_stack: FixedVec<TypeId, 32>,
    pub ops: FixedVec<Op, 96>,
}

pub struct Word {
    pub name: Atom,
    pub sig: Sig,
    pub entry: BlockId,
    pub types: FixedVec<Atom, 64>,
    pub type_sizes: FixedVec<u32, 64>,
    pub blocks: FixedVec<Block, 16>,
}

pub struct Module {
    pub name: Atom,
    pub words: FixedVec<Word, 64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyError {
    EntryBlockNotFound { span: Span },
    EntryStackLenMismatch { span: Span },
    EntryStackTypeMismatch { span: Span },
    CodeAfterTerminator { span: Span },
    TypeMismatch { span: Span },
    DropTypeMismatch { span: Span },
    SwapTypeMismatch { span: Span },
    LocalTypeMismatch { span: Span },
    CallStackUnderflow { span: Span },
    CallInputTypeMismatch { span: Span },
    LoadAddrNotPtr { span: Span },
    StoreValueTypeMismatch { span: Span },
    StoreAddrNotMutPtr { span: Span },
    MmioFieldAddrNotMmio { span: Span },
    MmioFieldTypeMismatch { span: Span },
    CheckSubtypeTypeMismatch { span: Span },
    TrapIfFalseNotBool { span: Span },
    BrTargetNotFound { span: Span },
    BrStackDepthMismatch { span: Span },
    BrStackContentMismatch { span: Span },
    BrIfCondNotBool { span: Span },
    BrIfTargetNotFound { span: Span },
    BrIfStackDepthMismatch { span: Span },
    BrIfStackContentMismatch { span: Span },
    RetStackDepthMismatch { span: Span },
    RetOutputTypeMismatch { span: Span },
    NotTerminated { span: Span },
    PopEmptyStack { span: Span },
    PushFullStack { span: Span },
}

impl VerifyError {
    pub fn code(self) -> u32 {
        match self {
            VerifyError::EntryBlockNotFound { .. } => 9001,
            VerifyError::EntryStackLenMismatch { .. } => 9002,
            VerifyError::EntryStackTypeMismatch { .. } => 9003,
            VerifyError::CodeAfterTerminator { .. } => 9010,
            VerifyError::TypeMismatch { .. } => 9011,
            VerifyError::DropTypeMismatch { .. } => 9012,
            VerifyError::SwapTypeMismatch { .. } => 9013,
            VerifyError::LocalTypeMismatch { .. } => 9016,
            VerifyError::CallStackUnderflow { .. } => 9017,
            VerifyError::CallInputTypeMismatch { .. } => 9018,
            VerifyError::LoadAddrNotPtr { .. } => 9019,
            VerifyError::StoreValueTypeMismatch { .. } => 9020,
            VerifyError::StoreAddrNotMutPtr { .. } => 9021,
            VerifyError::MmioFieldAddrNotMmio { .. } => 9022,
            VerifyError::MmioFieldTypeMismatch { .. } => 9023,
            VerifyError::CheckSubtypeTypeMismatch { .. } => 9024,
            VerifyError::TrapIfFalseNotBool { .. } => 9025,
            VerifyError::BrTargetNotFound { .. } => 9026,
            VerifyError::BrStackDepthMismatch { .. } => 9027,
            VerifyError::BrStackContentMismatch { .. } => 9028,
            VerifyError::BrIfCondNotBool { .. } => 9029,
            VerifyError::BrIfTargetNotFound { .. } => 9030,
            VerifyError::BrIfStackDepthMismatch { .. } => 9031,
            VerifyError::BrIfStackContentMismatch { .. } => 9032,
            VerifyError::RetStackDepthMismatch { .. } => 9033,
            VerifyError::RetOutputTypeMismatch { .. } => 9034,
            VerifyError::NotTerminated { .. } => 9035,
            VerifyError::PopEmptyStack { .. } => 9098,
            VerifyError::PushFullStack { .. } => 9099,
        }
    }

    pub fn span(self) -> Span {
        match self {
            VerifyError::EntryBlockNotFound { span }
            | VerifyError::EntryStackLenMismatch { span }
            | VerifyError::EntryStackTypeMismatch { span }
            | VerifyError::CodeAfterTerminator { span }
            | VerifyError::TypeMismatch { span }
            | VerifyError::DropTypeMismatch { span }
            | VerifyError::SwapTypeMismatch { span }
            | VerifyError::LocalTypeMismatch { span }
            | VerifyError::CallStackUnderflow { span }
            | VerifyError::CallInputTypeMismatch { span }
            | VerifyError::LoadAddrNotPtr { span }
            | VerifyError::StoreValueTypeMismatch { span }
            | VerifyError::StoreAddrNotMutPtr { span }
            | VerifyError::MmioFieldAddrNotMmio { span }
            | VerifyError::MmioFieldTypeMismatch { span }
            | VerifyError::CheckSubtypeTypeMismatch { span }
            | VerifyError::TrapIfFalseNotBool { span }
            | VerifyError::BrTargetNotFound { span }
            | VerifyError::BrStackDepthMismatch { span }
            | VerifyError::BrStackContentMismatch { span }
            | VerifyError::BrIfCondNotBool { span }
            | VerifyError::BrIfTargetNotFound { span }
            | VerifyError::BrIfStackDepthMismatch { span }
            | VerifyError::BrIfStackContentMismatch { span }
            | VerifyError::RetStackDepthMismatch { span }
            | VerifyError::RetOutputTypeMismatch { span }
            | VerifyError::NotTerminated { span }
            | VerifyError::PopEmptyStack { span }
            | VerifyError::PushFullStack { span } => span,
        }
    }
}

pub fn verify_module(m: &Module) -> Result<(), VerifyError> {
    for w in m.words.iter() {
        verify_word(w)?;
    }
    Ok(())
}

pub fn verify_word(w: &Word) -> Result<(), VerifyError> {
    let mut entry = None;
    for b in w.blocks.iter() {
        if b.id == w.entry {
            entry = Some(b);
            break;
        }
    }
    let Some(entry_block) = entry else {
        return Err(VerifyError::EntryBlockNotFound { span: Span::UNKNOWN });
    };
    if entry_block.entry_stack.len() != w.sig.in_len as usize {
        return Err(VerifyError::EntryStackLenMismatch { span: Span::UNKNOWN });
    }
    for i in 0..(w.sig.in_len as usize) {
        if *entry_block.entry_stack.get(i).expect("verified entry stack len") != w.sig.inputs[i] {
            return Err(VerifyError::EntryStackTypeMismatch { span: Span::UNKNOWN });
        }
    }

    for b in w.blocks.iter() {
        verify_block(w, b)?;
    }
    Ok(())
}

fn find_block(w: &Word, id: BlockId) -> Option<&Block> {
    w.blocks.iter().find(|b| b.id == id)
}

fn verify_block(w: &Word, b: &Block) -> Result<(), VerifyError> {
    let mut stack: [TypeId; 64] = [TY_EMPTY; 64];
    let mut sp = 0usize;
    for a in b.entry_stack.iter() {
        stack[sp] = *a;
        sp += 1;
    }

    let mut terminated = false;
    for op in b.ops.iter() {
        if terminated {
            return Err(VerifyError::CodeAfterTerminator { span: op.span });
        }
        match op.kind {
            OpKind::ConstI64(_) => {
                push(&mut stack, &mut sp, TY_I64, op.span)?;
            }
            OpKind::ConstBool(_) => {
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::ConstStr(_) => {
                push(&mut stack, &mut sp, TY_STR, op.span)?;
            }
            OpKind::AddrOf { mutable: false, .. } => {
                push(&mut stack, &mut sp, TY_PTR, op.span)?;
            }
            OpKind::AddrOf { mutable: true, .. } => {
                push(&mut stack, &mut sp, TY_PTR_MUT, op.span)?;
            }
            OpKind::MmioPlace { .. } => {
                push(&mut stack, &mut sp, TY_MMIO, op.span)?;
            }
            OpKind::ScopedEnter { ty, .. } => {
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::TaskSpawn { task_ty, .. } => {
                push(&mut stack, &mut sp, task_ty, op.span)?;
            }
            OpKind::PtrAddConst { ty, .. } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::PtrAddIndex { ty, .. } => {
                let idx = pop(&mut stack, &mut sp, op.span)?;
                if idx != TY_I64 {
                    return Err(VerifyError::CallInputTypeMismatch { span: op.span });
                }
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::Dup { ty } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::TypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
                push(&mut stack, &mut sp, top, op.span)?;
            }
            OpKind::Drop { ty } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                if top != ty {
                    return Err(VerifyError::DropTypeMismatch { span: op.span });
                }
            }
            OpKind::Swap { a, b } => {
                let top = pop(&mut stack, &mut sp, op.span)?;
                let below = pop(&mut stack, &mut sp, op.span)?;
                if top != b || below != a {
                    return Err(VerifyError::SwapTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, top, op.span)?;
                push(&mut stack, &mut sp, below, op.span)?;
            }
            OpKind::AddI64 | OpKind::SubI64 | OpKind::MulI64 => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_I64, op.span)?;
            }
            OpKind::Cmp { out, .. } => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, out, op.span)?;
            }
            OpKind::AndBool | OpKind::OrBool => {
                let _b1 = pop(&mut stack, &mut sp, op.span)?;
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::NotBool => {
                let _a1 = pop(&mut stack, &mut sp, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::LocalSet { ty, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
            }
            OpKind::LocalGet { ty, .. } => {
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::Cast { from, to } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != from {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, to, op.span)?;
            }
            OpKind::Bitcast { from, to } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != from {
                    return Err(VerifyError::LocalTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, to, op.span)?;
            }
            OpKind::Call { sig, .. } => {
                let need = sig.in_len as usize;
                if sp < need {
                    return Err(VerifyError::CallStackUnderflow { span: op.span });
                }
                for i in 0..need {
                    if stack[sp - need + i] != sig.inputs[i] {
                        return Err(VerifyError::CallInputTypeMismatch { span: op.span });
                    }
                }
                sp -= need;
                for i in 0..(sig.out_len as usize) {
                    push(&mut stack, &mut sp, sig.outputs[i], op.span)?;
                }
            }
            OpKind::Load { ty } => {
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if addr != TY_PTR && addr != TY_PTR_MUT {
                    return Err(VerifyError::LoadAddrNotPtr { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::Store { ty } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::StoreValueTypeMismatch { span: op.span });
                }
                if addr != TY_PTR_MUT {
                    return Err(VerifyError::StoreAddrNotMutPtr { span: op.span });
                }
            }
            OpKind::MmioVolLoad { ty, .. } => {
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if addr != TY_MMIO && addr != TY_PTR && addr != TY_PTR_MUT {
                    return Err(VerifyError::LoadAddrNotPtr { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
            }
            OpKind::MmioVolStore { ty, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let addr = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::StoreValueTypeMismatch { span: op.span });
                }
                if addr != TY_MMIO && addr != TY_PTR_MUT {
                    return Err(VerifyError::StoreAddrNotMutPtr { span: op.span });
                }
            }
            OpKind::MmioVolLoadField { field_ty, .. } => {
                let place = pop(&mut stack, &mut sp, op.span)?;
                if place != TY_MMIO {
                    return Err(VerifyError::MmioFieldAddrNotMmio { span: op.span });
                }
                push(&mut stack, &mut sp, field_ty, op.span)?;
            }
            OpKind::MmioVolStoreField { field_ty, .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                let place = pop(&mut stack, &mut sp, op.span)?;
                if place != TY_MMIO || v != field_ty {
                    return Err(VerifyError::MmioFieldTypeMismatch { span: op.span });
                }
            }
            OpKind::CheckSubtype { ty } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != ty {
                    return Err(VerifyError::CheckSubtypeTypeMismatch { span: op.span });
                }
                push(&mut stack, &mut sp, ty, op.span)?;
                push(&mut stack, &mut sp, TY_BOOL, op.span)?;
            }
            OpKind::TrapIfFalse { .. } => {
                let v = pop(&mut stack, &mut sp, op.span)?;
                if v != TY_BOOL {
                    return Err(VerifyError::TrapIfFalseNotBool { span: op.span });
                }
            }
            OpKind::Br { target } => {
                let Some(t) = find_block(w, target) else {
                    return Err(VerifyError::BrTargetNotFound { span: op.span });
                };
                if t.entry_stack.len() != sp {
                    return Err(VerifyError::BrStackDepthMismatch { span: op.span });
                }
                for (i, item) in stack.iter().enumerate().take(sp) {
                    if *t.entry_stack.get(i).expect("verified length matches sp") != *item {
                        return Err(VerifyError::BrStackContentMismatch { span: op.span });
                    }
                }
                terminated = true;
            }
            OpKind::BrIf { then_tgt, else_tgt } => {
                let cond = pop(&mut stack, &mut sp, op.span)?;
                if cond != TY_BOOL {
                    return Err(VerifyError::BrIfCondNotBool { span: op.span });
                }
                for &tgt in &[then_tgt, else_tgt] {
                    let Some(t) = find_block(w, tgt) else {
                        return Err(VerifyError::BrIfTargetNotFound { span: op.span });
                    };
                    if t.entry_stack.len() != sp {
                        return Err(VerifyError::BrIfStackDepthMismatch { span: op.span });
                    }
                    for (i, item) in stack.iter().enumerate().take(sp) {
                        if *t.entry_stack.get(i).expect("verified length matches sp") != *item {
                            return Err(VerifyError::BrIfStackContentMismatch { span: op.span });
                        }
                    }
                }
                terminated = true;
            }
            OpKind::Ret => {
                if sp != w.sig.out_len as usize {
                    return Err(VerifyError::RetStackDepthMismatch { span: op.span });
                }
                for (i, item) in stack.iter().enumerate().take(sp) {
                    if *item != w.sig.outputs[i] {
                        return Err(VerifyError::RetOutputTypeMismatch { span: op.span });
                    }
                }
                terminated = true;
            }
        }
    }
    if !terminated {
        return Err(VerifyError::NotTerminated { span: Span::UNKNOWN });
    }
    Ok(())
}

fn push(stack: &mut [TypeId; 64], sp: &mut usize, ty: TypeId, span: Span) -> Result<(), VerifyError> {
    if *sp >= stack.len() {
        return Err(VerifyError::PushFullStack { span });
    }
    stack[*sp] = ty;
    *sp += 1;
    Ok(())
}

fn pop(stack: &mut [TypeId; 64], sp: &mut usize, span: Span) -> Result<TypeId, VerifyError> {
    if *sp == 0 {
        return Err(VerifyError::PopEmptyStack { span });
    }
    *sp -= 1;
    Ok(stack[*sp])
}

pub fn write_module(out: &mut impl Output, m: &Module) {
    out.write(b"module ");
    out.write(m.name.as_bytes());
    out.write(b"\n");
    for w in m.words.iter() {
        write_word(out, w);
    }
}

pub fn write_word(out: &mut impl Output, w: &Word) {
    out.write(b"word ");
    out.write(w.name.as_bytes());
    out.write(b" ");
    write_sig(out, w, &w.sig);
    out.write(b"\n");
    for b in w.blocks.iter() {
        out.write(b"  block b");
        write_u32(out, b.id.0 as u32);
        out.write(b" (");
        write_stack(out, w, &b.entry_stack);
        out.write(b")\n");
        for op in b.ops.iter() {
            out.write(b"    ");
            write_op(out, w, op);
            out.write(b"\n");
        }
    }
}

fn type_atom(w: &Word, id: TypeId) -> &Atom {
    w.types.get(id.0 as usize).unwrap_or(&AT_EMPTY)
}

fn write_sig(out: &mut impl Output, w: &Word, sig: &Sig) {
    out.write(b"( ");
    for i in 0..(sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type_atom(out, type_atom(w, sig.inputs[i]), 4);
    }
    out.write(b" --");
    if sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type_atom(out, type_atom(w, sig.outputs[i]), 4);
    }
    out.write(b" )");
}

fn write_stack(out: &mut impl Output, w: &Word, stack: &FixedVec<TypeId, 32>) {
    let mut first = true;
    for a in stack.iter() {
        if !first {
            out.write(b" ");
        }
        first = false;
        write_type_atom(out, type_atom(w, *a), 4);
    }
}

fn write_type_atom(out: &mut impl Output, atom: &Atom, depth: u8) {
    if depth == 0 {
        out.write(atom.as_bytes());
        return;
    }
    let bytes = atom.as_bytes();
    if bytes.starts_with(b"Chan(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Chan(".len()..bytes.len() - 1];
        out.write(b"|");
        if let Some(a) = Atom::new(inner) {
            write_type_atom(out, &a, depth - 1);
        } else {
            out.write(inner);
        }
        out.write(b"|");
        return;
    }
    if bytes.starts_with(b"Array(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Array(".len()..bytes.len() - 1];
        let mut depth_paren = 0u32;
        for (i, &c) in inner.iter().enumerate() {
            match c {
                b'(' => depth_paren = depth_paren.wrapping_add(1),
                b')' => depth_paren = depth_paren.wrapping_sub(1),
                b',' if depth_paren == 0 => {
                    let elem = &inner[..i];
                    let len = &inner[i + 1..];
                    if let Some(a) = Atom::new(elem) {
                        write_type_atom(out, &a, depth - 1);
                    } else {
                        out.write(elem);
                    }
                    out.write(b"'");
                    out.write(len);
                    return;
                }
                _ => {}
            }
        }
    }
    out.write(bytes);
}

fn write_op(out: &mut impl Output, w: &Word, op: &Op) {
    match op.kind {
        OpKind::ConstI64(v) => {
            out.write(b"const_i64 ");
            write_i64(out, v);
        }
        OpKind::ConstBool(true) => out.write(b"const_bool true"),
        OpKind::ConstBool(false) => out.write(b"const_bool false"),
        OpKind::ConstStr(_) => out.write(b"const_str"),
        OpKind::AddrOf { place, mutable: false, const_addr } => {
            out.write(b"addr_of ");
            out.write(place.as_bytes());
            if let Some(addr) = const_addr {
                out.write(b"=0x");
                write_u64_hex(out, addr);
            }
        }
        OpKind::AddrOf { place, mutable: true, const_addr } => {
            out.write(b"addr_of_mut ");
            out.write(place.as_bytes());
            if let Some(addr) = const_addr {
                out.write(b"=0x");
                write_u64_hex(out, addr);
            }
        }
        OpKind::MmioPlace { place, addr } => {
            out.write(b"mmio_place ");
            out.write(place.as_bytes());
            out.write(b" addr=0x");
            write_u64_hex(out, addr);
        }
        OpKind::ScopedEnter { ty, .. } => {
            out.write(b"scoped_enter ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::TaskSpawn { name, .. } => {
            out.write(b"task_spawn ");
            out.write(name.as_bytes());
        }
        OpKind::PtrAddConst { ty, offset } => {
            out.write(b"ptr_add_const ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            write_u32(out, offset);
        }
        OpKind::PtrAddIndex { ty, scale } => {
            out.write(b"ptr_add_index ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            write_u32(out, scale);
        }
        OpKind::Dup { ty } => {
            out.write(b"dup ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Drop { ty } => {
            out.write(b"drop ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Swap { a, b } => {
            out.write(b"swap ");
            out.write(type_atom(w, a).as_bytes());
            out.write(b" ");
            out.write(type_atom(w, b).as_bytes());
        }
        OpKind::AddI64 => out.write(b"add_i64"),
        OpKind::SubI64 => out.write(b"sub_i64"),
        OpKind::MulI64 => out.write(b"mul_i64"),
        OpKind::AndBool => out.write(b"and_bool"),
        OpKind::OrBool => out.write(b"or_bool"),
        OpKind::NotBool => out.write(b"not_bool"),
        OpKind::Cmp { kind, .. } => match kind {
            CmpKind::Lt => out.write(b"cmp_lt"),
            CmpKind::Le => out.write(b"cmp_le"),
            CmpKind::Gt => out.write(b"cmp_gt"),
            CmpKind::Ge => out.write(b"cmp_ge"),
            CmpKind::Eq => out.write(b"cmp_eq"),
            CmpKind::Ne => out.write(b"cmp_ne"),
        },
        OpKind::LocalSet { slot, .. } => {
            out.write(b"local_set ");
            write_u32(out, slot as u32);
        }
        OpKind::LocalGet { slot, .. } => {
            out.write(b"local_get ");
            write_u32(out, slot as u32);
        }
        OpKind::Cast { to, .. } => {
            out.write(b"cast ");
            out.write(type_atom(w, to).as_bytes());
        }
        OpKind::Bitcast { to, .. } => {
            out.write(b"bitcast ");
            out.write(type_atom(w, to).as_bytes());
        }
        OpKind::Call { name, .. } => {
            out.write(b"call ");
            out.write(name.as_bytes());
        }
        OpKind::Load { ty } => {
            out.write(b"load ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::Store { ty } => {
            out.write(b"store ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::MmioVolLoad { ty, place } => {
            out.write(b"vol_load ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
        }
        OpKind::MmioVolStore { ty, place } => {
            out.write(b"vol_store ");
            out.write(type_atom(w, ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
        }
        OpKind::MmioVolLoadField { reg_ty, field_ty, place, mask, shift } => {
            out.write(b"vol_load_field ");
            out.write(type_atom(w, field_ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
            out.write(b" reg=");
            out.write(type_atom(w, reg_ty).as_bytes());
            out.write(b" mask=0x");
            write_u64_hex(out, mask);
            out.write(b" shift=");
            write_u32(out, shift as u32);
        }
        OpKind::MmioVolStoreField { reg_ty, field_ty, place, mask, shift } => {
            out.write(b"vol_store_field ");
            out.write(type_atom(w, field_ty).as_bytes());
            out.write(b" ");
            out.write(place.as_bytes());
            out.write(b" reg=");
            out.write(type_atom(w, reg_ty).as_bytes());
            out.write(b" mask=0x");
            write_u64_hex(out, mask);
            out.write(b" shift=");
            write_u32(out, shift as u32);
        }
        OpKind::CheckSubtype { ty } => {
            out.write(b"check_subtype ");
            out.write(type_atom(w, ty).as_bytes());
        }
        OpKind::TrapIfFalse { code } => {
            out.write(b"trap_if_false ");
            out.write(match code {
                TrapCode::ContractFail => b"CONTRACT_FAIL",
                TrapCode::SubtypeFail => b"SUBTYPE_FAIL",
                TrapCode::AssertFail => b"ASSERT_FAIL",
                TrapCode::StackOverflow => b"STACK_OVERFLOW",
                TrapCode::Unreachable => b"UNREACHABLE",
            });
        }
        OpKind::Br { target } => {
            out.write(b"br b");
            write_u32(out, target.0 as u32);
        }
        OpKind::BrIf { then_tgt, else_tgt } => {
            out.write(b"br_if b");
            write_u32(out, then_tgt.0 as u32);
            out.write(b" b");
            write_u32(out, else_tgt.0 as u32);
        }
        OpKind::Ret => out.write(b"ret"),
    }
}

fn write_u32(out: &mut impl Output, mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

fn write_i64(out: &mut impl Output, v: i64) {
    if v == 0 {
        out.write(b"0");
        return;
    }
    let mut buf = [0u8; 24];
    let mut n = 0usize;
    let mut x = v;
    if x < 0 {
        out.write(b"-");
        x = -x;
    }
    let mut u = x as u64;
    while u > 0 && n < buf.len() {
        buf[n] = b'0' + (u % 10) as u8;
        n += 1;
        u /= 10;
    }
    buf[..n].reverse();
    out.write(&buf[..n]);
}

fn write_u64_hex(out: &mut impl Output, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            let d = (v & 0xF) as u8;
            buf[n] = match d {
                0..=9 => b'0' + d,
                _ => b'a' + (d - 10),
            };
            n += 1;
            v >>= 4;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}
