use crate::typecheck::mmio::{MmioResolved, MmioResolvedReg};
use crate::types::TypeAtom;
use frontend::span::Span;

/// A unique identifier for a borrowed place within a single word compilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaceId(pub u16);

/// Sentinel meaning "no provenance" — used for unresolved or synthetic borrows.
pub const PLACE_NONE: PlaceId = PlaceId(u16::MAX);

/// Base for synthetic parameter PlaceIds (D-6).
/// Parameters get `PlaceId(PARAM_BASE + i)` so they never collide with
/// normal borrows minted inside the word body (0..63).
pub const PARAM_BASE: u16 = 0xFF00;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Plain(TypeAtom),
    Scoped {
        ty: TypeAtom,
        scope: u16,
    },
    Resource(TypeAtom),
    Quot(Span),
    MmioPlace(MmioResolved),
    Ptr {
        ty: TypeAtom,
        mutable: bool,
        place: PlaceId,
    },
    MmioPtr {
        reg: MmioResolvedReg,
        mutable: bool,
    },
}

impl Value {
    /// Returns the canonical 8-bit type atom for this value.
    /// Complex pointer types collapse to `ptr`/`ptr_mut`.
    pub fn to_type_atom(self) -> TypeAtom {
        match self {
            Value::Plain(t) => t,
            Value::Scoped { ty, .. } => ty,
            Value::Resource(_) => TypeAtom::RESOURCE,
            Value::Quot(_) => TypeAtom::QUOT,
            Value::MmioPlace(_) => TypeAtom::MMIO,
            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
        }
    }

    /// Returns the inner type for compound variants (Scoped, Ptr, Resource),
    /// or the plain type for `Plain`.
    pub fn inner_type(self) -> TypeAtom {
        match self {
            Value::Plain(t) => t,
            Value::Scoped { ty, .. } => ty,
            Value::Resource(t) => t,
            Value::Ptr { ty, .. } => ty,
            Value::MmioPtr { .. } => TypeAtom::MMIO,
            Value::Quot(_) => TypeAtom::QUOT,
            Value::MmioPlace(_) => TypeAtom::MMIO,
        }
    }
}
