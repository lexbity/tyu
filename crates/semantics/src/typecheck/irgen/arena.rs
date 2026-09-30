use super::*;
extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;

pub(super) const QUOTE_WORD_CAP: usize = 16;
// One main word + max quote words per build_ir_word call.
const WORD_ARENA_CAP: usize = QUOTE_WORD_CAP + 1;

pub struct ArenaAllocator {
    len: usize,
    // Heap-backed arena. Never build `[MaybeUninit<T>; N]` on the stack
    // (`Box::new([...])` materializes the array on the stack first): a `Word`
    // is ~100 KB, so a 17-word arena array would need ~1.7 MiB of stack per
    // `build_ir` call — beyond any 2 MiB test-thread stack (P3 fix).
    words: Box<[MaybeUninit<lir::Word>]>,
}

impl ArenaAllocator {
    pub fn new() -> Self {
        let words: Vec<MaybeUninit<lir::Word>> =
            (0..WORD_ARENA_CAP).map(|_| MaybeUninit::uninit()).collect();
        Self {
            len: 0,
            words: words.into_boxed_slice(),
        }
    }

    pub fn alloc(&mut self, word: lir::Word, span: Span) -> Result<&lir::Word, TcError> {
        if self.len >= WORD_ARENA_CAP {
            return Err(TcError::ArenaFull { span });
        }
        let slot = self.words[self.len].write(word);
        self.len += 1;
        Ok(&*slot)
    }

    /// Reset the arena, dropping all allocated words and resetting the length to zero.
    /// This allows the same arena to be reused across multiple `build_ir_word` calls.
    pub fn reset(&mut self) {
        for i in 0..self.len {
            unsafe { self.words[i].assume_init_drop() };
        }
        self.len = 0;
    }
}

impl Default for ArenaAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ArenaAllocator {
    fn drop(&mut self) {
        for i in 0..self.len {
            unsafe { self.words[i].assume_init_drop() };
        }
    }
}
