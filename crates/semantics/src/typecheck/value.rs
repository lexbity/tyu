use crate::types::TypeAtom;
use crate::typecheck::mmio::{MmioResolved, MmioResolvedReg};
use frontend::span::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Plain(TypeAtom),
    Scoped { ty: TypeAtom, scope: u16 },
    Resource(TypeAtom),
    Quot(Span),
    MmioPlace(MmioResolved),
    Ptr { ty: TypeAtom, mutable: bool },
    MmioPtr {
        reg: MmioResolvedReg,
        mutable: bool,
    },
}
