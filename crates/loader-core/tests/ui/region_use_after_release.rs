use loader_core::platform::{Region, Rw};

fn release_region(_region: Region<Rw>) {}

fn main() {
    let region = unsafe { Region::<Rw>::from_raw_parts(core::ptr::null_mut(), 0) };
    release_region(region);
    let _ = region.as_slice();
}
