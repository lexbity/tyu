//! Context stack — the ambient capability/effect fold for checking.
//!
//! Replaces the ad-hoc `allow_suspend: bool` threading with an explicit
//! context-frame stack (effect-context-model.md §3).  Each scope boundary
//! pushes a frame; the ambient fold is the union of all enclosing frames'
//! grants and forbids.

use crate::typecheck::error::TcError;
use crate::types::TypeAtom;
use frontend::span::Span;
use ir::{CapSet, EffectSet};

// ---------------------------------------------------------------------------
// Context kind
// ---------------------------------------------------------------------------

/// The kind of a context frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextKind {
    WordBody,
    Lock,
    ReadBorrow,
    MutBorrow,
    Isr,
    Handler,
    Bounded,
}

/// Parameter carried by a context frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameParam {
    None,
    Scope(u16),
    Resource(TypeAtom),
}

// ---------------------------------------------------------------------------
// Matrix — the normative context × effect/capability table
// ---------------------------------------------------------------------------

/// One row of the context matrix.
pub struct MatrixRow {
    pub kind: ContextKind,
    pub grants: CapSet,
    pub forbids: EffectSet,
    pub conditional_suspend: bool,
}

/// The context matrix — one row per context kind.
///
/// This is the normative artifact. Rows are exercised by the unit tests
/// below and by the fixture corpus in crates/tooling-tests/tests/corpus/;
/// a per-cell synthesized completeness oracle does not exist yet.
pub const MATRIX: [MatrixRow; 7] = [
    MatrixRow {
        kind: ContextKind::WordBody,
        grants: CapSet::from_bits(CapSet::SUSPENDABLE),
        forbids: EffectSet::empty(),
        conditional_suspend: false,
    },
    MatrixRow {
        kind: ContextKind::Lock,
        grants: CapSet::empty(),
        forbids: EffectSet::from_bits(EffectSet::SUSPEND),
        conditional_suspend: false,
    },
    MatrixRow {
        kind: ContextKind::ReadBorrow,
        grants: CapSet::empty(),
        forbids: EffectSet::empty(),
        conditional_suspend: true,
    },
    MatrixRow {
        kind: ContextKind::MutBorrow,
        grants: CapSet::empty(),
        forbids: EffectSet::from_bits(EffectSet::SUSPEND),
        conditional_suspend: false,
    },
    MatrixRow {
        kind: ContextKind::Isr,
        grants: CapSet::empty(),
        forbids: EffectSet::from_bits(EffectSet::SUSPEND),
        conditional_suspend: false,
    },
    MatrixRow {
        kind: ContextKind::Handler,
        grants: CapSet::from_bits(CapSet::SUSPENDABLE),
        forbids: EffectSet::empty(),
        conditional_suspend: false,
    },
    MatrixRow {
        kind: ContextKind::Bounded,
        grants: CapSet::from_bits(CapSet::BOUNDED_STACK),
        forbids: EffectSet::from_bits(EffectSet::DIVERGE),
        conditional_suspend: false,
    },
];

pub fn row(kind: ContextKind) -> &'static MatrixRow {
    MATRIX.iter().find(|r| r.kind == kind).expect("unknown ContextKind")
}

// ---------------------------------------------------------------------------
// Frame and stack
// ---------------------------------------------------------------------------

const MAX_CONTEXT_DEPTH: u8 = 16;

/// A single context frame on the stack.
#[derive(Clone, Copy, Debug)]
pub struct ContextFrame {
    pub kind: ContextKind,
    pub grants: CapSet,
    pub forbids: EffectSet,
    pub param: FrameParam,
    pub span: Span,
    saved_ambient_grants: CapSet,
    saved_ambient_forbids: EffectSet,
}

impl ContextFrame {
    /// The scope id carried by a ReadBorrow/MutBorrow frame, or None.
    pub fn scope(&self) -> Option<u16> {
        match self.param {
            FrameParam::Scope(s) => Some(s),
            _ => None,
        }
    }

    /// The resource atom carried by a Lock frame, or None.
    pub fn resource(&self) -> Option<TypeAtom> {
        match self.param {
            FrameParam::Resource(r) => Some(r),
            _ => None,
        }
    }
}

/// The context stack — a fixed-size array with running ambient fold.
///
/// Push/pop are O(1).  Pop restores the ambient fold from a snapshot taken
/// at push time (set-union is not invertible, so we snapshot instead of
/// recomputing the fold).
pub struct ContextStack {
    frames: [ContextFrame; MAX_CONTEXT_DEPTH as usize],
    depth: u8,
    pub ambient_grants: CapSet,
    pub ambient_forbids: EffectSet,
}

