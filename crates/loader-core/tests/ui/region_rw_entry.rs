use loader_core::platform::{Region, Rw};

fn main() {
    let region = unsafe { Region::<Rw>::from_raw_parts(core::ptr::null_mut(), 0) };
    let _ = region.entry(0);
}
