//! Cross references: where on the sheets a device is drawn.

use std::collections::HashMap;
use std::fmt;

/// A place in the document: the number of the sheet and the zone on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xref {
    pub sheet: usize,
    /// Zone of the frame, e.g. `C5` (see [`crate::doc::zone`]).
    pub zone: String,
}

impl fmt::Display for Xref {
    /// `/2.C3`: sheet 2, zone C3 (GOST IEC 61082-1 5.8: the sheet after a solidus, the zone after a
    /// point).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}.{}", self.sheet, self.zone)
    }
}

impl Xref {
    /// The reference as read on sheet `current`: `/.C3` for a zone of the same sheet (the sheet
    /// may be left out, 5.8), `/2.C3` for another sheet.
    pub fn from_sheet(&self, current: usize) -> String {
        if self.sheet == current {
            format!("/.{}", self.zone)
        } else {
            self.to_string()
        }
    }
}

/// Places of devices by designation (`K1`).
pub type Xrefs = HashMap<String, Xref>;
