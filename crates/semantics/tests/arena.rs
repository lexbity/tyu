// Miri tests for ArenaAllocator.
// Run: cargo +nightly miri test -p semantics --test arena
//
// ArenaAllocator embeds 17 ir::Word slots à ~114 KB = ~1.9 MB on the stack,
// which exceeds the 2 MB test-thread default.  We wrap each test body in
// spawn_stack so it runs on an 8 MB thread.

use std::thread;

use semantics::typecheck::irgen::arena::ArenaAllocator;
use frontend::fixed::FixedVec;
use frontend::span::Span;

fn spawn_stack(f: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn make_word(name: &[u8]) -> ir::Word {
    ir::Word {
        name: ir::Atom::new(name).unwrap(),
        sig: ir::Sig::empty(),
        entry: ir::BlockId(0),
        types: FixedVec::new(),
        type_sizes: FixedVec::new(),
        blocks: FixedVec::new(),
    }
}

#[test]
fn alloc_one_word() {
    spawn_stack(|| {
        let mut arena = ArenaAllocator::new();
        let w = arena.alloc(make_word(b"foo"), Span::UNKNOWN).unwrap();
        assert_eq!(w.name.as_bytes(), b"foo");
    });
}

#[test]
fn alloc_multiple_words() {
    spawn_stack(|| {
        let mut arena = ArenaAllocator::new();
        for i in 0..5 {
            let name = [b'a' + i as u8];
            let w = arena.alloc(make_word(&name), Span::UNKNOWN).unwrap();
            assert_eq!(w.name.as_bytes(), &name);
        }
    });
}

#[test]
fn alloc_up_to_capacity() {
    spawn_stack(|| {
        let mut arena = ArenaAllocator::new();
        for i in 0..17 {
            let name = [b'0' + (i % 10) as u8];
            assert!(arena.alloc(make_word(&name), Span::UNKNOWN).is_ok());
        }
    });
}

#[test]
fn alloc_overflow_returns_err() {
    spawn_stack(|| {
        let mut arena = ArenaAllocator::new();
        for _ in 0..17 {
            let _ = arena.alloc(make_word(b"x"), Span::UNKNOWN);
        }
        let err = arena.alloc(make_word(b"y"), Span::UNKNOWN);
        assert!(err.is_err());
    });
}

#[test]
fn empty_arena_drop_is_noop() {
    spawn_stack(|| {
        let arena = ArenaAllocator::new();
        drop(arena);
    });
}

#[test]
fn alloc_fill_then_drop() {
    spawn_stack(|| {
        let mut arena = ArenaAllocator::new();
        for i in 0..17 {
            let name = [b'0' + (i % 10) as u8];
            let w = arena.alloc(make_word(&name), Span::UNKNOWN).unwrap();
            assert_eq!(w.name.as_bytes(), &name);
        }
    });
}