impl ContextStack {
    pub const fn new() -> Self {
        const EMPTY: ContextFrame = ContextFrame {
            kind: ContextKind::WordBody,
            grants: CapSet::empty(),
            forbids: EffectSet::empty(),
            param: FrameParam::None,
            span: Span::new(0, 0),
            saved_ambient_grants: CapSet::empty(),
            saved_ambient_forbids: EffectSet::empty(),
        };
        ContextStack {
            frames: [EMPTY; MAX_CONTEXT_DEPTH as usize],
            depth: 0,
            ambient_grants: CapSet::empty(),
            ambient_forbids: EffectSet::empty(),
        }
    }

    /// Reset the stack to empty (called at each word's compile entry).
    /// Assert depth == 0 on finish (ICE otherwise).
    pub fn reset(&mut self) {
        self.depth = 0;
        self.ambient_grants = CapSet::empty();
        self.ambient_forbids = EffectSet::empty();
    }

    /// Push a new context frame.
    ///
    /// The frame's grants/forbids are computed from MATRIX[kind].
    /// The current ambient is snapshotted into the frame (for O(1) pop),
    /// then the frame's grants/forbids are unioned into the ambient fold.
    pub fn push(
        &mut self,
        kind: ContextKind,
        param: FrameParam,
        span: Span,
    ) -> Result<(), TcError> {
        if self.depth as usize >= self.frames.len() {
            return Err(TcError::ScopeDepthExceeded { span });
        }
        let r = row(kind);
        let idx = self.depth as usize;
        self.frames[idx] = ContextFrame {
            kind,
            grants: r.grants,
            forbids: r.forbids,
            param,
            span,
            saved_ambient_grants: self.ambient_grants,
            saved_ambient_forbids: self.ambient_forbids,
        };
        self.ambient_grants = self.ambient_grants.union(r.grants);
        self.ambient_forbids = self.ambient_forbids.union(r.forbids);
        self.depth += 1;
        Ok(())
    }

    /// Pop the top frame, restoring the ambient fold to its pre-push state.
    ///
    /// Panics (ICE) if the stack is already empty.
    pub fn pop(&mut self) {
        assert!(
            self.depth > 0,
            "ContextStack::pop() called on empty stack"
        );
        self.depth -= 1;
        let idx = self.depth as usize;
        self.ambient_grants = self.frames[idx].saved_ambient_grants;
        self.ambient_forbids = self.frames[idx].saved_ambient_forbids;
    }

    /// Returns the innermost Lock frame, if any.
    pub fn lock_frame(&self) -> Option<&ContextFrame> {
        self.frames()
            .iter()
            .rev()
            .find(|f| f.kind == ContextKind::Lock)
    }

    /// Iterate over all frames of a given kind, innermost first.
    pub fn frames_of(&self, kind: ContextKind) -> impl Iterator<Item = &ContextFrame> {
        self.frames()
            .iter()
            .rev()
            .filter(move |f| f.kind == kind)
    }

    /// Find the span of the innermost frame that forbids the given effect.
    pub fn forbidding_span(&self, effect: EffectSet) -> Option<Span> {
        self.frames()
            .iter()
            .rev()
            .find(|f| f.forbids.intersect(effect).bits() != 0)
            .map(|f| f.span)
    }

    /// Current frame depth (for assertions and diagnostics).
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// Raw slice of active frames (for iteration).
    pub fn frames(&self) -> &[ContextFrame] {
        &self.frames[..self.depth as usize]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn matrix_rows_are_distinct() {
        let mut kinds: Vec<ContextKind> = MATRIX.iter().map(|r| r.kind).collect();
        kinds.sort_by(|a, b| (*a as u8).cmp(&(*b as u8)));
        kinds.dedup();
        assert_eq!(kinds.len(), MATRIX.len(), "duplicate ContextKind in MATRIX");
    }

    #[test]
    fn new_stack_is_empty() {
        let ctx = ContextStack::new();
        assert_eq!(ctx.depth(), 0);
        assert!(ctx.ambient_grants.is_empty());
        assert!(ctx.ambient_forbids.is_empty());
    }

    #[test]
    fn reset_clears_stack() {
        let mut ctx = ContextStack::new();
        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(0, 0)).unwrap();
        assert_eq!(ctx.depth(), 1);
        ctx.reset();
        assert_eq!(ctx.depth(), 0);
        assert!(ctx.ambient_grants.is_empty());
        assert!(ctx.ambient_forbids.is_empty());
    }

