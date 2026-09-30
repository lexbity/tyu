// Miri tests for ArenaAllocator.
// Run: cargo +nightly miri test -p semantics --test arena
//
// ArenaAllocator is heap-backed (Box<[MaybeUninit<Word>]>, P3 root-cause fix):
// the words arena never lives on the stack. The tests run on the
// workspace-configured test-thread stack (`.cargo/config.toml` sets
// `RUST_MIN_STACK`).

use frontend::fixed::FixedVec;
use frontend::span::Span;
use ir::{CapSet, EffectSet, StackBound};
use semantics::typecheck::irgen::arena::ArenaAllocator;

fn make_word(name: &[u8]) -> ir::Word {
    ir::Word {
        name: ir::Atom::new(name).unwrap(),
        sig: ir::Sig::empty(),
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
        entry: ir::BlockId(0),
        types: FixedVec::new(),
        type_sizes: FixedVec::new(),
        type_classes: FixedVec::new(),
        apertures: FixedVec::new(),
        subtype_bases: FixedVec::new(),
        blocks: FixedVec::new(),
    }
}

#[test]
fn alloc_one_word() {
    let mut arena = ArenaAllocator::new();
    let w = arena.alloc(make_word(b"foo"), Span::UNKNOWN).unwrap();
    assert_eq!(w.name.as_bytes(), b"foo");
}

#[test]
fn alloc_multiple_words() {
    let mut arena = ArenaAllocator::new();
    for i in 0..5 {
        let name = [b'a' + i as u8];
        let w = arena.alloc(make_word(&name), Span::UNKNOWN).unwrap();
        assert_eq!(w.name.as_bytes(), &name);
    }
}

#[test]
fn alloc_up_to_capacity() {
    let mut arena = ArenaAllocator::new();
    for i in 0..17 {
        let name = [b'0' + (i % 10) as u8];
        assert!(arena.alloc(make_word(&name), Span::UNKNOWN).is_ok());
    }
}

#[test]
fn alloc_overflow_returns_err() {
    let mut arena = ArenaAllocator::new();
    for _ in 0..17 {
        let _ = arena.alloc(make_word(b"x"), Span::UNKNOWN);
    }
    let err = arena.alloc(make_word(b"y"), Span::UNKNOWN);
    assert!(err.is_err());
}

#[test]
fn empty_arena_drop_is_noop() {
    let arena = ArenaAllocator::new();
    drop(arena);
}

#[test]
fn alloc_fill_then_drop() {
    let mut arena = ArenaAllocator::new();
    for i in 0..17 {
        let name = [b'0' + (i % 10) as u8];
        let w = arena.alloc(make_word(&name), Span::UNKNOWN).unwrap();
        assert_eq!(w.name.as_bytes(), &name);
    }
    drop(arena);
}
