use core::marker::PhantomData;
use core::mem::MaybeUninit;

pub struct FixedVec<T, const N: usize> {
    len: usize,
    data: [MaybeUninit<T>; N],
}

impl<T, const N: usize> Default for FixedVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
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

    #[allow(clippy::result_unit_err)]
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

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        if idx >= self.len {
            return None;
        }
        Some(unsafe { self.data[idx].assume_init_mut() })
    }

    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter { v: self, i: 0 }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, T, N> {
        IterMut {
            ptr: self.data.as_mut_ptr(),
            len: self.len,
            i: 0,
            _marker: PhantomData,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(self) -> IntoIter<T, N> {
        IntoIter { v: self, i: 0 }
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

pub struct IterMut<'a, T, const N: usize> {
    ptr: *mut MaybeUninit<T>,
    len: usize,
    i: usize,
    _marker: PhantomData<&'a mut T>,
}

pub struct IntoIter<T, const N: usize> {
    v: FixedVec<T, N>,
    i: usize,
}

impl<T, const N: usize> Iterator for IntoIter<T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.v.len {
            return None;
        }
        let idx = self.i;
        self.i += 1;
        Some(unsafe { self.v.data[idx].assume_init_read() })
    }
}

impl<T, const N: usize> Drop for IntoIter<T, N> {
    fn drop(&mut self) {
        while self.i < self.v.len {
            unsafe { self.v.data[self.i].assume_init_drop() };
            self.i += 1;
        }
        self.v.len = 0;
    }
}

impl<'a, T, const N: usize> Iterator for IterMut<'a, T, N> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.len {
            return None;
        }
        let idx = self.i;
        self.i += 1;
        // Safety: `idx < len` and the slot at `idx` has been written.
        // Each index is yielded exactly once, so the returned `&mut T` does not alias.
        Some(unsafe { &mut *self.ptr.add(idx).as_mut().unwrap_unchecked().as_mut_ptr() })
    }
}
