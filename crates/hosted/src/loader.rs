//! Hosted (Linux x86_64) implementation of `LoaderPlatform`.
//!
//! Uses `mmap`/`mprotect` for W^X section placement (S2 Phase 7, Phase 8).

use crate::{c, mem};
use core::ffi::c_void;
use loader_core::platform::{LoaderPlatform, Region, Tier};

/// Load error codes.
const E_MMAP_FAILED: u32 = 1;
const E_MPROTECT_FAILED: u32 = 2;

pub struct HostedLoaderPlatform {
    expected_abi_hash: u64,
    key: [u8; 64],
    key_len: usize,
    tier: Tier,
    kek: [u8; 32],
    /// Optional single-block reservation for adjacent allocations.
    /// When `Some`, all `alloc_*` calls carve from this block instead
    /// of calling `mmap`.  This guarantees PC-relative proximity.
    block: Option<Block>,
}

struct Block {
    base: *mut u8,
    capacity: usize,
    used: usize,
}

impl HostedLoaderPlatform {
    pub fn new(expected_abi_hash: u64) -> Self {
        Self {
            expected_abi_hash,
            key: [0u8; 64],
            key_len: 0,
            tier: Tier::Zero,
            kek: [0u8; 32],
            block: None,
        }
    }

    /// Configure for Tier 1 operation with an HMAC key.
    /// `key` must be 1–64 bytes.
    pub fn with_key(mut self, key: &[u8], tier: Tier) -> Self {
        let n = key.len().min(64);
        self.key[..n].copy_from_slice(&key[..n]);
        self.key_len = n;
        self.tier = tier;
        self
    }

    /// Configure a KEK for encrypted module decryption.
    pub fn with_kek(mut self, kek: &[u8; 32]) -> Self {
        self.kek = *kek;
        self
    }

    /// Allocate a block large enough for code + rodata + data + runtime data,
    /// all from a single `mmap`.  Use [`reserve`] before calling the loader.
    /// Subsequent `alloc_*` calls draw from this block.
    pub fn reserve(&mut self, capacity: usize) -> Result<(), u32> {
        let slice = mem::mmap_anon_rw(capacity).map_err(|_| E_MMAP_FAILED)?;
        let ptr = slice.as_mut_ptr();
        let _ = slice;
        self.block = Some(Block {
            base: ptr,
            capacity,
            used: 0,
        });
        Ok(())
    }

    /// Return the base address of the reserved block, or `None` if no
    /// reservation has been made.  Useful for placing runtime data at
    /// known offsets within the block.
    pub fn block_base(&self) -> Option<*mut u8> {
        self.block.as_ref().map(|b| b.base)
    }

    /// Return the capacity of the reserved block, or `None`.
    pub fn block_capacity(&self) -> Option<usize> {
        self.block.as_ref().map(|b| b.capacity)
    }
}

impl HostedLoaderPlatform {
    /// Release a previously allocated region (munmap).
    fn release_region(&mut self, region: &mut Region) {
        if region.len() > 0 && !region.as_ptr().is_null() {
            unsafe {
                c::munmap(
                    region.as_mut_ptr() as *mut core::ffi::c_void,
                    region.len(),
                );
            }
        }
    }
}

impl LoaderPlatform for HostedLoaderPlatform {
    fn alloc_exec(&mut self, len: usize) -> Result<Region, u32> {
        if let Some(ref mut b) = self.block {
            if b.used + len > b.capacity {
                return Err(E_MMAP_FAILED);
            }
            let ptr = unsafe { b.base.add(b.used) };
            b.used += len;
            return unsafe { Ok(Region::from_raw_parts(ptr, len)) };
        }
        let slice = mem::mmap_anon_rw(len).map_err(|_| E_MMAP_FAILED)?;
        let ptr = slice.as_mut_ptr();
        let _ = slice;
        unsafe { Ok(Region::from_raw_parts(ptr, len)) }
    }

