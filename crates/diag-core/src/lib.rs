//! Shared diagnostic record schema for the runtime diagnostic protocol.
//!
//! Provides:
//! - [`DiagRecord`]: the fixed 35-byte header of a runtime diagnostic record.
//! - [`encode_header`]: `no_std` encoding of a `DiagRecord` into a byte buffer.
//! - [`DiagRecord::parse`] (behind `feature = "std"`): decode a `DiagRecord`
//!   from a byte slice, with slot-count bounds validation.
//! - [`claims`]: trap-code to human-readable claim-text mapping.
//!
//! Wire format follows `devdocs/design-doc/runtime-diagnostic-protocol-v1.md §2`.
//!
//! Optional (`feature = "std"`):
//! - [`decode`]: host-side decoder with `ModinfoIndex`, `Diagnostic`, and `resolve`.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

pub mod claims;

#[cfg(feature = "std")]
pub mod decode;

#[cfg(feature = "std")]
pub mod render;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Current schema version for `DiagRecord`.
pub const DIAG_RECORD_VERSION: u8 = 1;

/// Size of the fixed DiagRecord header in bytes.
pub const DIAG_HEADER_SIZE: usize = 35;

/// Sentinel value for `ds_declared` meaning "unknown or ⊤" (abi-contract §2.2).
pub const DS_DECLARED_UNKNOWN: u32 = 0xFFFF_FFFF;

/// Well-known `origin` field values.
pub mod origin {
    /// Emitted by the in-guest B agent (trap handler asm).
    pub const IN_GUEST: u8 = 1;
    /// Extracted by the host A escalation (gdbstub RSP client).
    pub const GDBSTUB: u8 = 2;
}

// ---------------------------------------------------------------------------
// DiagRecord — fixed 35-byte header
// ---------------------------------------------------------------------------

/// The fixed header of a runtime diagnostic record.
///
/// Wire format (little-endian):
///
/// | Offset | Size | Field         | Meaning                                      |
/// |--------|------|---------------|----------------------------------------------|
/// | 0      | 1    | version       | schema version (currently 1)                 |
/// | 1      | 1    | origin        | 1=in-guest, 2=gdbstub                        |
/// | 2      | 1    | valid         | 1=language trap, 0=hardware fault            |
/// | 3      | 2    | trap_code     | runtime / 50xx / 51xx claim code             |
/// | 5      | 4    | source_line   | from debug_trap_loc; 0=unknown               |
/// | 9      | 8    | word_hash     | full fnv1a_u64(name)                         |
/// | 17     | 8    | trap_pc       | faulting PC                                  |
/// | 25     | 4    | ds_depth      | live data-stack depth at trap, in slots      |
/// | 29     | 4    | ds_declared   | declared high(word); 0xFFFFFFFF=⊤/unknown    |
/// | 33     | 2    | slot_count    | number of trailing u64 DS slots (0=header-only) |
///
/// Total fixed header: 35 bytes.
/// Variable tail: `slot_count × 8` bytes of slot data (deepest-last).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagRecord {
    pub version: u8,
    pub origin: u8,
    pub valid: bool,
    pub trap_code: u16,
    pub source_line: u32,
    pub word_hash: u64,
    pub trap_pc: u64,
    pub ds_depth: u32,
    pub ds_declared: u32,
    pub slot_count: u16,
}

impl DiagRecord {
    /// Encode the 35-byte fixed header into `buf`.
    ///
    /// Returns `Some(DIAG_HEADER_SIZE)` on success, or `None` if `buf` is too
    /// small.  The caller is responsible for appending `slot_count × 8` bytes
    /// of slot data after the header (deepest-last, each as u64-le).
    pub fn encode_header(&self, buf: &mut [u8]) -> Option<usize> {
        if buf.len() < DIAG_HEADER_SIZE {
            return None;
        }

        buf[0] = self.version;
        buf[1] = self.origin;
        buf[2] = self.valid as u8;
        buf[3..5].copy_from_slice(&self.trap_code.to_le_bytes());
        buf[5..9].copy_from_slice(&self.source_line.to_le_bytes());
        buf[9..17].copy_from_slice(&self.word_hash.to_le_bytes());
        buf[17..25].copy_from_slice(&self.trap_pc.to_le_bytes());
        buf[25..29].copy_from_slice(&self.ds_depth.to_le_bytes());
        buf[29..33].copy_from_slice(&self.ds_declared.to_le_bytes());
        buf[33..35].copy_from_slice(&self.slot_count.to_le_bytes());

        Some(DIAG_HEADER_SIZE)
    }
}

