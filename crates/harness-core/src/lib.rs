//! Host-side harness logic shared by the toolchain driver (`tyu`) and
//! integration tests (`execution-tests`).
//!
//! Re-exports the canonical stack-bound scanner from `loader-core::rederive`
//! and provides the serial-protocol parser and ELF code-section scanner — all
//! as no_std pure-byte functions so they can be called from any context.

#![no_std]

pub use loader_core::rederive::{rederive_stack_high, Arch, TOP_SENTINEL};

// ---------------------------------------------------------------------------
// Framed diagnostic protocol — records & parser
// ---------------------------------------------------------------------------

/// A single record from the framed diagnostic protocol.
///
/// Wire format: `marker:u8 | len:u16-le | payload[len]`.
/// The [`Record`] borrows from the input buffer for variable-length payloads
/// (`Diag`); all other variants are owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record<'a> {
    /// A test assertion failure (`F` marker, len=0).
    Failure,
    /// All suites completed normally (`S` marker, len=1, payload[0]=0x0A).
    Complete,
    /// Peak data-stack depth measurement in slots (`H` marker, 4-byte u32-le payload).
    HighWater(u32),
    /// Number of assertions executed (`P` marker, 4-byte u32-le payload).
    Pass(u32),
    /// Trap/error diagnostic record (`D` marker, variable-length payload).
    Diag(&'a [u8]),
    /// Protocol version byte (`V` marker, 1-byte payload).
    Version(u8),
    /// Unrecognized marker with valid framing — the parser skips it.
    Unknown { marker: u8, len: u16 },
}

/// Framed-protocol record iterator.
///
/// Parses `marker | u16-le len | payload[len]` records in sequence.
/// Stops at the first truncated record and exposes `truncated()`.
pub struct ParseRecords<'a> {
    data: &'a [u8],
    pos: usize,
    truncated: bool,
}

impl<'a> ParseRecords<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            truncated: false,
        }
    }

    /// Returns `true` if the iterator stopped due to a truncated record
    /// (marker with missing len, or len without full payload).
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl<'a> Iterator for ParseRecords<'a> {
    type Item = Record<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }

        let marker = self.data[self.pos];
        self.pos += 1;

        // Every record has a u16-le length field.
        if self.pos + 2 > self.data.len() {
            self.truncated = true;
            return None;
        }
        let len =
            u16::from_le_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap()) as usize;
        self.pos += 2;

        if self.pos + len > self.data.len() {
            self.truncated = true;
            return None;
        }
        let payload = &self.data[self.pos..self.pos + len];
        self.pos += len;

        Some(match marker {
            b'F' if len == 0 => Record::Failure,
            b'S' if len == 1 && payload[0] == 0x0A => Record::Complete,
            b'H' if len == 4 => {
                Record::HighWater(u32::from_le_bytes(payload[..4].try_into().unwrap()))
            }
            b'P' if len == 4 => Record::Pass(u32::from_le_bytes(payload[..4].try_into().unwrap())),
            b'D' => Record::Diag(payload),
            b'V' if len == 1 => Record::Version(payload[0]),
            _ => Record::Unknown {
                marker,
                len: len as u16,
            },
        })
    }
}

/// Parse a byte slice into a framed-record iterator.
pub fn parse_records(data: &[u8]) -> ParseRecords<'_> {
    ParseRecords::new(data)
}

// ---------------------------------------------------------------------------
// Serial completion-protocol parser
// ---------------------------------------------------------------------------