    fn alloc_ro(&mut self, len: usize) -> Result<Region, u32> {
        if let Some(ref mut b) = self.block {
            if b.used + len > b.capacity {
                return Err(E_MMAP_FAILED);
            }
            let ptr = unsafe { b.base.add(b.used) };
            b.used += len;
            return unsafe { Ok(Region::from_raw_parts(ptr, len)) };
        }
        let ptr = mem::mmap_anon(len, mem::prot::READ).map_err(|_| E_MMAP_FAILED)?;
        unsafe { Ok(Region::from_raw_parts(ptr as *mut u8, len)) }
    }

    fn alloc_rw(&mut self, len: usize) -> Result<Region, u32> {
        if let Some(ref mut b) = self.block {
            if b.used + len > b.capacity {
                return Err(E_MMAP_FAILED);
            }
            let ptr = unsafe { b.base.add(b.used) };
            b.used += len;
            return unsafe { Ok(Region::from_raw_parts(ptr, len)) };
        }
        let slice = mem::mmap_anon_rw(len).map_err(|_| E_MMAP_FAILED)?;
        let ptr = slice.as_mut_ptr();
        let _ = slice;
        unsafe { Ok(Region::from_raw_parts(ptr, len)) }
    }

    fn make_exec(&mut self, region: &mut Region) -> Result<(), u32> {
        unsafe {
            mem::mprotect(
                region.as_mut_ptr() as *mut c_void,
                region.len(),
                mem::prot::READ | mem::prot::EXEC,
            )
            .map_err(|_| E_MPROTECT_FAILED)
        }
    }

    fn verify_sig(&self, signed: &[u8], sig: &[u8]) -> bool {
        if self.key_len == 0 {
            return true; // Tier 0: trust unconditionally
        }
        let expected = crate::hmac_sha256::hmac_sha256(&self.key[..self.key_len], signed);
        expected.as_slice() == sig
    }

    #[cfg(feature = "encryption")]
    fn unwrap_cek(&self, _key_id: u64, wrapped: &[u8], out_cek: &mut [u8; 32]) -> Result<(), u32> {
        use loader_core::crypto::chacha20poly1305::unwrap_cek as do_unwrap;
        const WRAP_LEN: usize = 12 + 32 + 16; // nonce + cek_ciphertext + tag
        let wrapped_arr: &[u8; WRAP_LEN] =
            wrapped.try_into().map_err(|_| loader_core::load::E_ENC_BAD_HEADER)?;
        let cek = do_unwrap(&self.kek, wrapped_arr)
            .map_err(|_| loader_core::load::E_ENC_NO_KEY)?;
        *out_cek = cek;
        Ok(())
    }

    fn expected_abi_hash(&self) -> u64 {
        self.expected_abi_hash
    }

    fn trust_tier(&self) -> Tier {
        self.tier
    }