// ---------------------------------------------------------------------------
// Decode — available only with `feature = "std"`
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
impl DiagRecord {
    /// Decode a `DiagRecord` from a byte slice.
    ///
    /// The slice must contain at least 35 bytes (the fixed header).  If
    /// `slot_count > 0`, the slice must also contain `slot_count × 8` bytes
    /// of slot data after the header.  Returns `None` if either constraint is
    /// violated.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < DIAG_HEADER_SIZE {
            return None;
        }

        let version = buf[0];
        let origin = buf[1];
        let valid = buf[2] != 0;
        let trap_code = u16::from_le_bytes([buf[3], buf[4]]);
        let source_line = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let word_hash = u64::from_le_bytes([
            buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15], buf[16],
        ]);
        let trap_pc = u64::from_le_bytes([
            buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23], buf[24],
        ]);
        let ds_depth = u32::from_le_bytes([buf[25], buf[26], buf[27], buf[28]]);
        let ds_declared = u32::from_le_bytes([buf[29], buf[30], buf[31], buf[32]]);
        let slot_count = u16::from_le_bytes([buf[33], buf[34]]);

        // Validate slot_count bounds: at least slot_count × 8 bytes must
        // remain after the fixed header.
        let required = DIAG_HEADER_SIZE + slot_count as usize * 8;
        if buf.len() < required {
            return None;
        }

        Some(Self {
            version,
            origin,
            valid,
            trap_code,
            source_line,
            word_hash,
            trap_pc,
            ds_depth,
            ds_declared,
            slot_count,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::claims;
    use alloc::vec;

    // -----------------------------------------------------------------------
    // encode_header — always available
    // -----------------------------------------------------------------------

    #[test]
    fn encode_header_returns_size() {
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: true,
            trap_code: 22,
            source_line: 100,
            word_hash: 0xAABB_CCDD_EEFF_0011,
            trap_pc: 0x1234_5678_9ABC_DEF0,
            ds_depth: 16,
            ds_declared: 256,
            slot_count: 0,
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        let n = rec.encode_header(&mut buf).unwrap();
        assert_eq!(n, DIAG_HEADER_SIZE);
    }

    #[test]
    fn encode_header_buffer_too_small() {
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: true,
            trap_code: 10,
            source_line: 0,
            word_hash: 0,
            trap_pc: 0,
            ds_depth: 0,
            ds_declared: DS_DECLARED_UNKNOWN,
            slot_count: 0,
        };
        let mut tiny = [0u8; 10];
        assert!(rec.encode_header(&mut tiny).is_none());
    }

    #[test]
    fn encode_header_all_fields_wire_order() {
        // Encode a known record and verify every byte of the output.
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: true,
            trap_code: 0x0A0B,
            source_line: 0x0102_0304,
            word_hash: 0x0807_0605_0403_0201,
            trap_pc: 0x1817_1615_1413_1211,
            ds_depth: 0x2021_2223,
            ds_declared: 0x2829_2A2B,
            slot_count: 0x0201,
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        rec.encode_header(&mut buf).unwrap();

        let expected: &[u8] = &[
            0x01, // version = 1
            0x01, // origin = IN_GUEST
            0x01, // valid = true
            0x0B, 0x0A, // trap_code = 0x0A0B (LE)
            0x04, 0x03, 0x02, 0x01, // source_line = 0x01020304 (LE)
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // word_hash (LE)
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, // trap_pc (LE)
            0x23, 0x22, 0x21, 0x20, // ds_depth = 0x20212223 (LE)
            0x2B, 0x2A, 0x29, 0x28, // ds_declared = 0x28292A2B (LE)
            0x01, 0x02, // slot_count = 0x0201 (LE)
        ];
        assert_eq!(&buf[..], expected);
    }

    #[test]
    fn encode_header_slot_count_zero() {
        let rec = DiagRecord {
            slot_count: 0,
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 0,
            }
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        rec.encode_header(&mut buf).unwrap();
        // slot_count bytes are 0x00 0x00
        assert_eq!(buf[33], 0);
        assert_eq!(buf[34], 0);
    }

    // -----------------------------------------------------------------------
    // encode → parse round-trip — requires `feature = "std"`
    // -----------------------------------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn encode_parse_roundtrip() {
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: true,
            trap_code: 5001,
            source_line: 42,
            word_hash: 0xDEAD_BEEF_CAFE_BABE,
            trap_pc: 0x8000_0000_1234_5678,
            ds_depth: 16,
            ds_declared: 256,
            slot_count: 0,
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        let n = rec.encode_header(&mut buf).unwrap();
        assert_eq!(n, DIAG_HEADER_SIZE);

        let decoded = DiagRecord::parse(&buf[..n]).unwrap();
        assert_eq!(decoded.version, rec.version);
        assert_eq!(decoded.origin, rec.origin);
        assert_eq!(decoded.valid, rec.valid);
        assert_eq!(decoded.trap_code, rec.trap_code);
        assert_eq!(decoded.source_line, rec.source_line);
        assert_eq!(decoded.word_hash, rec.word_hash);
        assert_eq!(decoded.trap_pc, rec.trap_pc);
        assert_eq!(decoded.ds_depth, rec.ds_depth);
        assert_eq!(decoded.ds_declared, rec.ds_declared);
        assert_eq!(decoded.slot_count, 0);
    }

    #[cfg(feature = "std")]
    #[test]
    fn encode_parse_roundtrip_with_slots() {
        let rec = DiagRecord {
            version: 1,
            origin: origin::GDBSTUB,
            valid: true,
            trap_code: 10,
            source_line: 7,
            word_hash: 0x1111_2222_3333_4444,
            trap_pc: 0xFFFF_FFFF_FFFF_FFF0,
            ds_depth: 32,
            ds_declared: DS_DECLARED_UNKNOWN,
            slot_count: 3,
        };
        let total = DIAG_HEADER_SIZE + 3 * 8;
        let mut buf = vec![0u8; total];
        let n = rec.encode_header(&mut buf).unwrap();
        assert_eq!(n, DIAG_HEADER_SIZE);

        // Write three sentinel slot values (deepest-last order)
        let slot_bytes: &[u64] = &[0xAAAAAAAAAAAA, 0xBBBBBBBBBBBB, 0xCCCCCCCCCCCC];
        for (i, &s) in slot_bytes.iter().enumerate() {
            let off = DIAG_HEADER_SIZE + i * 8;
            buf[off..off + 8].copy_from_slice(&s.to_le_bytes());
        }

        let decoded = DiagRecord::parse(&buf).unwrap();
        assert_eq!(decoded.slot_count, 3);
        assert_eq!(decoded.origin, origin::GDBSTUB);
        assert_eq!(decoded.trap_code, 10);
    }

    // -----------------------------------------------------------------------
    // valid=0 (hardware fault) path
    // -----------------------------------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn valid_zero_decodes_correctly() {
        // Encode a record with valid=false (hardware fault).
        // word_hash and source_line may be garbage; trap_pc is authoritative.
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: false,
            trap_code: 10,
            source_line: 0,
            word_hash: 0,
            trap_pc: 0x8000_1234,
            ds_depth: 0,
            ds_declared: DS_DECLARED_UNKNOWN,
            slot_count: 0,
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        rec.encode_header(&mut buf).unwrap();

        let decoded = DiagRecord::parse(&buf).unwrap();
        assert!(!decoded.valid, "valid=false must survive round-trip");
        // word_hash and source_line carry whatever was encoded (here 0).
        assert_eq!(decoded.word_hash, 0, "word_hash preserved even for valid=0");
        assert_eq!(
            decoded.source_line, 0,
            "source_line preserved even for valid=0"
        );
        // trap_pc is authoritative for hardware faults.
        assert_eq!(
            decoded.trap_pc, 0x8000_1234,
            "trap_pc is authoritative for valid=0"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn valid_zero_with_garbage_fields() {
        // Encode with valid=false and non-zero word_hash/line to verify
        // the decoder does NOT special-case them (they pass through).
        let rec = DiagRecord {
            version: 1,
            origin: origin::IN_GUEST,
            valid: false,
            trap_code: 10,
            source_line: 0xDEAD_CAFE,
            word_hash: 0xBAAD_F00D_BAAD_F00D,
            trap_pc: 0x0800_0000,
            ds_depth: 0,
            ds_declared: DS_DECLARED_UNKNOWN,
            slot_count: 0,
        };
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        rec.encode_header(&mut buf).unwrap();

        let decoded = DiagRecord::parse(&buf).unwrap();
        assert!(!decoded.valid);
        // Fields pass through unchanged even though they're semantically
        // meaningless for a hardware fault.
        assert_eq!(decoded.source_line, 0xDEAD_CAFE);
        assert_eq!(decoded.word_hash, 0xBAAD_F00D_BAAD_F00D);
        assert_eq!(decoded.trap_pc, 0x0800_0000);
    }

    // -----------------------------------------------------------------------
    // slot_count bounds enforcement
    // -----------------------------------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn slot_count_bounds_rejected_when_too_short() {
        let rec = DiagRecord {
            slot_count: 4, // declares 4 slots = 32 bytes of slot data
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 4,
            }
        };
        let mut header_only = [0u8; DIAG_HEADER_SIZE]; // 35 bytes, no slot data
        rec.encode_header(&mut header_only).unwrap();

        // parse must reject because slot_count=4 requires 35+32=67 bytes,
        // but we only provide 35.
        assert!(
            DiagRecord::parse(&header_only[..]).is_none(),
            "slot_count=4 with only 35 bytes must be rejected"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn slot_count_bounds_accepted_when_sufficient() {
        let rec = DiagRecord {
            slot_count: 4,
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 4,
            }
        };
        let total = DIAG_HEADER_SIZE + 4 * 8;
        let mut buf = vec![0u8; total];
        rec.encode_header(&mut buf).unwrap();

        // parse succeeds because buffer is large enough for declared slots.
        let decoded = DiagRecord::parse(&buf).unwrap();
        assert_eq!(decoded.slot_count, 4);
    }

    #[cfg(feature = "std")]
    #[test]
    fn slot_count_zero_accepted() {
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        let rec = DiagRecord {
            slot_count: 0,
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 0,
            }
        };
        rec.encode_header(&mut buf).unwrap();
        assert!(
            DiagRecord::parse(&buf).is_some(),
            "slot_count=0 always valid"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn slot_count_large_rejected() {
        // slot_count that makes HEADER_SIZE + count*8 overflow usize is
        // impossible on realistic platforms (count = u16::MAX = 65535,
        // 65535 * 8 = 524280 < usize::MAX on any 32+ bit target).  Check
        // that a very large but not-yet-overflowing count is rejected when
        // the buffer is too small.
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        let rec = DiagRecord {
            slot_count: 0xFFFF, // declares 65535 slots = 524280 bytes
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 0xFFFF,
            }
        };
        rec.encode_header(&mut buf).unwrap();
        // Buffer is only 35 bytes, need 35 + 524280 = 524315.
        assert!(DiagRecord::parse(&buf).is_none());
    }

    // -----------------------------------------------------------------------
    // parse rejects short buffers
    // -----------------------------------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn parse_rejects_short_buffer() {
        assert!(DiagRecord::parse(b"").is_none());
        assert!(DiagRecord::parse(&[0u8; 10]).is_none());
        assert!(DiagRecord::parse(&[0u8; 34]).is_none());
        // 35 bytes (exact header size, slot_count = 0) should succeed.
        // Build one properly.
        let mut buf = [0u8; DIAG_HEADER_SIZE];
        let rec = DiagRecord {
            slot_count: 0,
            ..DiagRecord {
                version: 1,
                origin: origin::IN_GUEST,
                valid: true,
                trap_code: 10,
                source_line: 0,
                word_hash: 0,
                trap_pc: 0,
                ds_depth: 0,
                ds_declared: 0,
                slot_count: 0,
            }
        };
        rec.encode_header(&mut buf).unwrap();
        assert!(DiagRecord::parse(&buf).is_some());
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn diag_record_version_constant() {
        assert_eq!(DIAG_RECORD_VERSION, 1);
    }

    #[test]
    fn header_size_constant() {
        assert_eq!(DIAG_HEADER_SIZE, 35);
    }

    #[test]
    fn ds_declared_unknown_sentinel() {
        assert_eq!(DS_DECLARED_UNKNOWN, 0xFFFF_FFFF);
    }

    #[test]
    fn origin_constants_distinct() {
        assert_eq!(origin::IN_GUEST, 1);
        assert_eq!(origin::GDBSTUB, 2);
        assert_ne!(origin::IN_GUEST, origin::GDBSTUB);
    }

    // -----------------------------------------------------------------------
    // claims re-export smoke test
    // -----------------------------------------------------------------------

    #[test]
    fn claim_text_coverage() {
        // The test in claims.rs covers full registry; this is a smoke test
        // that the module is accessible.
        assert_eq!(claims::claim_text(10), "STACK_OVERFLOW");
        assert_eq!(claims::claim_text(9999), "UNKNOWN_TRAP_CODE");
    }
}