/// Parsed summary of serial/test output emitted by a running image.
///
/// Handles two wire formats:
///
/// **Framed protocol** (detected by leading `V` byte):
/// - `V` (0x56): protocol version byte
/// - `F` (0x46): a test failure was signalled
/// - `S` (0x53): all suites completed
/// - `H` (0x48): runtime high-water measurement (slots)
/// - `P` (0x50): assertion count
/// - `D` (0x44): diagnostic record
/// - Unknown records are skipped by their length field.
///
/// **Legacy byte-scanning** (used when first byte is not `V`):
/// - `F` (0x46): a test failure was signalled
/// - `S` (0x53) followed by `\n`: all suites completed
/// - `H` (0x48) followed by u32-le: runtime high-water measurement (slots)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSummary {
    /// Number of `F` (failure) records.
    pub failures: usize,
    /// Whether an `S` (suite-complete) record was seen.
    pub completed: bool,
    /// Peak data-stack depth from the last `H` (high-water) record, in slots.
    pub high_slots: u32,
    /// Number of assertions executed, from the last `P` (pass-count) record.
    pub assertions: u32,
    /// Number of `D` (diagnostic) records.
    pub diagnostics: usize,
    /// True if a framed record was truncated (marker without len, or len
    /// without full payload).  Always `false` for legacy-parsed streams.
    pub truncated: bool,
    /// Protocol version from the `V` record, if the stream is framed.
    pub protocol_version: Option<u8>,
}

/// Parse serial output from a test run into an `OutputSummary`.
///
/// Auto-detects the wire format: a stream whose first byte is `V` (0x56) is
/// parsed as framed protocol; otherwise the legacy byte-scanning algorithm is
/// used.
pub fn parse_output(stdout: &[u8]) -> OutputSummary {
    if stdout.first() == Some(&b'V') {
        parse_output_framed(stdout)
    } else {
        parse_output_legacy(stdout)
    }
}

/// Parse a framed-protocol byte stream.
fn parse_output_framed(stdout: &[u8]) -> OutputSummary {
    let mut parser = ParseRecords::new(stdout);
    let mut failures = 0usize;
    let mut completed = false;
    let mut high_slots = 0u32;
    let mut assertions = 0u32;
    let mut diagnostics = 0usize;
    let mut protocol_version: Option<u8> = None;

    for rec in parser.by_ref() {
        match rec {
            Record::Failure => failures += 1,
            Record::Complete => completed = true,
            Record::HighWater(slots) => high_slots = slots,
            Record::Pass(count) => assertions = count,
            Record::Diag(..) => diagnostics += 1,
            Record::Version(v) => protocol_version = Some(v),
            Record::Unknown { .. } => {}
        }
    }

    let truncated = parser.truncated();

    OutputSummary {
        failures,
        completed,
        high_slots,
        assertions,
        diagnostics,
        truncated,
        protocol_version,
    }
}

/// Parse a legacy byte-scanning stream (backward compatible).
fn parse_output_legacy(stdout: &[u8]) -> OutputSummary {
    let mut failures = 0usize;
    let mut completed = false;
    let mut high_slots = 0u32;
    let mut i = 0;
    while i < stdout.len() {
        match stdout[i] {
            b'F' => failures += 1,
            b'S' => {
                if stdout.get(i + 1) == Some(&b'\n') {
                    completed = true;
                }
            }
            b'H' => {
                if i + 4 < stdout.len() {
                    high_slots = u32::from_le_bytes(stdout[i + 1..i + 5].try_into().unwrap());
                }
            }
            _ => {}
        }
        i += 1;
    }
    OutputSummary {
        failures,
        completed,
        high_slots,
        assertions: 0,
        diagnostics: 0,
        truncated: false,
        protocol_version: None,
    }
}

// ---------------------------------------------------------------------------
// ELF code-section scanner
// ---------------------------------------------------------------------------