    fn release(&mut self, region: &mut Region) {
        self.release_region(region);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    type FnReturningI64 = unsafe extern "C" fn() -> i64;

    const MOV_RAX_42_RET: &[u8] = &[
        0x48, 0xc7, 0xc0, 0x2a, 0x00, 0x00, 0x00,
        0xc3,
    ];

    #[test]
    fn alloc_exec_write_code_and_rx_execute() {
        let mut plat = HostedLoaderPlatform::new(0);
        let len = MOV_RAX_42_RET.len();
        let mut region = plat.alloc_exec(len).unwrap();
        assert!(region.len() >= len);
        assert!(!region.is_empty());
        unsafe {
            region.as_mut_slice()[..len].copy_from_slice(MOV_RAX_42_RET);
        }
        plat.make_exec(&mut region).unwrap();
        let func: FnReturningI64 = unsafe { core::mem::transmute(region.as_ptr()) };
        assert_eq!(unsafe { func() }, 42);
    }

    #[test]
    fn alloc_ro_readable() {
        let mut plat = HostedLoaderPlatform::new(0);
        let region = plat.alloc_ro(16).unwrap();
        assert_eq!(region.len(), 16);
        assert_eq!(&region.as_slice()[..8], &[0u8; 8]);
    }

    #[test]
    fn alloc_rw_writable() {
        let mut plat = HostedLoaderPlatform::new(0);
        let mut region = plat.alloc_rw(16).unwrap();
        assert!(region.len() >= 4);
        unsafe {
            region.as_mut_slice()[0..4].copy_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
        }
        assert_eq!(&region.as_slice()[0..4], &[0xaa, 0xbb, 0xcc, 0xdd]);
    }

    #[test]
    fn reserve_allocations_are_adjacent() {
        let mut plat = HostedLoaderPlatform::new(0);
        plat.reserve(8192).unwrap();
        let mut a = plat.alloc_exec(64).unwrap();
        let b = plat.alloc_ro(32).unwrap();
        let c = plat.alloc_rw(16).unwrap();
        // All three should be within 8192 bytes of each other.
        let a_start = a.as_ptr() as usize;
        let b_start = b.as_ptr() as usize;
        let c_start = c.as_ptr() as usize;
        assert!(b_start >= a_start + 64);
        assert!(c_start >= b_start + 32);
        // All are within the same block.
        unsafe {
            a.as_mut_slice()[0..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        }
        // Flip exec to RX after writing.
        plat.make_exec(&mut a).unwrap();
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn make_exec_page_is_rx_not_w() {
        // Verifies that after make_exec, the page is r-x (PROT_READ|PROT_EXEC)
        // and does NOT have write permission (no 'w' in /proc/self/maps).
        use std::vec::Vec;
        use std::string::String;

        fn prot_of(addr: *const u8) -> String {
            let pid = std::process::id();
            // Build "/proc/<pid>/maps" without std::format!
            let prefix = b"/proc/";
            let suffix = b"/maps\0";
            let mut path_buf = Vec::with_capacity(32);
            path_buf.extend_from_slice(prefix);
            let mut tmp = pid;
            let mut digits = [0u8; 10];
            let mut nd = 10;
            loop {
                nd -= 1;
                digits[nd] = b'0' + (tmp % 10) as u8;
                tmp /= 10;
                if tmp == 0 { break; }
            }
            path_buf.extend_from_slice(&digits[nd..10]);
            path_buf.extend_from_slice(suffix);
            // Remove trailing NUL
            let path_str = core::str::from_utf8(&path_buf[..path_buf.len() - 1]).unwrap();

            let mut maps = String::new();
            use std::io::Read;
            let mut file = std::fs::File::open(path_str).unwrap();
            file.read_to_string(&mut maps).unwrap();
            let addr_val = addr as usize;
            for line in maps.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 2 { continue; }
                let range: Vec<&str> = parts[0].split('-').collect();
                if range.len() != 2 { continue; }
                let start = usize::from_str_radix(range[0], 16).ok();
                let end = usize::from_str_radix(range[1], 16).ok();
                if let (Some(s), Some(e)) = (start, end) {
                    if addr_val >= s && addr_val < e {
                        return String::from(parts[1]);
                    }
                }
            }
            String::from("unknown")
        }

        let mut plat = HostedLoaderPlatform::new(0);
        let mut region = plat.alloc_exec(4096).unwrap();

        // Write known data while page is RW.
        unsafe { region.as_mut_slice()[0] = 0x01; }

        // Before make_exec: page should be rw- (writable).
        let before = prot_of(region.as_ptr());
        assert!(before.contains('w'), "before make_exec, page must be writable, got {before}");

        // Flip to RX.
        plat.make_exec(&mut region).unwrap();

        // After make_exec: page must be r-x (no 'w').
        let after = prot_of(region.as_ptr());
        assert!(!after.contains('w'), "after make_exec, page must NOT be writable, got {after}");
        assert!(
            after.contains('r') && after.contains('x'),
            "after make_exec, page must be readable+executable, got {after}"
        );

        // Verify code is still readable.
        let val = region.as_slice()[0];
        assert_eq!(val, 0x01, "code should be readable after make_exec");
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn make_exec_page_is_rx_not_w() {
        // Non-Linux: skip this test (the hosted platform targets Linux).
        eprintln!("SKIP: W^X verification requires Linux /proc/self/maps");
    }

    #[test]
    fn expected_abi_hash_matches() {
        let expected = lmod::abi_hash::compute_abi_hash(8, 64, lmod::modinfo::MODINFO_VER);
        let plat = HostedLoaderPlatform::new(expected);
        assert_eq!(plat.expected_abi_hash(), expected);
    }
}
