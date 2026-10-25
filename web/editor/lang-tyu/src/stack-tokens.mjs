// The atomic stack-effect-comment tokenizer (PLAN-WASM-2 FR-21/R-W3).
//
// At `(`, scan forward (via the stream's lookahead — the stream is NOT
// advanced) to the balanced `)`, treating double-quoted strings and nested
// groups as opaque. A `--` at the OUTERMOST level inside the group claims
// the WHOLE group as one StackEffectComment token — never re-tokenized
// inside; otherwise the `(` falls through as the plain LParen token
// (attribute lists, grouping). A group that spans a top-level newline is
// not a signature — signatures are single-line.
//
// Term ids come from the GENERATED `./parser.terms.js` (the same lezer
// run): this module is a build sibling of the parser, never hand-maintained.

import { ExternalTokenizer } from "@lezer/lr";
import * as terms from "./parser.terms.js";

const OPEN = 40; // (
const CLOSE = 41; // )
const QUOTE = 34; // "
const BACKSLASH = 92; // \
const DASH = 45; // -
const NEWLINE = 10;

export const stackToks = new ExternalTokenizer(
  (input, token) => {
    // `input.acceptToken(term, endOffset)` records the token; `token` is the
    // parser-side cache (its end is what acceptToken sets).
    if (input.next !== OPEN) return;

    let pos = input.pos;
    let depth = 0;
    let inString = false;
    let escaped = false;
    // Offset of an outermost `--` inside the group (-1 = none).
    let dashDash = -1;

    for (;;) {
      const off = pos - input.pos;
      const ch = input.peek(off);
      if (ch < 0) return; // EOF in the group: not a comment, not even a `(`
      if (!inString && ch === NEWLINE) return; // signatures are single-line
      if (escaped) {
        escaped = false;
      } else if (inString) {
        if (ch === BACKSLASH) escaped = true;
        else if (ch === QUOTE) inString = false;
      } else if (ch === QUOTE) {
        inString = true;
      } else if (ch === OPEN) {
        depth++;
      } else if (ch === CLOSE) {
        depth--;
        if (depth === 0) break;
      } else if (
        depth === 1 &&
        ch === DASH &&
        dashDash < 0 &&
        input.peek(off + 1) === DASH
      ) {
        dashDash = off;
      }
      pos++;
    }
    // `pos` is at the group's closing `)`.

    if (dashDash < 0) {
      // Accept just the `(` (end offset is relative to the stream position,
      // which this lookahead scan never advanced).
      input.acceptToken(terms.LParen, 1);
      return;
    }
    input.acceptToken(terms.StackEffectComment, pos - input.pos + 1);
  },
  { extend: false }
);
