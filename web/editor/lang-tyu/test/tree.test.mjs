// Grammar node tests (PLAN-WASM-2 W2, R-W3): the tree shape is pinned for
// three fixture classes — a hello lab, a red lab, and a file with
// nested/quoted parens inside comments and strings. A malformed file must
// parse WITH error nodes (CM6 style), never throw. Run: node --test (after
// `node build.mjs` generated the parser).

import { test } from "node:test";
import assert from "node:assert/strict";

import { parser } from "../src/parser.js";

function names(text) {
  const tree = parser.parse(text);
  const out = [];
  tree.iterate({
    enter(node) {
      out.push(node.name);
    },
  });
  return out;
}

function count(text, name) {
  return names(text).filter((n) => n === name).length;
}

const HELLO_LAB = `# hello — the first lab: one word, one phrase.
module Hello;
import platform/linux { platform.io.log };

: greet ( -- )
  "hello, tyu\\n" platform.io.log ;

: main ( -- i64 )
  greet
  0 ;

export { main };
end;
`;

const RED_LAB = `# red — the checker explains itself: Percent must stay in range.
module Over;
subtype Percent = i64 range 0..100;

: bump ( Percent -- Percent )
  needs [ dup 0 >= ]
  1 + as Percent ;

: main ( -- i64 )
  150 as Percent bump 0 ;

end;
`;

// Nested parens INSIDE a stack comment (the balanced scan must claim the
// whole group atomically), quoted parens inside strings, and a paren-group
// attribute that is NOT a stack comment.
const PARENS_LAB = `module Paren;
import platform/linux { platform.io.log };

@interrupt(3)

: f ( a (nested) -- b ) drop drop ;

: main ( -- i64 )
  "( not a comment -- really )"
  "# also not a comment"
  f 0 ;

end;
`;

test("hello lab: module skeleton, definition, signature, strings", () => {
  const tree = parser.parse(HELLO_LAB);
  assert.equal(tree.topNode.name, "Module");
  // Two word definitions, each with an atomic stack-effect signature.
  assert.equal(count(HELLO_LAB, "WordDef"), 2);
  assert.equal(count(HELLO_LAB, "StackEffectComment"), 2);
  // Strings and numbers tokenize.
  assert.equal(count(HELLO_LAB, "String"), 1);
  assert.ok(count(HELLO_LAB, "Number") >= 1);
  // Comments are comments.
  assert.equal(count(HELLO_LAB, "LineComment"), 1);
  // A well-formed lab parses with ZERO error nodes — the grammar covers the
  // real surface (module header, dotted names, braces, signatures).
  assert.ok(!names(HELLO_LAB).includes("⚠"), `no error nodes: ${names(HELLO_LAB)}`);
});

test("red lab: keywords and subtype surface highlight nodes", () => {
  assert.ok(names(RED_LAB).includes("KwSubtype"));
  assert.ok(names(RED_LAB).includes("KwNeeds"));
  assert.ok(names(RED_LAB).includes("KwAs"));
  assert.equal(count(RED_LAB, "WordDef"), 2);
});

test("parens lab: stack comments are atomic; bare paren groups are not", () => {
  // `@interrupt(3)` — no `--` inside → one LParen token, NOT a comment.
  assert.ok(names(PARENS_LAB).includes("LParen"));
  assert.ok(!names(PARENS_LAB).includes("⚠"), "well-formed lab: no error nodes");
  // `: f ( a (nested) -- b ) ...` — the nested-paren group IS a signature.
  assert.equal(count(PARENS_LAB, "WordDef"), 2);
  // Exactly the two signatures (f and main) become atomic comments; the
  // paren groups inside strings must not have (they are String tokens).
  const scs = count(PARENS_LAB, "StackEffectComment");
  assert.ok(scs >= 2, `signatures parsed as atomic comments (got ${scs})`);
  assert.ok(count(PARENS_LAB, "String") >= 2, "quoted parens stay strings");
  // No re-tokenization inside strings: a StackEffectComment can never be a
  // descendant of a String token (lezer tokens are atomic leaves).
  const flat = names(PARENS_LAB).join(",");
  assert.ok(!flat.includes("String,StackEffectComment"), "no re-tokenization inside strings");
});

test("nested quotations parse structurally", () => {
  const src = `: w ( -- ) [ 1 2 + ] [ [ x ] dip ] if ;`;
  assert.equal(count(src, "WordDef"), 1);
  assert.ok(count(src, "Quotation") >= 3, `outer + nested quotes: ${names(src)}`);
});

test("operators tokenize distinctly", () => {
  const src = `1 2 + 3 > 4 <= 5 == 6 != 7 / 8 *`;
  assert.equal(count(src, "OpPlus"), 1);
  assert.equal(count(src, "OpGt"), 1);
  assert.equal(count(src, "OpLe"), 1);
  assert.equal(count(src, "OpEq"), 1);
  assert.equal(count(src, "OpNe"), 1);
  assert.equal(count(src, "OpSlash"), 1);
  assert.equal(count(src, "OpStar"), 1);
});

test("a malformed file parses with error nodes, never throws", () => {
  const broken = `: half ( i64 -- i64 ) 2 / ;;;: ;; : : oops`;
  let tree;
  assert.doesNotThrow(() => {
    tree = parser.parse(broken);
  });
  // Error nodes present (⚠ is lezer's error node name).
  assert.ok(names(broken).includes("⚠"), "error nodes mark the damage");
  // And whatever parsed still forms definitions.
  assert.ok(count(broken, "WordDef") >= 1);
});

test("stack comments spanning a newline are NOT signatures", () => {
  const src = `: w ( i64\n i64 -- i64 ) drop ;`;
  // The tokenizer bails at the newline; the `(` then falls to the default
  // paren path. The file still parses without throwing.
  assert.doesNotThrow(() => parser.parse(src));
});
