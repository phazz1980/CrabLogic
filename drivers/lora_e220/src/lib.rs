//! LoRa E220 driver block. Plain Rust that also builds for the host (tests with a simulated
//! air); the entry point for the master is added with the blocks. Design: specs/lora-e220/plan.md.

// no_std on the board; on the host (tests) a normal library.
#![cfg_attr(all(not(test), target_arch = "wasm32"), no_std)]
// The module may not link `core::fmt` (a bounds-check panic pulls in a function table): no
// indexing, ranges, `unwrap` or `copy_from_slice` outside the tests.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic
    )
)]

pub mod config;
pub mod e220;
#[cfg(target_arch = "wasm32")]
mod entry;
pub mod frame;
pub mod master;
pub mod safe;
pub mod slave;

use frame::{Ty, MAX_VALUES};

/// The radio as the automata see it: whole frames in, whole frames out, never blocking.
pub trait Radio {
    /// Hands a frame to the modem; `false` when it cannot take it now (modem busy).
    fn send(&mut self, frame: &[u8]) -> bool;
    /// One received frame, if there is one (copied into `buf`, longer frames are the radio's to drop).
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize>;
}

/// Types of the values one side sends (at most [`MAX_VALUES`]), without a heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    ty: [Ty; MAX_VALUES],
    n: usize,
}

impl Layout {
    /// `None` if there are more than [`MAX_VALUES`] types.
    pub fn new(types: &[Ty]) -> Option<Self> {
        if types.len() > MAX_VALUES {
            return None;
        }
        let mut ty = [Ty::Bool; MAX_VALUES];
        safe::copy(&mut ty, types);
        Some(Self { ty, n: types.len() })
    }

    pub fn types(&self) -> &[Ty] {
        self.ty.get(..self.n).unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// Milliseconds from `since` to `now` on a wrapping 32-bit clock.
pub(crate) fn since(now: u32, since: u32) -> u32 {
    now.wrapping_sub(since)
}