/// Re-derive a conservative data-stack high-water bound (in slots) from the
/// code section of an ELF (ELF64 or ELF32), given as raw bytes.
///
/// Walks the program headers, selects `PT_LOAD` segments with `PF_X`, and
/// runs the architecture-specific scanner on each.  Returns the maximum over
/// all executable segments.
pub fn rederive_elf_high(elf: &[u8], slot_bytes: u8) -> u32 {
    if elf.len() < 64 {
        return 0;
    }
    if &elf[0..4] != b"\x7fELF" {
        return 0;
    }

    let elf_class = elf[4]; // 1 = ELF32, 2 = ELF64

    let (e_phoff, e_phentsize, e_phnum) = if elf_class == 2 {
        if elf.len() < 0x3a {
            return 0;
        }
        let phoff = u64::from_le_bytes(elf[0x20..0x28].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(elf[0x36..0x38].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(elf[0x38..0x3a].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    } else if elf_class == 1 {
        if elf.len() < 0x2e {
            return 0;
        }
        let phoff = u32::from_le_bytes(elf[0x1c..0x20].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(elf[0x2a..0x2c].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(elf[0x2c..0x2e].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    } else {
        return 0;
    };

    let mut total_high = 0u32;

    for i in 0..e_phnum {
        let off = e_phoff + i * e_phentsize;
        let phdr_size = if elf_class == 2 { 56usize } else { 32usize };
        if off + phdr_size > elf.len() {
            break;
        }

        // p_type at offset 0, p_flags at offset 4
        let p_type = u32::from_le_bytes(elf[off..off + 4].try_into().unwrap());
        if p_type != 1 {
            continue; // not PT_LOAD
        }
        let p_flags = u32::from_le_bytes(elf[off + 4..off + 8].try_into().unwrap());
        if p_flags & 1 == 0 {
            continue; // no PF_X
        }

        let (p_offset, p_filesz) = if elf_class == 2 {
            let po = u64::from_le_bytes(elf[off + 8..off + 16].try_into().unwrap()) as usize;
            let sz = u64::from_le_bytes(elf[off + 32..off + 40].try_into().unwrap()) as usize;
            (po, sz)
        } else {
            let po = u32::from_le_bytes(elf[off + 12..off + 16].try_into().unwrap()) as usize;
            let sz = u32::from_le_bytes(elf[off + 16..off + 20].try_into().unwrap()) as usize;
            (po, sz)
        };

        if p_offset + p_filesz > elf.len() || p_filesz == 0 {
            continue;
        }

        let code = &elf[p_offset..p_offset + p_filesz];
        let arch = Arch::detect_from_code(code);
        let slot32 = slot_bytes as u32;
        let high = rederive_stack_high(code, arch, slot32);
        total_high = total_high.max(high);
    }

    total_high
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    // ===================================================================
    // Framed-protocol record reader tests
    // ===================================================================

    // --- Helper: build a single framed record in a small Vec. -----------

    fn framed_failure() -> Vec<u8> {
        let mut buf = Vec::with_capacity(3);
        buf.push(b'F');
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf
    }

    fn framed_complete() -> Vec<u8> {
        let mut buf = Vec::with_capacity(4);
        buf.push(b'S');
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.push(0x0A);
        buf
    }

    fn framed_high_water(slots: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(7);
        buf.push(b'H');
        buf.extend_from_slice(&4u16.to_le_bytes());
        buf.extend_from_slice(&slots.to_le_bytes());
        buf
    }

    fn framed_pass(count: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(7);
        buf.push(b'P');
        buf.extend_from_slice(&4u16.to_le_bytes());
        buf.extend_from_slice(&count.to_le_bytes());
        buf
    }

    fn framed_version(ver: u8) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4);
        buf.push(b'V');
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.push(ver);
        buf
    }

    fn framed_diag(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + payload.len());
        buf.push(b'D');
        buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    fn framed_unknown(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + payload.len());
        buf.push(marker);
        buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    // --- Records -------------------------------------------------------

    #[test]
    fn parse_records_empty() {
        let mut r = parse_records(b"");
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_failure() {
        let data = framed_failure();
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::Failure));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_complete() {
        let data = framed_complete();
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::Complete));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_high_water() {
        let data = framed_high_water(42);
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::HighWater(42)));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_pass() {
        let data = framed_pass(7);
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::Pass(7)));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_version() {
        let data = framed_version(1);
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::Version(1)));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_diag() {
        let payload = b"\x10\x00\x00\x00\x01\x02\x03";
        let data = framed_diag(payload);
        let mut r = parse_records(&data);
        assert_eq!(r.next(), Some(Record::Diag(payload.as_slice())));
        assert!(r.next().is_none());
        assert!(!r.truncated());
    }

    #[test]
    fn parse_records_diag_containing_marker_bytes() {
        // D payload containing F (0x46), S (0x53), H (0x48) — must NOT
        // produce phantom Failure/Complete/HighWater records.
        let payload = b"\x46\x53\x48\x00\x01\x02";
        let data = framed_diag(payload);
        let records: Vec<Record<'_>> = parse_records(&data).collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], Record::Diag(payload.as_slice()));
    }

    #[test]
    fn parse_records_unknown_skipped_by_len() {
        let mut data = Vec::new();
        // Unknown marker 'X' with 4-byte payload
        data.extend_from_slice(&framed_unknown(b'X', b"AAAA"));
        // Followed by a known complete record
        data.extend_from_slice(&framed_complete());
        let records: Vec<Record<'_>> = parse_records(&data).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0],
            Record::Unknown {
                marker: b'X',
                len: 4,
            }
        );
        assert_eq!(records[1], Record::Complete);
    }

    #[test]
    fn parse_records_two_bytes_then_truncated_len() {
        // Marker followed by only 1 byte of len (needs 2)
        let data = b"F\x00";
        let mut r = parse_records(data);
        assert!(r.next().is_none());
        assert!(r.truncated());
    }

    #[test]
    fn parse_records_marker_only_truncated() {
        // Just a marker byte, no len
        let data = b"F";
        let mut r = parse_records(data);
        assert!(r.next().is_none());
        assert!(r.truncated());
    }

    #[test]
    fn parse_records_truncated_payload() {
        // H marker, len=4, but only 3 payload bytes follow
        let mut data = vec![b'H'];
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&[0x01, 0x02, 0x03]);
        let mut r = parse_records(&data);
        assert!(r.next().is_none());
        assert!(r.truncated());
    }

    #[test]
    fn parse_records_version_then_truncated() {
        let mut data = framed_version(1);
        // Append a truncated record: marker 'H' with len=4 but only 2 bytes
        data.push(b'H');
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&[0x01, 0x02]);
        let mut parser = ParseRecords::new(&data);
        let records: Vec<Record<'_>> = parser.by_ref().collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], Record::Version(1));
        assert!(parser.truncated());
    }

    #[test]
    fn parse_records_interleaved_stream() {
        // V F H P S D
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_failure());
        data.extend_from_slice(&framed_high_water(8));
        data.extend_from_slice(&framed_pass(3));
        data.extend_from_slice(&framed_complete());
        data.extend_from_slice(&framed_diag(b"diag"));

        let records: Vec<Record<'_>> = parse_records(&data).collect();
        assert_eq!(records.len(), 6);
        assert_eq!(records[0], Record::Version(1));
        assert_eq!(records[1], Record::Failure);
        assert_eq!(records[2], Record::HighWater(8));
        assert_eq!(records[3], Record::Pass(3));
        assert_eq!(records[4], Record::Complete);
        assert_eq!(records[5], Record::Diag(b"diag"));
    }

    // ===================================================================
    // parse_output — framed path
    // ===================================================================

    #[test]
    fn framed_empty_stream() {
        let s = parse_output(&framed_version(1));
        assert_eq!(s.failures, 0);
        assert!(!s.completed);
        assert_eq!(s.high_slots, 0);
        assert_eq!(s.assertions, 0);
        assert_eq!(s.diagnostics, 0);
        assert!(!s.truncated);
        assert_eq!(s.protocol_version, Some(1));
    }

    #[test]
    fn framed_failure_record() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_failure());
        let s = parse_output(&data);
        assert_eq!(s.failures, 1);
        assert!(!s.truncated);
    }

    #[test]
    fn framed_multiple_failures() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_failure());
        data.extend_from_slice(&framed_failure());
        let s = parse_output(&data);
        assert_eq!(s.failures, 2);
    }

    #[test]
    fn framed_completion() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_complete());
        let s = parse_output(&data);
        assert!(s.completed);
        assert!(!s.truncated);
    }

    #[test]
    fn framed_high_water_output() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_high_water(42));
        let s = parse_output(&data);
        assert_eq!(s.high_slots, 42);
        assert!(!s.truncated);
    }

    #[test]
    fn framed_pass_count() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_pass(7));
        let s = parse_output(&data);
        assert_eq!(s.assertions, 7);
        assert!(!s.truncated);
    }

    #[test]
    fn framed_diagnostics_count() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_diag(b"first"));
        data.extend_from_slice(&framed_diag(b"second"));
        let s = parse_output(&data);
        assert_eq!(s.diagnostics, 2);
        assert!(!s.truncated);
    }

    #[test]
    fn framed_diag_phantom_failure_prevention() {
        // D record whose payload contains F, S, H marker bytes.
        // The framed parser must NOT produce phantom failures/completions/water.
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_diag(b"\x46\x53\x48\x00\x01\x02"));
        data.extend_from_slice(&framed_complete());
        let s = parse_output(&data);
        assert_eq!(
            s.failures, 0,
            "D payload bytes must not cause phantom failures"
        );
        assert_eq!(s.diagnostics, 1, "exactly one D record");
        assert!(s.completed, "trailing S record still works");
        assert!(!s.truncated);
    }

    #[test]
    fn framed_unknown_record_skipped() {
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_unknown(b'X', b"skip"));
        data.extend_from_slice(&framed_complete());
        let s = parse_output(&data);
        assert!(s.completed);
        assert_eq!(s.protocol_version, Some(1));
        assert!(!s.truncated);
    }

    #[test]
    fn framed_truncated_len() {
        // V marker but no len byte
        let data = b"V";
        let s = parse_output(data);
        assert!(s.truncated);
    }

    #[test]
    fn framed_truncated_payload() {
        // H with len=4 but only 3 payload bytes
        let mut data = framed_version(1);
        data.push(b'H');
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&[1, 2, 3]);
        let s = parse_output(&data);
        assert!(s.truncated);
    }

    #[test]
    fn framed_interleaved_records() {
        // V F H P S — all record types in one stream
        let mut data = framed_version(1);
        data.extend_from_slice(&framed_failure());
        data.extend_from_slice(&framed_high_water(8));
        data.extend_from_slice(&framed_pass(3));
        data.extend_from_slice(&framed_complete());

        let s = parse_output(&data);
        assert_eq!(s.failures, 1);
        assert_eq!(s.high_slots, 8);
        assert_eq!(s.assertions, 3);
        assert!(s.completed);
        assert_eq!(s.protocol_version, Some(1));
        assert!(!s.truncated);
    }

    #[test]
    fn framed_without_version_uses_legacy() {
        // A stream that doesn't start with V is handed to the legacy parser.
        // The framed parser never runs, so protocol_version is None.
        // This tests the detection boundary.
        let data = framed_failure(); // starts with F, not V
        let s = parse_output(&data);
        // Legacy parser sees bare F byte.
        assert_eq!(s.failures, 1);
        assert_eq!(s.protocol_version, None);
        assert!(!s.truncated);
    }

    // ===================================================================
    // parse_output — legacy backward-compatibility path
    // ===================================================================

    #[test]
    fn legacy_empty_output() {
        let s = parse_output(b"");
        assert_eq!(s.failures, 0);
        assert!(!s.completed);
        assert_eq!(s.high_slots, 0);
        assert_eq!(s.protocol_version, None);
        assert!(!s.truncated);
    }

    #[test]
    fn legacy_completion_marker() {
        let s = parse_output(b"S\n");
        assert!(s.completed);
        assert_eq!(s.failures, 0);
    }

    #[test]
    fn legacy_failure_marker() {
        let s = parse_output(b"F");
        assert_eq!(s.failures, 1);
        assert!(!s.completed);
    }

    #[test]
    fn legacy_multiple_failures() {
        let s = parse_output(b"FF");
        assert_eq!(s.failures, 2);
    }

    #[test]
    fn legacy_s_without_newline_not_completed() {
        let s = parse_output(b"S");
        assert!(!s.completed);
    }

    #[test]
    fn legacy_high_water_marker() {
        let mut buf = Vec::new();
        buf.push(b'H');
        buf.extend_from_slice(&42u32.to_le_bytes());
        let s = parse_output(&buf);
        assert_eq!(s.high_slots, 42);
    }

    #[test]
    fn legacy_truncated_high_water_ignored() {
        // Only H + 3 bytes (needs 4)
        let buf = vec![b'H', 0x01, 0x02, 0x03];
        let s = parse_output(&buf);
        assert_eq!(s.high_slots, 0, "truncated H must not be parsed");
        assert!(!s.truncated, "legacy parser never sets truncated");
    }

    #[test]
    fn legacy_all_markers_together() {
        // F, H+8u32-le, S\n
        let mut buf = vec![b'F'];
        buf.push(b'H');
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(b"S\n");
        let s = parse_output(&buf);
        assert_eq!(s.failures, 1);
        assert_eq!(s.high_slots, 8);
        assert!(s.completed);
    }

    // ===================================================================
    // rederive_elf_high tests
    // ===================================================================

    /// Build a minimal ELF64 with one PT_LOAD+PF_X segment containing `code`.
    /// Returns the ELF bytes.
    fn make_elf64(code: &[u8]) -> Vec<u8> {
        let phdr_size: usize = 56;
        let ehdr_size: usize = 64;
        let total = ehdr_size + phdr_size + code.len();
        let mut buf = vec![0u8; total];

        // ELF identification
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // ELF64
        buf[5] = 1; // little-endian
        buf[6] = 1; // ELF version

        // e_phoff (64-bit): offset 0x20, 8 bytes
        buf[0x20..0x28].copy_from_slice(&(ehdr_size as u64).to_le_bytes());
        // e_phentsize: offset 0x36, 2 bytes
        buf[0x36..0x38].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        // e_phnum: offset 0x38, 2 bytes
        buf[0x38..0x3a].copy_from_slice(&(1u16).to_le_bytes());

        // Program header at offset ehdr_size
        let phoff = ehdr_size;
        // p_type = PT_LOAD (1)
        buf[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes());
        // p_flags = PF_X | PF_R (5)
        buf[phoff + 4..phoff + 8].copy_from_slice(&5u32.to_le_bytes());
        // p_offset (8 bytes) = after phdr
        let code_off = (ehdr_size + phdr_size) as u64;
        buf[phoff + 8..phoff + 16].copy_from_slice(&code_off.to_le_bytes());
        // p_filesz (8 bytes)
        buf[phoff + 32..phoff + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());

        // Code
        let code_off_usize = ehdr_size + phdr_size;
        buf[code_off_usize..code_off_usize + code.len()].copy_from_slice(code);

        buf
    }

    /// Build a minimal ELF32 with one PT_LOAD+PF_X segment containing `code`.
    fn make_elf32(code: &[u8]) -> Vec<u8> {
        let phdr_size: usize = 32;
        let ehdr_size: usize = 52; // ELF32 ehdr is 52 bytes
        let total = ehdr_size + phdr_size + code.len();
        let mut buf = vec![0u8; total];

        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 1; // ELF32
        buf[5] = 1;
        buf[6] = 1;

        // e_phoff (32-bit): offset 0x1c, 4 bytes
        buf[0x1c..0x20].copy_from_slice(&(ehdr_size as u32).to_le_bytes());
        // e_phentsize: offset 0x2a, 2 bytes
        buf[0x2a..0x2c].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        // e_phnum: offset 0x2c, 2 bytes
        buf[0x2c..0x2e].copy_from_slice(&(1u16).to_le_bytes());

        let phoff = ehdr_size;
        buf[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        buf[phoff + 4..phoff + 8].copy_from_slice(&5u32.to_le_bytes()); // p_flags = PF_X|PF_R
                                                                        // p_offset (4 bytes) = after phdr
        let code_off = (ehdr_size + phdr_size) as u32;
        buf[phoff + 12..phoff + 16].copy_from_slice(&code_off.to_le_bytes());
        // p_filesz (4 bytes)
        buf[phoff + 16..phoff + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());

        let code_off_usize = ehdr_size + phdr_size;
        buf[code_off_usize..code_off_usize + code.len()].copy_from_slice(code);

        buf
    }

    #[test]
    fn elf64_no_code_returns_zero() {
        assert_eq!(rederive_elf_high(&make_elf64(b""), 8), 0);
    }

    #[test]
    fn elf32_no_code_returns_zero() {
        assert_eq!(rederive_elf_high(&make_elf32(b""), 4), 0);
    }

    #[test]
    fn elf64_x86_push_detected() {
        // add r15, 8 (push, 1 slot of 8 bytes)
        let code = vec![0x49, 0x83, 0xc7, 0x08];
        let elf = make_elf64(&code);
        assert_eq!(rederive_elf_high(&elf, 8), 1);
    }

    #[test]
    fn elf32_arm_push_detected() {
        // adds r4, r4, #4 (push, 1 slot of 4 bytes)
        // 0x1A44 LE = [0x44, 0x1A]
        let code = vec![0x44, 0x1a];
        let elf = make_elf32(&code);
        assert_eq!(rederive_elf_high(&elf, 4), 1);
    }

    #[test]
    fn elf64_riscv_push_detected() {
        // addi s2, s2, 4 (push, 1 slot of 4 bytes)
        let code = vec![0x13, 0x09, 0x49, 0x00]; // addi s2, s2, 4
        let elf = make_elf64(&code);
        assert_eq!(rederive_elf_high(&elf, 4), 1);
    }

    #[test]
    fn not_an_elf() {
        assert_eq!(rederive_elf_high(b"not an elf", 8), 0);
    }

    #[test]
    fn too_short_elf() {
        assert_eq!(rederive_elf_high(b"\x7fELF", 8), 0);
    }

    #[test]
    fn non_executable_segment_ignored() {
        // An ELF with PF_R only (no PF_X) should give 0
        let code = vec![0x49, 0x83, 0xc7, 0x08];
        let mut elf = make_elf64(&code);
        let phoff = 64usize;
        elf[phoff + 4..phoff + 8].copy_from_slice(&4u32.to_le_bytes()); // PF_R only
        assert_eq!(rederive_elf_high(&elf, 8), 0);
    }

    #[test]
    fn multiple_segments_max_taken() {
        // Two PT_LOAD+PF_X segments; scan takes the max high-water
        let push1 = vec![0x49, 0x83, 0xc7, 0x08]; // 1 slot
        let push3 = vec![
            0x49, 0x83, 0xc7, 0x08, 0x49, 0x83, 0xc7, 0x08, 0x49, 0x83, 0xc7, 0x08,
        ]; // 3 slots

        let phdr_size: usize = 56;
        let ehdr_size: usize = 64;
        let phdrs_off = ehdr_size;
        let seg1_off = phdrs_off + 2 * phdr_size;
        let seg2_off = seg1_off + push1.len();
        let total = seg2_off + push3.len();
        let mut buf = vec![0u8; total];

        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2;
        buf[5] = 1;
        buf[6] = 1;

        buf[0x20..0x28].copy_from_slice(&(phdrs_off as u64).to_le_bytes());
        buf[0x36..0x38].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        buf[0x38..0x3a].copy_from_slice(&(2u16).to_le_bytes());

        // PHDR 0: seg1
        let ph0 = phdrs_off;
        buf[ph0..ph0 + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[ph0 + 4..ph0 + 8].copy_from_slice(&5u32.to_le_bytes());
        buf[ph0 + 8..ph0 + 16].copy_from_slice(&(seg1_off as u64).to_le_bytes());
        buf[ph0 + 32..ph0 + 40].copy_from_slice(&(push1.len() as u64).to_le_bytes());

        // PHDR 1: seg2
        let ph1 = phdrs_off + phdr_size;
        buf[ph1..ph1 + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[ph1 + 4..ph1 + 8].copy_from_slice(&5u32.to_le_bytes());
        buf[ph1 + 8..ph1 + 16].copy_from_slice(&(seg2_off as u64).to_le_bytes());
        buf[ph1 + 32..ph1 + 40].copy_from_slice(&(push3.len() as u64).to_le_bytes());

        buf[seg1_off..seg1_off + push1.len()].copy_from_slice(&push1);
        buf[seg2_off..seg2_off + push3.len()].copy_from_slice(&push3);

        assert_eq!(rederive_elf_high(&buf, 8), 3);
    }
}
