//! Portable parts of the Rust master firmware.
//!
//! The board binary is kept small and platform-specific, while the line
//! protocol is tested on the desktop. Protocol compatibility is intentional:
//! the editor already speaks this protocol to the C++ master.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod boot_guard;
pub mod protocol;
pub mod restart;
pub mod storage;
pub mod wamr;
