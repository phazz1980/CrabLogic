//! Panic-free slice access (as in `drivers/display`): the module may not link `core::fmt`, which
//! a bounds-check panic would pull in with a function table, so every index goes through `get`:
//! an out-of-range read gives the default, a write is dropped.

pub trait Safe<T: Copy + Default> {
    /// Element `i`, or the default value.
    fn rd(&self, i: usize) -> T;
    /// Stores `v` at `i` when it exists.
    fn wr(&mut self, i: usize, v: T);
    /// The first `n` elements (all of them when there are fewer).
    fn part(&self, n: usize) -> &[T];
    /// Elements `a..b`, empty when out of range.
    fn span(&self, a: usize, b: usize) -> &[T];
}

impl<T: Copy + Default> Safe<T> for [T] {
    fn rd(&self, i: usize) -> T {
        self.get(i).copied().unwrap_or_default()
    }

    fn wr(&mut self, i: usize, v: T) {
        if let Some(x) = self.get_mut(i) {
            *x = v;
        }
    }

    fn part(&self, n: usize) -> &[T] {
        self.get(..n).unwrap_or(self)
    }

    fn span(&self, a: usize, b: usize) -> &[T] {
        self.get(a..b).unwrap_or(&[])
    }
}

/// Copies as many elements as both slices have (never panics, unlike `copy_from_slice`).
pub fn copy<T: Copy>(dst: &mut [T], src: &[T]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = *s;
    }
}
