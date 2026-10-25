// The span-bridge property test (PLAN-WASM-2 W4, FR-36): for random strings
// spanning ASCII, the BMP, the astral plane, and combining marks — and for
// random spans — `toUtf16` output slices the JS string to EXACTLY the
// substring whose UTF-8 encoding is `bytes[start..start+len]`. Round-trips
// both directions. Deterministic (seeded LCG); node's built-in runner.

import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSpanMap } from "../spanmap.mjs";

// A seeded LCG: the property space is deterministic (same failures repeat).
function lcg(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 0x100000000;
  };
}

const ATOMS = [
  "a",
  "Z",
  "9",
  " ",
  "\n",
  "\t",
  "é", // BMP, 2 UTF-8 bytes
  "→", // BMP, 3 bytes
  "中", // BMP, 3 bytes
  "𝄞", // astral, 4 bytes, surrogate pair in UTF-16
  "́", // combining acute (U+0301), 2 bytes
  "🚀", // astral emoji
];

function randomString(rand) {
  const n = 1 + Math.floor(rand() * 40);
  let s = "";
  for (let i = 0; i < n; i++) {
    s += ATOMS[Math.floor(rand() * ATOMS.length)];
  }
  return s;
}

test("ascii fast path is the identity", () => {
  const src = ": w ( i64 -- i64 ) dup + ;";
  const map = buildSpanMap(src);
  assert.equal(map.isAscii, true);
  const { from, to } = map.toUtf16(2, 1);
  assert.equal(src.slice(from, to), "w");
  assert.equal(map.byteLength, src.length);
});

test("general path: exact byte→UTF-16 round trip over random corpora", () => {
  const rand = lcg(0x5eed);
  const encoder = new TextEncoder();
  for (let iter = 0; iter < 500; iter++) {
    const src = randomString(rand);
    const bytes = encoder.encode(src);
    const map = buildSpanMap(src);
    // Either path (a random draw can be pure ASCII): the round trip is the
    // property, not which branch served it.
    assert.equal(map.byteLength, bytes.length, `byte length: ${JSON.stringify(src)}`);

    // Random code-point-aligned spans.
    const cps = [...src];
    for (let k = 0; k < 10; k++) {
      const i = Math.floor(rand() * cps.length);
      const j = i + Math.floor(rand() * (cps.length - i) + (rand() < 0.9 ? 1 : 0));
      const want = cps.slice(i, j).join("");
      const byteStart = encoder.encode(cps.slice(0, i).join("")).length;
      const byteLen = encoder.encode(want).length;
      const { from, to } = map.toUtf16(byteStart, byteLen);
      assert.equal(
        src.slice(from, to),
        want,
        `iter ${iter}: bytes[${byteStart}..${byteStart + byteLen}] of ${JSON.stringify(src)}`
      );
    }

    // Degenerate spans never tear a code point.
    const zero = map.toUtf16(0, 0);
    assert.equal(zero.from, 0);
    assert.equal(zero.to, 0);
    const past = map.toUtf16(bytes.length + 100, 5);
    assert.equal(past.from, src.length);
    assert.equal(past.to, src.length);
  }
});

test("torn byte offsets clamp to the containing code point's start", () => {
  // "aé🚀" — bytes: a(0..1), é(1..3), 🚀(3..7). Every byte inside a code
  // point resolves to that code point's START; the span is then a half-open
  // code-point range — a torn edge NEVER slices a code point in half.
  const src = "aé🚀";
  const map = buildSpanMap(src);
  // [2..6): starts inside é → é's start; ends inside 🚀 → 🚀's start
  // (exclusive) → exactly "é".
  const { from, to } = map.toUtf16(2, 4);
  assert.equal(src.slice(from, to), "é");
  // [5..7): starts inside 🚀 → 🚀's start; ends at the buffer end → "🚀".
  const torn = map.toUtf16(5, 2);
  assert.equal(src.slice(torn.from, torn.to), "🚀");
});

test("multi-byte anchored spans used by real diagnostics", () => {
  // A word defined after a multi-byte comment: the engine's span lands
  // mid-buffer past multi-byte content.
  const src = "# n'est pas → comment\n: w ( -- ) ;";
  const map = buildSpanMap(src);
  const bytes = new TextEncoder().encode(src);
  // Find the byte offset of "w" and translate it.
  const u16Pos = src.indexOf("w");
  const bytePos = bytes.indexOf("w".charCodeAt(0), new TextEncoder().encode("# n'est pas →").length - 2);
  const { from, to } = map.toUtf16(bytePos, 1);
  assert.equal(from, u16Pos);
  assert.equal(src.slice(from, to), "w");
});
