use crate::typecheck::error::TcError;
use crate::types::TypeAtom;
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::span::Span;
use frontend::token::TokenKind;

/// A single step in a place path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Struct field access: `.field_name`
    Field(TypeAtom),
    /// Static array index: `'N` or `.N` (v1 uses `'`, S-14 swaps to `.`)
    Index(u32),
    /// Dynamic array index: `'(expr)` — the Span covers the inner expression
    DynamicIndex(Span),
}

/// A parsed place path: `IDENT (step)*`
pub struct PlacePath {
    /// Span of the root identifier
    pub root: Span,
    /// Up to 8 chained steps
    pub steps: FixedVec<Step, 8>,
    /// Span covering the full place expression
    pub full: Span,
}

impl PlacePath {
    /// Absolute span of the root identifier given the word's base span offset.
    pub fn root_abs(&self, base: usize) -> Span {
        Span::new(base + self.root.start, base + self.root.end)
    }
}

/// Parse a place path from the lexer's current position.
///
/// Grammar (v1): `IDENT ( '.' IDENT | '\'' NUMBER | '\'' '(' expr ')' )*`
///
/// Returns `PlaceParseFailed` if the token stream does not match.
pub fn parse_place_path(
    lex: &mut Lexer<'_>,
    slice: &[u8],
) -> Result<PlacePath, TcError> {
    // Probe-based parsing: work on a copy of the lexer and only advance
    // the real lexer when we know the full place is valid.
    let mut probe = *lex;
    let first = probe.next();
    if first.kind != TokenKind::Ident {
        return Err(TcError::PlaceParseFailed {
            span: Span::new(first.span.start, first.span.end),
        });
    }
    let root = first.span;
    let mut end = root.end;
    let mut steps: FixedVec<Step, 8> = FixedVec::new();

    loop {
        let mut step_probe = probe;
        let next = step_probe.next();
        match next.kind {
            TokenKind::PunctDot => {
                let seg = step_probe.next();
                if seg.kind != TokenKind::Ident {
                    return Err(TcError::PlaceParseFailed {
                        span: Span::new(seg.span.start, seg.span.end),
                    });
                }
                let field = TypeAtom::new(&slice[seg.span.start..seg.span.end])
                    .ok_or(TcError::PlaceParseFailed {
                        span: Span::new(seg.span.start, seg.span.end),
                    })?;
                steps.push(Step::Field(field))
                    .map_err(|_| TcError::PlaceTooDeep {
                        span: Span::new(root.start, seg.span.end),
                    })?;
                end = seg.span.end;
                probe = step_probe;
            }
            TokenKind::PunctApostrophe => {
                let idx = step_probe.next();
                match idx.kind {
                    TokenKind::Number => {
                        let n = crate::typecheck::util::parse_u32_any(
                            &slice[idx.span.start..idx.span.end],
                        ).ok_or(TcError::PlaceParseFailed {
                            span: Span::new(idx.span.start, idx.span.end),
                        })?;
                        steps.push(Step::Index(n))
                            .map_err(|_| TcError::PlaceTooDeep {
                                span: Span::new(root.start, idx.span.end),
                            })?;
                        end = idx.span.end;
                        probe = step_probe;
                    }
                    TokenKind::PunctLParen => {
                        // Dynamic index: '( expr )
                        let mut depth = 1u32;
                        let expr_start = idx.span.start + 1;
                        let mut last_end;
                        loop {
                            let t = step_probe.next();
                            last_end = t.span.end;
                            if t.kind == TokenKind::PunctRParen {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                        // After loop, last_end has advanced past the ')'
                        let expr_end = last_end - 1; // back to before ')'
                        steps.push(Step::DynamicIndex(Span::new(expr_start, expr_end)))
                            .map_err(|_| TcError::PlaceTooDeep {
                                span: Span::new(root.start, expr_end),
                            })?;
                        end = last_end;
                        probe = step_probe;
                    }
                    _ => {
                        return Err(TcError::PlaceParseFailed {
                            span: Span::new(idx.span.start, idx.span.end),
                        });
                    }
                }
            }
            _ => break,
        }
    }

    *lex = probe;
    let full = Span::new(root.start, end);
    Ok(PlacePath { root, steps, full })
}
