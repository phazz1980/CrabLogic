//! Compact binary description of one display's screens, widgets and menu.
//!
//! The sheet module hands this blob to the display driver through `crab.config`.
//! Everything is little-endian; `str` is `len:u8` + UTF-8 bytes (at most 255 bytes);
//! "none" for optional `u16` fields is `0xFFFF`.
//!
//! ```text
//! Config   := version:u8(=3; 1 and 2 are still read) display:u8 n_screens:u8 flags:u8(bit0 = menu) Screen* Menu?
//! Screen   := body_len:u16 n_widgets:u8 Widget*          (body_len counts n_widgets + widgets)
//! Widget   := kind:u8 x:u8 y:u8 w:u8 h:u8 font:u8 input:u16 payload
//!   kind 0 Text : str                      kind 3 Led : -
//!   kind 1 Value: format:u8 decimals:u8 unit:str
//!   kind 2 Bar  : min:f32 max:f32          kind 4 Icon: id:u8
//!   kind 5 String: -   (a string input slot: the text is clipped to the width `w`)
//! Menu     := timeout_s:u16 font:u8 hints:u8 n_items:u16 Item*
//!   font: 8 or 16 (item font on the OLED); hints: 0/1 (soft-key bar).
//! Item     := kind:u8 parent:u16 var_slot:u16 vflags:u8(bit0 = the variable is `float`) caption:str payload
//!   kind 0 Submenu: -        kind 3 Choice: n:u8 str*n
//!   kind 1 Number : min:i32 max:i32 step:i32 decimals:u8 unit:str
//!   kind 2 Bool   : -        kind 4 Action: -        kind 5 View: -
//! display: 0 Ssd1306_128x64, 1 Ssd1306_128x32, 2 Sh1106_128x64, 3 Lcd1602, 4 Lcd2004
//! ```
//!
//! Version 3 (`specs/no-f64`): the numbers of a menu item (`min`, `max`, `step`) are `i32` in
//! units of `10^-decimals` (`0.5` with one decimal is `5`), so the driver needs no floating
//! point for the menu. `vflags` tells the driver which import reads the item's variable cell
//! (`input_f` for `float`, `input_i` otherwise). Bar limits stay `f32` (its input is `float`).
//! Versions 1 and 2 are still read: their `f32` limits are converted to the scaled integers
//! with `f32` arithmetic (no `f64`), a number item with decimals counts as a `float` variable
//! (old blobs have no `vflags`), version 1 has no menu `font`/`hints` (16, on).
//!
//! Slots of widget inputs: numeric widgets (`Value`, `Bar`, `Led`) take `0..Wn` in screen order,
//! string widgets (`String`) continue with `Wn..Wn + Ws`; the driver adds its own input ports.
//!
//! A menu item's `parent` must be the index of an earlier item (so the tree is acyclic).
//! The PC side (`alloc` feature) builds [`Config`] and calls [`Config::encode`]; the
//! driver side uses the zero-copy, allocation-free, panic-free [`ConfigView::parse`].
#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(test)]
extern crate std;

mod view;
pub use view::*;

#[cfg(feature = "alloc")]
mod model;
#[cfg(feature = "alloc")]
pub use model::*;

/// Current format version (first byte of the blob).
pub const VERSION: u8 = 3;
/// Oldest readable version (1 and 2 have `f32` menu limits, converted while reading).
pub const MIN_VERSION: u8 = 1;
/// Marker for an absent optional `u16`.
pub const NONE_U16: u16 = 0xFFFF;

/// Display kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
#[allow(non_camel_case_types)]
pub enum DisplayKind {
    Ssd1306_128x64 = 0,
    Ssd1306_128x32 = 1,
    Sh1106_128x64 = 2,
    Lcd1602 = 3,
    Lcd2004 = 4,
}

impl DisplayKind {
    /// Decode from the wire byte.
    pub fn from_u8(v: u8) -> Option<DisplayKind> {
        Some(match v {
            0 => DisplayKind::Ssd1306_128x64,
            1 => DisplayKind::Ssd1306_128x32,
            2 => DisplayKind::Sh1106_128x64,
            3 => DisplayKind::Lcd1602,
            4 => DisplayKind::Lcd2004,
            _ => return None,
        })
    }
}

/// Decoding error. The reader never panics; malformed input ends up here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// Input ended too early.
    Truncated,
    /// First byte is not [`VERSION`].
    BadVersion,
    /// Unknown display, widget or item kind, or reserved flag bits.
    BadKind,
    /// Text is not valid UTF-8.
    BadUtf8,
    /// Menu parent does not point to an earlier item.
    BadParent,
    /// Bytes left after the last structure.
    Trailing,
}

#[cfg(feature = "std")]
impl core::fmt::Display for FormatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            FormatError::Truncated => "truncated config",
            FormatError::BadVersion => "unsupported config version",
            FormatError::BadKind => "unknown kind in config",
            FormatError::BadUtf8 => "invalid UTF-8 in config",
            FormatError::BadParent => "bad menu parent",
            FormatError::Trailing => "trailing bytes in config",
        })
    }
}

#[cfg(feature = "std")]
impl core::error::Error for FormatError {}

#[cfg(test)]
mod tests;
