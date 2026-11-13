//! Display driver block: OLED (SSD1306 / SH1106) and character LCD (HD44780 behind a PCF8574),
//! screens built in the designer plus the operator menu. The sheet module hands the screens and
//! the menu over as a `crab-ui-format` blob (`crab.config`). All drawing and menu logic is plain
//! Rust that also builds for the host (tests); only `entry` talks to the master.
//! Input/output index contract: specs/display/plan.md, «Договорённость генератора и драйвера».

#![cfg_attr(not(test), no_std)]

pub mod driver;
pub mod fb;
pub mod font;
pub mod lcd;
pub mod menu;
pub mod oled;
pub mod safe;
pub mod text;

#[cfg(all(target_arch = "wasm32", any(feature = "oled", feature = "lcd")))]
mod entry;

#[cfg(test)]
mod tests;