    #[test]
    fn push_then_pop_restores_ambient() {
        let mut ctx = ContextStack::new();
        let grants_before = ctx.ambient_grants;
        let forbids_before = ctx.ambient_forbids;
        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(0, 0)).unwrap();
        assert_eq!(ctx.depth(), 1);
        assert!(!ctx.ambient_forbids.is_empty());
        ctx.pop();
        assert_eq!(ctx.depth(), 0);
        assert_eq!(ctx.ambient_grants, grants_before);
        assert_eq!(ctx.ambient_forbids, forbids_before);
    }

    #[test]
    fn nested_push_pop_restores_correctly() {
        let mut ctx = ContextStack::new();
        let g0 = ctx.ambient_grants;
        let f0 = ctx.ambient_forbids;

        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(0, 0)).unwrap();
        let g1 = ctx.ambient_grants;
        let f1 = ctx.ambient_forbids;
        assert!(!f1.is_empty()); // Lock forbids SUSPEND

        ctx.push(ContextKind::Handler, FrameParam::None, Span::new(0, 0)).unwrap();
        let g2 = ctx.ambient_grants;
        assert!(!g2.is_empty()); // Handler grants SUSPENDABLE

        ctx.pop();
        assert_eq!(ctx.ambient_grants, g1);
        assert_eq!(ctx.ambient_forbids, f1);

        ctx.pop();
        assert_eq!(ctx.ambient_grants, g0);
        assert_eq!(ctx.ambient_forbids, f0);
    }

    #[test]
    fn lock_frame_returns_inner_lock() {
        let mut ctx = ContextStack::new();
        assert!(ctx.lock_frame().is_none());
        ctx.push(ContextKind::Isr, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.lock_frame().is_none());
        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(1, 2)).unwrap();
        let lf = ctx.lock_frame().unwrap();
        assert_eq!(lf.kind, ContextKind::Lock);
        assert_eq!(lf.span, Span::new(1, 2));
    }

    #[test]
    fn forbidding_span_returns_correct_frame() {
        let mut ctx = ContextStack::new();
        ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0)).unwrap();
        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(5, 10)).unwrap();
        let span = ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND));
        assert_eq!(span, Some(Span::new(5, 10)));
    }

    #[test]
    fn depth_overflow_returns_error() {
        let mut ctx = ContextStack::new();
        for _ in 0..MAX_CONTEXT_DEPTH {
            ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0)).unwrap();
        }
        assert_eq!(ctx.depth(), MAX_CONTEXT_DEPTH);
        let result = ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0));
        assert!(result.is_err());
    }

    #[test]
    #[should_panic(expected = "empty stack")]
    fn pop_empty_stack_panics() {
        let mut ctx = ContextStack::new();
        ctx.pop();
    }

    #[test]
    fn read_borrow_does_not_forbid_suspend() {
        let mut ctx = ContextStack::new();
        ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND)).is_none());
        ctx.push(ContextKind::ReadBorrow, FrameParam::Scope(1), Span::new(0, 0)).unwrap();
        assert!(ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND)).is_none());
    }

    #[test]
    fn ambient_grants_wordbody_grants_suspendable() {
        let mut ctx = ContextStack::new();
        assert!(!ctx.ambient_grants.contains(CapSet::SUSPENDABLE));
        ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.ambient_grants.contains(CapSet::SUSPENDABLE));
    }

    #[test]
    fn ambient_grants_handler_grants_suspendable() {
        let mut ctx = ContextStack::new();
        assert!(!ctx.ambient_grants.contains(CapSet::SUSPENDABLE));
        ctx.push(ContextKind::Handler, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.ambient_grants.contains(CapSet::SUSPENDABLE));
    }

    #[test]
    fn lock_forbids_suspend() {
        let mut ctx = ContextStack::new();
        ctx.push(ContextKind::WordBody, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND)).is_none());
        ctx.push(ContextKind::Lock, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND)).is_some());
    }

    #[test]
    fn isr_forbids_suspend() {
        let mut ctx = ContextStack::new();
        ctx.push(ContextKind::Isr, FrameParam::None, Span::new(0, 0)).unwrap();
        assert!(ctx.forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND)).is_some());
    }
}
