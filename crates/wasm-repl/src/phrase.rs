//! Phrase effect computation (PLAN-WASM-1 §7.2): the trailing-phrase tokens
//! are wrapped into a synthetic `repl-phrase` word, and that word needs an
//! exact signature. The typechecker *verifies* a declared signature (it does
//! not expose the computed one), so this module computes the phrase's net
//! `(inputs -- outputs)` by a forward pass over the tokens using each word's
//! declared signature. The checker re-validates the produced signature — this
//! pass is the session-builder accommodation §7.2 permits, never a second
//! semantics.
//!
//! Supported phrase constructs: literals, word calls (including the
//! operator-words `+ - * > < >= <= == != and or not`), `=>` bindings and
//! their local references, `as TYPE` casts, and `[ ... ]` quotations consumed
//! by `if`/`while`/`loop` (the quotation bodies' effects are computed
//! recursively). Anything else (an unknown word, a quotation escaping the
//! phrase net) is `None` — a clean "phrase not inferable" diagnostic, never a
//! guess.

use alloc::vec::Vec;
use frontend::lex::Lexer;
use frontend::token::TokenKind;

/// The net effect of a phrase (or word): the stack types it pops (bottom-first
/// input slots) and pushes (top-first outputs).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectSig {
    pub inputs: Vec<Vec<u8>>,
    pub outputs: Vec<Vec<u8>>,
}

/// The kind of a stack slot during computation.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Slot {
    /// A named type.
    Ty(Vec<u8>),
    /// A quotation whose *body* effect is the payload.
    Quot(EffectSig),
}

/// Compute the net effect of a phrase; `lookup` resolves word names to their
/// declared effects (builtins + interpreter builtins + session words).
pub fn phrase_effect(
    src: &[u8],
    mut lookup: impl FnMut(&[u8]) -> Option<EffectSig>,
) -> Option<EffectSig> {
    let mut env = Env {
        stack: Vec::new(),
        ambient: Vec::new(),
        locals: Vec::new(),
        lookup: &mut lookup,
    };
    env.run_tokens(src)?;
    // Ambient pops happen top-first; the declared inputs are bottom-first.
    env.ambient.reverse();
    let inputs = core::mem::take(&mut env.ambient);
    let outputs = env.stack.iter().map(|s| slot_ty(s).to_vec()).collect();
    Some(EffectSig { inputs, outputs })
}

struct Env<'a> {
    stack: Vec<Slot>,
    /// Input slots consumed from the session stack, in pop order.
    ambient: Vec<Vec<u8>>,
    locals: Vec<(Vec<u8>, Vec<u8>)>,
    lookup: &'a mut dyn FnMut(&[u8]) -> Option<EffectSig>,
}

