use crate::typecheck::error::Output;
use crate::typecheck::util::write_stack;
use crate::typecheck::value::Value;
use frontend::span::Span;

/// Callback trait invoked during IR generation to observe the typechecking
/// process. The default implementation (`NullObserver`) does nothing,
/// allowing the compiler to skip the diagnostic overhead when not needed.
///
/// Stackcheck uses this trait to produce its `--emit=tc` diagnostic output
/// without duplicating the IR generator's typechecking logic.
pub trait TypecheckObserver {
    /// Called before a word body is compiled.
    fn on_word_begin(&mut self, _src: &[u8], _name_span: Span, _sig: &[u8]) {}

    /// Called after each token is processed, with the current stack state.
    /// `token_text` is the raw source slice of the token.
    fn on_token(&mut self, _stack: &[Value; 256], _sp: usize, _token_text: &[u8]) {}

    /// Called after a word body has been fully compiled.
    fn on_word_end(&mut self) {}
}

/// Observer that does nothing — used for normal IR generation.
pub struct NullObserver;
impl TypecheckObserver for NullObserver {}

/// Observer that prints stack state after each token, matching
/// the old `--emit=tc` output format.
pub struct StackcheckObserver<'a> {
    pub out: &'a mut dyn Output,
}

impl<'a> TypecheckObserver for StackcheckObserver<'a> {
    fn on_word_begin(&mut self, src: &[u8], name_span: Span, sig: &[u8]) {
        // Printed by the caller (emit_stackcheck) before calling build_ir_word.
        let _ = (src, name_span, sig);
    }

    fn on_token(&mut self, stack: &[Value; 256], sp: usize, token_text: &[u8]) {
        self.out.write(b"  ");
        self.out.write(token_text);
        self.out.write(b" | stack: ");
        write_stack(self.out, stack, sp);
        self.out.write(b"\n");
    }

    fn on_word_end(&mut self) {}
}
