//! The binary WebAssembly encoder (PLAN-WASM-2 B1, DEC-35): LEB128, section
//! framing, and the value encodings the backend needs — hand-rolled, zero
//! dependencies (R-C). Every emitted module passes `wasm-tools validate` in
//! CI before it is run (the validator is a TOOL, never a crate dep); the
//! unit tests pin the byte layout, and the G54 legs prove the validator is
//! actually consulted (a malformed module must be rejected, not silently
//! accepted — `tests/validator.rs` feeds deliberately malformed sections to
//! the validator and requires the rejection).

use alloc::vec::Vec;

/// Unsigned LEB128 (u32/u64 widths the format uses).
pub fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Signed LEB128 (i32/i64 constants, block types).
pub fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let sign = b & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// A FIXED-WIDTH (padded) unsigned LEB: format-legal (the format allows
/// redundant leading zero groups) and byte-stable, which is what makes call
/// operands patchable after emission — the backend records the operand's
/// offset during streaming and overwrites these bytes once the callee's
/// function index is known. The LAST byte never carries the continuation
/// bit (a 0x80-heavy pad would swallow the code that follows into the
/// operand).
pub fn padded_uleb(out: &mut Vec<u8>, v: u64, width: usize) {
    let mut v = v;
    for i in 0..width {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if i + 1 < width {
            byte |= 0x80;
        }
        out.push(byte);
    }
}

/// A name as the format encodes it: u32 length + UTF-8 bytes.
pub fn name(out: &mut Vec<u8>, n: &[u8]) {
    uleb(out, n.len() as u64);
    out.extend_from_slice(n);
}

/// Section framing: id + u32 byte length + payload. `payload` is built by
/// `f` into a scratch buffer so the length is always exact.
pub fn section(out: &mut Vec<u8>, id: u8, f: impl FnOnce(&mut Vec<u8>)) {
    let mut body = Vec::new();
    f(&mut body);
    out.push(id);
    uleb(out, body.len() as u64);
    out.extend_from_slice(&body);
}

// Section ids (the MVP set the backend emits).
pub const SEC_TYPE: u8 = 1;
pub const SEC_IMPORT: u8 = 2;
pub const SEC_FUNCTION: u8 = 3;
pub const SEC_MEMORY: u8 = 5;
pub const SEC_GLOBAL: u8 = 6;
pub const SEC_EXPORT: u8 = 7;
pub const SEC_CODE: u8 = 10;
pub const SEC_DATA: u8 = 11;

// Value type encodings.
pub const VAL_I32: u8 = 0x7f;
pub const VAL_I64: u8 = 0x7e;

