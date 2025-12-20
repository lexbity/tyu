use crate::c;

#[derive(Clone, Copy)]
pub struct RawArgs {
    argc: isize,
    argv: *const *const c::c_char,
}

impl RawArgs {
    pub const unsafe fn new(argc: isize, argv: *const *const c::c_char) -> Self {
        Self { argc, argv }
    }

    pub fn len(&self) -> usize {
        if self.argc <= 0 {
            0
        } else if self.argc > 4096 {
            4096
        } else {
            self.argc as usize
        }
    }

    pub fn get(&self, index: usize) -> Option<*const c::c_char> {
        if index >= self.len() {
            return None;
        }
        Some(unsafe { *self.argv.add(index) })
    }

    pub fn iter(&self) -> RawArgsIter<'_> {
        RawArgsIter {
            args: *self,
            index: 0,
            _marker: core::marker::PhantomData,
        }
    }
}

pub struct RawArgsIter<'a> {
    args: RawArgs,
    index: usize,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> Iterator for RawArgsIter<'a> {
    type Item = *const c::c_char;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.args.get(self.index)?;
        self.index += 1;
        Some(value)
    }
}
