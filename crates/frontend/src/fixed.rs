use core::mem::MaybeUninit;

pub struct FixedVec<T, const N: usize> {
    len: usize,
    data: [MaybeUninit<T>; N],
}

impl<T, const N: usize> FixedVec<T, N> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            data: [const { MaybeUninit::uninit() }; N],
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, value: T) -> Result<(), ()> {
        if self.len >= N {
            return Err(());
        }
        self.data[self.len].write(value);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        if idx >= self.len {
            return None;
        }
        Some(unsafe { self.data[idx].assume_init_ref() })
    }

    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter { v: self, i: 0 }
    }
}

impl<T, const N: usize> Drop for FixedVec<T, N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            unsafe { self.data[i].assume_init_drop() };
        }
    }
}

pub struct Iter<'a, T, const N: usize> {
    v: &'a FixedVec<T, N>,
    i: usize,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.v.get(self.i)?;
        self.i += 1;
        Some(item)
    }
}

