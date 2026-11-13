//! Panic-free slice access. The driver may not link `core::fmt` (a bounds-check panic message
//! pulls it in together with a function table, which driver modules must not have), so every
//! index goes through `get`: out-of-range reads give the default, writes are dropped.

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

/// Copies as many elements as both slices have.
pub fn copy<T: Copy>(dst: &mut [T], src: &[T]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = *s;
    }
}
