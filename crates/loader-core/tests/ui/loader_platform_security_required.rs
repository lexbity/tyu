use loader_core::platform::{LoaderPlatform, Region, Rw, Rx};

struct IncompletePlatform;

impl LoaderPlatform for IncompletePlatform {
    fn alloc_exec(&mut self, _len: usize) -> Result<Region<Rw>, u32> {
        unimplemented!()
    }

    fn alloc_ro(&mut self, _len: usize) -> Result<Region<Rw>, u32> {
        unimplemented!()
    }

    fn alloc_rw(&mut self, _len: usize) -> Result<Region<Rw>, u32> {
        unimplemented!()
    }

    fn make_exec(&mut self, _region: Region<Rw>) -> Result<Region<Rx>, u32> {
        unimplemented!()
    }

    fn expected_abi_hash(&self) -> u64 {
        0
    }

    #[cfg(feature = "encryption")]
    fn unwrap_cek(
        &self,
        _key_id: u64,
        _wrapped: &[u8],
        _out_cek: &mut [u8; 32],
    ) -> Result<(), u32> {
        unimplemented!()
    }
}

fn main() {}