// Fixed-width opcodes the backend emits. The single home of opcode bytes in
// the crate — backends never inline a raw byte.
pub const OP_UNREACHABLE: u8 = 0x00;
pub const OP_BLOCK: u8 = 0x02;
pub const OP_LOOP: u8 = 0x03;
pub const OP_IF: u8 = 0x04;
pub const OP_END: u8 = 0x0b;
pub const OP_ELSE: u8 = 0x05;
pub const OP_BR: u8 = 0x0c;
pub const OP_BR_TABLE: u8 = 0x0e;
pub const OP_RETURN: u8 = 0x0f;
pub const OP_CALL: u8 = 0x10;
pub const OP_DROP: u8 = 0x1a;
pub const OP_SELECT: u8 = 0x1b;
pub const OP_LOCAL_GET: u8 = 0x20;
pub const OP_LOCAL_SET: u8 = 0x21;
pub const OP_GLOBAL_GET: u8 = 0x23;
pub const OP_GLOBAL_SET: u8 = 0x24;
pub const OP_I64_LOAD: u8 = 0x29;
pub const OP_I64_STORE: u8 = 0x37;
pub const OP_MEMORY_SIZE: u8 = 0x3f;
pub const OP_MEMORY_GROW: u8 = 0x40;
pub const OP_I32_CONST: u8 = 0x41;
pub const OP_I64_CONST: u8 = 0x42;
pub const OP_I32_EQZ: u8 = 0x45;
pub const OP_I32_EQ: u8 = 0x46;
pub const OP_I32_ADD: u8 = 0x6a;
pub const OP_I32_GE_U: u8 = 0x4f;
pub const OP_I32_LE_U: u8 = 0x4d;
pub const OP_I32_AND: u8 = 0x71;
pub const OP_I32_SUB: u8 = 0x6b;
pub const OP_I32_SHL: u8 = 0x74;
pub const OP_I64_EQZ: u8 = 0x50;
pub const OP_I64_EQ: u8 = 0x51;
pub const OP_I64_NE: u8 = 0x52;
pub const OP_I64_LT_S: u8 = 0x53;
#[allow(dead_code)] // the pinned signed/unsigned compare table stays complete
pub const OP_I64_LT_U: u8 = 0x54;
pub const OP_I64_GT_S: u8 = 0x55;
pub const OP_I64_GT_U: u8 = 0x56;
pub const OP_I64_LE_S: u8 = 0x57;
#[allow(dead_code)] // the pinned signed/unsigned compare table stays complete
pub const OP_I64_LE_U: u8 = 0x58;
pub const OP_I64_GE_S: u8 = 0x59;
pub const OP_I64_GE_U: u8 = 0x5a;
pub const OP_I64_ADD: u8 = 0x7c;
pub const OP_I64_SUB: u8 = 0x7d;
pub const OP_I64_MUL: u8 = 0x7e;
pub const OP_I64_AND: u8 = 0x83;
pub const OP_I64_OR: u8 = 0x84;
pub const OP_I64_XOR: u8 = 0x85;
pub const OP_I64_SHL: u8 = 0x86;
pub const OP_I64_SHR_U: u8 = 0x88;
pub const OP_I64_SHR_S: u8 = 0x87;
pub const OP_I32_WRAP_I64: u8 = 0xa7;
pub const OP_I64_EXTEND_I32_U: u8 = 0xad;

/// `memarg` (align log2, offset) — the load/store addressing operand pair.
/// Align 0 (= 1-byte granularity hint) is always valid; the offset stays in
/// the instruction (word addresses are runtime values).
pub fn memarg(out: &mut Vec<u8>, offset: u64) {
    uleb(out, 0);
    uleb(out, offset);
}

/// A function type entry: `0x60 (params) -> (results)` with per-slot
/// valtypes (the `tyu.trap` import mixes i64 and i32 params).
pub fn functype(out: &mut Vec<u8>, params: &[u8], results: &[u8]) {
    out.push(0x60);
    uleb(out, params.len() as u64);
    out.extend_from_slice(params);
    uleb(out, results.len() as u64);
    out.extend_from_slice(results);
}

