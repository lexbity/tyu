// The span bridge (PLAN-WASM-2 W4, DEC-21, I-9): the ONLY translator between
// the engine's byte spans and the editor's UTF-16 offsets. Engine spans are
// (start: u32, len: u32) UTF-8 BYTE offsets into the exact string submitted;
// CodeMirror positions are UTF-16 code-unit offsets. No component may index
// the doc with a raw engine span — everything goes through a map built here
// once per buffer at submit time.
//
// ASCII fast path: pure-ASCII buffers make the bridge an identity (asserted
// by a quick scan). The general path walks the UTF-8 encoding once (O(n))
// recording, for each byte index, the UTF-16 offset of the code point that
// starts there.

export function buildSpanMap(src) {
  const isAscii = (() => {
    for (let i = 0; i < src.length; i++) {
      if (src.charCodeAt(i) > 0x7f) return false;
    }
    return true;
  })();

  if (isAscii) {
    // Identity, by construction: UTF-8 bytes == code points == UTF-16 units.
    return {
      isAscii: true,
      toUtf16(byteStart, byteLen) {
        const clamp = (b) => Math.max(0, Math.min(b | 0, src.length));
        const from = clamp(byteStart);
        const to = Math.max(from, clamp(from + (byteLen | 0)));
        return { from, to };
      },
      byteLength: src.length,
    };
  }

  // General path: byte offset → UTF-16 offset for every code-point start,
  // plus the total byte length. A byte index inside a multi-byte sequence
  // resolves to the containing code point's start (spans are code-point
  // aligned by construction; a torn span clamps inward, never slices a
  // code point in half).
  const u16AtByte = new Map();
  const enc = [];
  let u16 = 0;
  for (let i = 0; i < src.length; i++) {
    const cp = src.codePointAt(i);
    const width = cp > 0xffff ? 2 : 1; // UTF-16 units
    const byteStart = enc.length;
    u16AtByte.set(byteStart, u16);
    if (cp < 0x80) enc.push(cp);
    else if (cp < 0x800) enc.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
    else if (cp < 0x10000)
      enc.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    else
      enc.push(
        0xf0 | (cp >> 18),
        0x80 | ((cp >> 12) & 0x3f),
        0x80 | ((cp >> 6) & 0x3f),
        0x80 | (cp & 0x3f)
      );
    u16 += width;
    if (cp > 0xffff) i++; // surrogate pair: one code point, two units
  }

  return {
    isAscii: false,
    byteLength: enc.length,
    toUtf16(byteStart, byteLen) {
      const clamp = (b) => {
        if (b <= 0) return 0;
        if (b >= enc.length) return u16;
        // Exact code-point start, or scan back to the start of the code
        // point containing this byte.
        let hit = u16AtByte.get(b);
        if (hit !== undefined) return hit;
        for (let back = b - 1; back >= 0; back--) {
          hit = u16AtByte.get(back);
          if (hit !== undefined) return hit;
        }
        return 0;
      };
      const from = clamp(byteStart | 0);
      const to = clamp((byteStart | 0) + (byteLen | 0));
      return { from, to: Math.max(from, to) };
    },
  };
}
