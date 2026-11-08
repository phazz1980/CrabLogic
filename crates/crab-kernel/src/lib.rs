//! `crab-kernel`: the module microkernel shared by the PC simulator and the board master
//! (`specs/wasm-kernel`). No GUI, no arduino-cli, no network; `no_std` + `alloc`.
//!
//! * [`handle`] — generational handles and the slot map behind them;
//! * [`bus`] — topics, mailboxes, loss and refusal counters;
//! * [`lease`] — outputs owned by a module, safe state on drop;
//! * [`abi`] — the one list of host imports with signatures and permissions;
//! * [`host`] — what the imports do ([`host::ModuleHost`]) and the board side ([`host::Devices`]);
//! * [`engine`] — the engine trait; [`wasmi`] (feature `engine-wasmi`) runs modules on the PC;
//! * [`kernel`] — modules, bus and leases together: load, scan, swap, unload.

#![cfg_attr(not(any(test, feature = "std")), no_std)]

extern crate alloc;

pub mod abi;
pub mod bus;
pub mod engine;
pub mod handle;
pub mod host;
pub mod kernel;
pub mod lease;
#[cfg(feature = "engine-wasmi")]
pub mod wasmi;

pub use handle::{Handle, SlotMap};
pub use host::{Val, Word};
pub use kernel::Kernel;