/// A local GROUP declaration sequence: `(count, type)` runs — the leading
/// uleb is the GROUP count, not the local count.
pub fn locals_group(out: &mut Vec<u8>, count: usize, ty: u8) {
    uleb(out, count as u64);
    out.push(ty);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn uleb_known_encodings() {
        // The format's examples: 624485, 0, 127, 128, and u32::MAX.
        let enc = |v: u64| {
            let mut b = Vec::new();
            uleb(&mut b, v);
            b
        };
        assert_eq!(enc(0), vec![0x00]);
        assert_eq!(enc(1), vec![0x01]);
        assert_eq!(enc(127), vec![0x7f]);
        assert_eq!(enc(128), vec![0x80, 0x01]);
        assert_eq!(enc(624485), vec![0xe5, 0x8e, 0x26]);
        assert_eq!(enc(u32::MAX as u64), vec![0xff, 0xff, 0xff, 0xff, 0x0f]);
    }

    #[test]
    fn sleb_boundary_encodings() {
        let enc = |v: i64| {
            let mut b = Vec::new();
            sleb(&mut b, v);
            b
        };
        assert_eq!(enc(0), vec![0x00]);
        assert_eq!(enc(1), vec![0x01]);
        assert_eq!(enc(-1), vec![0x7f]);
        assert_eq!(enc(63), vec![0x3f]);
        assert_eq!(enc(64), vec![0xc0, 0x00]);
        assert_eq!(enc(-64), vec![0x40]);
        assert_eq!(enc(-65), vec![0xbf, 0x7f]);
        assert_eq!(enc(-123456), vec![0xc0, 0xbb, 0x78]);
        assert_eq!(enc(i32::MIN as i64), vec![0x80, 0x80, 0x80, 0x80, 0x78]);
        assert_eq!(enc(i32::MAX as i64), vec![0xff, 0xff, 0xff, 0xff, 0x07]);
        assert_eq!(
            enc(i64::MIN),
            vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f]
        );
        assert_eq!(
            enc(i64::MAX),
            // The 10th byte (0x00) disambiguates the sign — without it the
            // leading-ones tail would read as -1.
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]
        );
    }

    #[test]
    fn padded_uleb_is_fixed_width_and_terminates_clean() {
        for v in [0u64, 1, 127, 128, 10_000, u32::MAX as u64] {
            let mut b = Vec::new();
            padded_uleb(&mut b, v, 5);
            assert_eq!(b.len(), 5);
            // The last byte must NOT carry the continuation bit — a padded
            // operand that swallowed the next opcode would mis-decode.
            assert_eq!(b[4] & 0x80, 0);
        }
        // A padded zero decodes as zero: strip continuation bits, shift back.
        let mut b = Vec::new();
        padded_uleb(&mut b, 42, 5);
        let decoded = b
            .iter()
            .rev()
            .fold(0u64, |acc, byte| (acc << 7) | u64::from(byte & 0x7f));
        assert_eq!(decoded, 42);
    }

    #[test]
    fn section_frames_length_exactly() {
        let mut out = Vec::new();
        section(&mut out, SEC_TYPE, |b| {
            uleb(b, 1); // 1 type
            functype(b, &[VAL_I64, VAL_I64], &[VAL_I64]);
        });
        // id(1) + len + payload; the length must cover the payload exactly.
        assert_eq!(out[0], SEC_TYPE);
        let len = out[1] as usize;
        assert_eq!(out.len(), 2 + len);
        // payload: count(1) + functype(1 + 1+2 + 1+1) = 7
        assert_eq!(len, 7);
    }

    #[test]
    fn mixed_param_functype_encodes_per_slot() {
        let mut out = Vec::new();
        functype(&mut out, &[VAL_I64, VAL_I32, VAL_I32], &[]);
        assert_eq!(
            out,
            vec![0x60, 0x03, VAL_I64, VAL_I32, VAL_I32, 0x00],
            "tyu.trap's (i64, i32, i32) -> () signature"
        );
    }

    #[test]
    fn empty_section_is_well_formed() {
        let mut out = Vec::new();
        section(&mut out, SEC_MEMORY, |b| {
            uleb(b, 1); // 1 memory
            uleb(b, 1); // limits flag: min AND max
            uleb(b, 16);
            uleb(b, 256);
        });
        assert_eq!(&out[2..], &[1, 1, 16, 0x80, 0x02]);
    }

    #[test]
    fn signed_compare_opcodes_match_the_spec() {
        // The G54 parity surface leans on signed i64 comparisons (native
        // `setl/setle/setg/setge`); the signed/unsigned byte split here is
        // the format's table — pinned so a typo cannot become a miscompile.
        assert_eq!(OP_I64_LT_S, 0x53);
        assert_eq!(OP_I64_LT_U, 0x54);
        assert_eq!(OP_I64_GT_S, 0x55);
        assert_eq!(OP_I64_GT_U, 0x56);
        assert_eq!(OP_I64_LE_S, 0x57);
        assert_eq!(OP_I64_LE_U, 0x58);
        assert_eq!(OP_I64_GE_S, 0x59);
        assert_eq!(OP_I64_GE_U, 0x5a);
    }
}
