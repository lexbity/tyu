use crate::c;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Errno(pub i32);

impl Errno {
    pub fn last() -> Self {
        let value = unsafe { *c::__errno_location() };
        Self(value)
    }
}
