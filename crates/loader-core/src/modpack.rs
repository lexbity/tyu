//! Module acquisition backends (S2 Phase 14).
//!
//! Three sources:
//! 1. **Modpack** — embedded blob array in the firmware image.
//! 2. **Host FS** — read `.lmod` from the filesystem (hosted targets).
//! 3. **Semihosting** — open file via debug protocol (reserved for ARM).
//!
//! The modpack is a section in the runtime image containing zero or more
//! `.lmod` blobs, each prefixed with a `u32` length (little-endian).
//!
//! Layout:
//! ```text
//! __lang_modpack_start:
//!   u32 len_1
//!   [len_1 bytes of .lmod data]
//!   u32 len_2
//!   [len_2 bytes of .lmod data]
//!   ...
//! __lang_modpack_end:
//! ```

// ---------------------------------------------------------------------------
// Modpack scanner
// ---------------------------------------------------------------------------

/// Iterates over `.lmod` blobs in an embedded modpack.
///
/// Each blob is length-prefixed (u32 LE).  Create via
/// [`ModpackIter::new_from_slice`] or the raw-pointer
/// [`ModpackIter::new_from_range`] (which requires `unsafe`).
pub struct ModpackIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ModpackIter<'a> {
    /// Create an iterator over a byte slice containing modpack data.
    pub fn new_from_slice(data: &'a [u8]) -> Self {
        ModpackIter { data, pos: 0 }
    }

    /// Create an iterator over the modpack range `[start, end)`.
    ///
    /// # Safety
    ///
    /// `start` and `end` must point to a valid memory range.  The range
    /// should correspond to a `.modpack` section in the runtime image.
    pub unsafe fn new_from_range(start: *const u8, end: *const u8) -> Self {
        let start_addr = start as usize;
        let end_addr = end as usize;
        let len = end_addr.saturating_sub(start_addr);
        let data = if len > 0 {
            unsafe { core::slice::from_raw_parts(start, len) }
        } else {
            &[]
        };
        ModpackIter { data, pos: 0 }
    }

    /// Return the next blob, or `None` if exhausted or malformed.
    pub fn next_blob(&mut self) -> Option<&'a [u8]> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        let len = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]) as usize;
        self.pos += 4;
        if len == 0 || self.pos + len > self.data.len() {
            return None;
        }
        let blob = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Some(blob)
    }

    /// The number of blobs in the modpack.
    pub fn count(&mut self) -> usize {
        let mut n = 0;
        while self.next_blob().is_some() {
            n += 1;
        }
        n
    }
}

// ---------------------------------------------------------------------------
// Hosted FS acquisition
// ---------------------------------------------------------------------------

/// Error codes for module acquisition.
pub const E_MODULE_NOT_FOUND: u32 = 5211;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn build_modpack(blobs: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        for b in blobs {
            let len = b.len() as u32;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(b);
        }
        data
    }

    #[test]
    fn empty_modpack_yields_none() {
        let mut iter = ModpackIter::new_from_slice(b"");
        assert!(iter.next_blob().is_none());
    }

    #[test]
    fn single_blob_modpack() {
        let blob = b"hello";
        let data = build_modpack(&[blob]);
        let mut iter = ModpackIter::new_from_slice(&data);
        let result = iter.next_blob().unwrap();
        assert_eq!(result, blob);
        assert!(iter.next_blob().is_none());
    }

    #[test]
    fn multiple_blobs_iterated() {
        let data = build_modpack(&[b"first", b"second", b"third"]);
        let mut iter = ModpackIter::new_from_slice(&data);
        assert_eq!(iter.next_blob().unwrap(), b"first");
        assert_eq!(iter.next_blob().unwrap(), b"second");
        assert_eq!(iter.next_blob().unwrap(), b"third");
        assert!(iter.next_blob().is_none());
    }

    #[test]
    fn count_blobs() {
        let data = build_modpack(&[b"a", b"bb", b"ccc"]);
        let mut iter = ModpackIter::new_from_slice(&data);
        assert_eq!(iter.count(), 3);
    }

    #[test]
    fn zero_length_blob_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes());
        let mut iter = ModpackIter::new_from_slice(&data);
        assert!(iter.next_blob().is_none());
    }

    #[test]
    fn new_from_range_works() {
        let data = build_modpack(&[b"test"]);
        let mut iter = unsafe {
            ModpackIter::new_from_range(data.as_ptr(), data.as_ptr().add(data.len()))
        };
        assert_eq!(iter.next_blob().unwrap(), b"test");
        assert!(iter.next_blob().is_none());
    }

    #[test]
    fn truncated_blob_length_rejected() {
        let data = vec![0x10u8, 0, 0, 0];
        let mut iter = ModpackIter::new_from_slice(&data);
        assert!(iter.next_blob().is_none());
    }
}