impl<'a> Env<'a> {
    fn run_tokens(&mut self, src: &[u8]) -> Option<()> {
        let mut lex = Lexer::new(src);
        loop {
            let tok = lex.next();
            if tok.kind == TokenKind::Eof {
                return Some(());
            }
            let text = &src[tok.span.start..tok.span.end];
            match tok.kind {
                TokenKind::Number => {
                    let _: i64 = core::str::from_utf8(text).ok()?.parse().ok()?;
                    self.stack.push(Slot::Ty(b"i64".to_vec()));
                }
                TokenKind::PunctArrowBind => {
                    // `=> name` pops one value and binds a local.
                    let v = self.pop(None)?;
                    let name_tok = lex.next();
                    if name_tok.kind != TokenKind::Ident {
                        return None;
                    }
                    let name = src[name_tok.span.start..name_tok.span.end].to_vec();
                    let ty = slot_ty(&v).to_vec();
                    self.locals.push((name, ty));
                }
                TokenKind::PunctLBracket => {
                    let (body, rest) = split_quote(src, tok.span.end)?;
                    let body_sig = phrase_effect(body, &mut *self.lookup)?;
                    self.stack.push(Slot::Quot(body_sig));
                    self.run_tokens(rest)?;
                    return Some(());
                }
                TokenKind::PunctGe => self.call_word(b">=", &mut lex, src)?,
                TokenKind::PunctLe => self.call_word(b"<=", &mut lex, src)?,
                TokenKind::PunctEqEq => self.call_word(b"==", &mut lex, src)?,
                TokenKind::PunctNe => self.call_word(b"!=", &mut lex, src)?,
                TokenKind::PunctSlash => self.call_word(b"/", &mut lex, src)?,
                TokenKind::Ident => match text {
                    b"true" | b"false" => self.stack.push(Slot::Ty(b"bool".to_vec())),
                    b"as" => {
                        // `as TYPE` retags one value.
                        let _ = self.pop(None)?;
                        let ty_tok = lex.next();
                        if ty_tok.kind != TokenKind::Ident {
                            return None;
                        }
                        let ty = src[ty_tok.span.start..ty_tok.span.end].to_vec();
                        self.stack.push(Slot::Ty(ty));
                    }
                    b"if" => {
                        // `cond [a] [b] if` — pop cond + 2 quotations; the two
                        // body effects must agree (the checker re-checks).
                        let _ = self.pop(None)?; // cond
                        let b = self.pop_quot()?;
                        let a = self.pop_quot()?;
                        if a != b {
                            return None;
                        }
                        self.push_effect(a);
                    }
                    b"while" => {
                        // `[cond] [body] while` — body effect remains.
                        let body = self.pop_quot()?;
                        let _cond = self.pop_quot()?;
                        self.push_effect(body);
                    }
                    b"loop" => {
                        let body = self.pop_quot()?;
                        self.push_effect(body);
                    }
                    _ => {
                        // A user local?
                        if let Some((_, ty)) = self.locals.iter().find(|(n, _)| n == text) {
                            self.stack.push(Slot::Ty(ty.clone()));
                            continue;
                        }
                        // A word with a declared effect (operator words like
                        // `+` `-` `*` `>` `<` are lexed as Idents).
                        self.call_word(text, &mut lex, src)?;
                    }
                },
                _ => return None,
            }
        }
    }

    /// Resolve and apply a word (an operator name or an identifier).
    fn call_word(&mut self, name: &[u8], _lex: &mut Lexer<'_>, _src: &[u8]) -> Option<()> {
        let sig = (self.lookup)(name)?;
        self.apply_sig(sig);
        Some(())
    }

    fn apply_sig(&mut self, sig: EffectSig) {
        let n = sig.inputs.len();
        for k in (0..n).rev() {
            let _ = self.pop(Some(&sig.inputs[k]));
        }
        for t in sig.outputs {
            self.stack.push(Slot::Ty(t));
        }
    }

    fn push_effect(&mut self, sig: EffectSig) {
        for t in sig.outputs {
            self.stack.push(Slot::Ty(t));
        }
    }

    fn pop(&mut self, want: Option<&[u8]>) -> Option<Slot> {
        if let Some(s) = self.stack.pop() {
            return Some(s);
        }
        // An ambient pop: a value the phrase consumes from the session stack.
        let ty = want.unwrap_or(b"i64").to_vec();
        self.ambient.push(ty.clone());
        Some(Slot::Ty(ty))
    }

    fn pop_quot(&mut self) -> Option<EffectSig> {
        match self.pop(None)? {
            Slot::Quot(sig) => Some(sig),
            _ => None,
        }
    }
}

fn slot_ty(s: &Slot) -> &[u8] {
    match s {
        Slot::Ty(ty) => ty,
        // A quotation escaping the phrase net is unsupported in v1.
        Slot::Quot(_) => b"quot",
    }
}

/// Split a `[ ... ]` quotation: returns the body slice and the remainder of
/// the source after the matching `]`. Nested brackets are handled.
fn split_quote(src: &[u8], start: usize) -> Option<(&[u8], &[u8])> {
    let mut depth = 1u32;
    let mut i = start;
    while i < src.len() {
        match src[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let body = &src[start..i];
                    let rest = &src[i + 1..];
                    return Some((body, rest));
                }
            }
            b'#' => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
