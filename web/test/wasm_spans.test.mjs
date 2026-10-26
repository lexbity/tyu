import test from "node:test";
import assert from "node:assert/strict";
import { decodeSpanPayload, resolveHardwareTrap } from "../wasm_spans.mjs";

function uleb(value) {
  const out = [];
  do {
    let byte = value & 0x7f;
    value >>>= 7;
    if (value) byte |= 0x80;
    out.push(byte);
  } while (value);
  return out;
}

test("tyu.spans/1 decodes exact source records", () => {
  const payload = Uint8Array.from([1, 2, ...uleb(19), ...uleb(4), ...uleb(6), ...uleb(31), ...uleb(2), 0]);
  assert.deepEqual(decodeSpanPayload(payload), [
    { codeOffset: 19, spanStart: 4, spanLen: 6 },
    { codeOffset: 31, spanStart: 2, spanLen: 0 },
  ]);
});

test("tyu.spans/1 rejects unknown versions and malformed records", () => {
  assert.throws(() => decodeSpanPayload(Uint8Array.from([2, 0])), /unsupported/);
  assert.throws(() => decodeSpanPayload(Uint8Array.from([1, 1, 3, 4])), /LEB128/);
  assert.throws(() => decodeSpanPayload(Uint8Array.from([1, 0, 9])), /trailing/);
});

function section(id, payload) {
  return [id, ...uleb(payload.length), ...payload];
}

function wasmName(value) {
  const bytes = new TextEncoder().encode(value);
  return [...uleb(bytes.length), ...bytes];
}

test("hardware OOB PCs that cannot map to a Wasm byte offset fail closed", async () => {
  // JS engines expose implementation-specific compiled-code offsets in
  // RuntimeError stacks, not the Wasm byte offset. The backend therefore
  // guards source memory accesses and sends their spans through tyu.trap;
  // this fallback must never invent a span for a raw hardware trap.
  const bytes = Uint8Array.from([
    0, 97, 115, 109, 1, 0, 0, 0,
    ...section(1, [1, 0x60, 0, 0]),
    ...section(3, [1, 0]),
    ...section(5, [1, 0, 1]),
    ...section(7, [2, ...wasmName("memory"), 2, 0, ...wasmName("run"), 0, 0]),
    ...section(10, [1, 10, 0, 0x41, 0x80, 0x80, 4, 0x28, 2, 0, 0x1a, 0x0b]),
  ]);
  const module = await WebAssembly.compile(bytes);
  const instance = await WebAssembly.instantiate(module);
  let trap;
  try {
    instance.exports.run();
  } catch (error) {
    trap = error;
  }
  assert.ok(trap instanceof WebAssembly.RuntimeError, "fixture traps on its OOB load");

  const resolved = resolveHardwareTrap(bytes, module, trap, [
    { codeOffset: 48, spanStart: 23, spanLen: 3 },
  ]);
  assert.equal(resolved, null,
    "unmappable engine PCs must trigger the explicit fallback path, never a guessed source span");
});
