// Consumer for the versioned codegen-wasm source-span custom section.
// Unknown versions, malformed varints, duplicate sections, and partial
// records fail closed; callers must not guess a source span.

function readUleb(bytes, cursor) {
  let value = 0;
  let shift = 0;
  for (;;) {
    if (cursor.at >= bytes.length || shift >= 35) throw new Error("invalid tyu.spans/1 LEB128");
    const byte = bytes[cursor.at++];
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return value >>> 0;
    shift += 7;
  }
}

export function decodeSpanPayload(payload) {
  const bytes = new Uint8Array(payload);
  const cursor = { at: 0 };
  const version = readUleb(bytes, cursor);
  if (version !== 1) throw new Error(`unsupported tyu.spans version ${version}`);
  const count = readUleb(bytes, cursor);
  const records = [];
  for (let i = 0; i < count; i++) {
    records.push({
      codeOffset: readUleb(bytes, cursor),
      spanStart: readUleb(bytes, cursor),
      spanLen: readUleb(bytes, cursor),
    });
  }
  if (cursor.at !== bytes.length) throw new Error("trailing bytes in tyu.spans/1");
  return records;
}

export function resolveHardwareTrap(_bytes, _module, _error, _records) {
  // RuntimeError.stack is implementation-specific and commonly reports a
  // JIT code offset. There is no portable proof that it is a module byte
  // offset, so joining it to tyu.spans/1 could land on an unrelated op. The
  // generated backend routes source memory OOB through tyu.trap before the
  // hardware instruction; unexpected raw traps take the visible interp
  // fallback in app.js.
  return null;
}
