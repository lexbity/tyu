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
///
/// `ambient` is the ACTUAL persistent stack, top-first as type names: the
/// session passes its real stack so the synthetic phrase word's declared
/// inputs are what the stack actually holds — not what the first consuming
/// word's signature wishes for (the W1 reconciliation: this boundary was the
/// hole in §Q7's reachability argument — an ambient-mismatched phrase would
/// have compiled and run loose; now the checker verifies the phrase against
/// the real stack, keeping L2's engine-only checks genuinely unreachable).
/// When a phrase consumes below the stack's depth the declared `want` type is
/// the fallback (the checker then rejects the underflow, as before).
pub fn phrase_effect(
    src: &[u8],
    ambient: &[Vec<u8>],
    mut lookup: impl FnMut(&[u8]) -> Option<EffectSig>,
) -> Option<EffectSig> {
    let mut env = Env {
        stack: Vec::new(),
        ambient: Vec::new(),
        ambient_actual: ambient,
        ambient_idx: 0,
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
    /// The session's actual stack types, top-first (see `phrase_effect`).
    ambient_actual: &'a [Vec<u8>],
    /// How many ambient pops have happened so far (indexes `ambient_actual`).
    ambient_idx: usize,
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
                TokenKind::String => {
                    // A string literal pushes a `str` (the checker's
                    // `compile_string` types it `TypeAtom::STR`); this is
                    // what makes `"x\n" testio.write-str` / `platform.io.log`
                    // phrases inferable.
                    self.stack.push(Slot::Ty(b"str".to_vec()));
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
                    // A quotation body cannot consume the ambient stack — its
                    // ambient slice is empty (an ambient pop inside a quote is
                    // the `want` fallback, which the checker rejects).
                    let body_sig = phrase_effect(body, &[], &mut *self.lookup)?;
                    self.stack.push(Slot::Quot(body_sig));
                    self.run_tokens(rest)?;
                    return Some(());
                }
                TokenKind::PunctGe => self.call_word(b">=")?,
                TokenKind::PunctLe => self.call_word(b"<=")?,
                TokenKind::PunctEqEq => self.call_word(b"==")?,
                TokenKind::PunctNe => self.call_word(b"!=")?,
                TokenKind::PunctSlash => self.call_word(b"/")?,
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
                        // `cond [a] [b] if` — the source order pushes cond,
                        // then [a], then [b], so the pops run b, a, cond.
                        // (The old arm popped the cond first, hit a
                        // quotation where it wanted the bool, and rejected
                        // every natural phrase-if as E3200.) The two
                        // branches' effects must agree — the checker
                        // re-checks, and rejects depth mismatches (E3246).
                        let b = self.pop_quot()?;
                        let a = self.pop_quot()?;
                        let cond = self.pop(None)?;
                        if slot_ty(&cond) != b"bool" {
                            return None;
                        }
                        if a != b {
                            return None;
                        }
                        self.push_effect(a);
                    }
                    b"while" => {
                        // `[cond] [body] while` — the loop values are the
                        // condition's inputs: consumed from the stack below
                        // the quotations, the condition's outputs (minus its
                        // trailing bool) persist across checks, and the body
                        // transforms the loop values in place each
                        // iteration. (The old arm pushed the body's outputs
                        // without consuming anything, so every value-
                        // carrying loop synthesized one value too many and
                        // the checker rejected the phrase E3220.)
                        let body = self.pop_quot()?;
                        let cond = self.pop_quot()?;
                        for ty in cond.inputs.iter().rev() {
                            self.pop(Some(ty))?;
                        }
                        self.push_effect(cond.clone());
                        // The loop consumes the condition's trailing bool.
                        let top = self.stack.pop()?;
                        if slot_ty(&top) != b"bool" {
                            return None;
                        }
                        self.apply_sig(body);
                    }
                    b"loop" => {
                        // `[ body ] loop` — the body consumes its inputs from
                        // the stack and replaces them with its outputs,
                        // forever (the same one-value leak as the old
                        // `while` arm, fixed the same way).
                        let body = self.pop_quot()?;
                        self.apply_sig(body);
                    }
                    _ => {
                        // A user local?
                        if let Some((_, ty)) = self.locals.iter().find(|(n, _)| n == text) {
                            self.stack.push(Slot::Ty(ty.clone()));
                            continue;
                        }
                        // A word with a declared effect (operator words like
                        // `+` `-` `*` `>` `<` are lexed as Idents).
                        //
                        // Dotted names (`testio.write-byte`,
                        // `platform.mem.region-create`) lex as
                        // `Ident (PunctDot Ident)*` — reassemble the full
                        // call name before the lookup, or the interpreter
                        // builtins and imported module words are
                        // unreachable from phrases. (§7.2: this
                        // accommodation lives in the session layer, never
                        // the production parser.)
                        let mut name: Vec<u8> = text.to_vec();
                        loop {
                            let mut look = lex;
                            if look.next().kind != TokenKind::PunctDot {
                                break;
                            }
                            let id = look.next();
                            if id.kind != TokenKind::Ident {
                                return None;
                            }
                            name.push(b'.');
                            name.extend_from_slice(&src[id.span.start..id.span.end]);
                            lex = look;
                        }
                        self.call_word(&name)?;
                    }
                },
                _ => return None,
            }
        }
    }

    /// Resolve and apply a word (an operator name or an identifier).
    fn call_word(&mut self, name: &[u8]) -> Option<()> {
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
        // The ACTUAL stack type wins (that is what will be on the runtime
        // stack); the consuming word's declared type is the fallback when the
        // phrase pops deeper than the stack (the checker then rejects it).
        let ty = match self.ambient_actual.get(self.ambient_idx) {
            Some(actual) => actual.clone(),
            None => want.unwrap_or(b"i64").to_vec(),
        };
        self.ambient_idx += 1;
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
