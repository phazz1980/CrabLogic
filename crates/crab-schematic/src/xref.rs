//! Cross references: where on the sheets a device is drawn.

use std::collections::HashMap;
use std::fmt;

/// A place in the document: the number of the sheet and the zone on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xref {
    pub sheet: usize,
    /// Zone of the frame, the number of the column, e.g. `5` (see [`crate::doc::Page::zone`]).
    pub zone: String,
}

impl fmt::Display for Xref {
    /// `/2.3`: sheet 2, column 3 (GOST IEC 61082-1 5.8: the sheet after a solidus, the zone after a
    /// point).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}.{}", self.sheet, self.zone)
    }
}

impl Xref {
    /// The reference as read on sheet `current`: `/.3` for a column of the same sheet (the sheet
    /// may be left out, 5.8), `/2.3` for another sheet.
    pub fn from_sheet(&self, current: usize) -> String {
        if self.sheet == current {
            format!("/.{}", self.zone)
        } else {
            self.to_string()
        }
    }
}

/// The reference a caption ends with: `/2.3` (sheet 2, column 3), `/.3` (column 3 of the sheet
/// the caption is on, the sheet `None`) or `13 /2.3` (with the terminals before it).
pub fn find_ref(text: &str) -> Option<(Option<usize>, String)> {
    let token = text.split_whitespace().last()?;
    let (sheet, zone) = token.strip_prefix('/')?.split_once('.')?;
    let sheet = if sheet.is_empty() {
        None
    } else {
        Some(sheet.parse().ok()?)
    };
    (!zone.is_empty() && zone.chars().all(|c| c.is_ascii_digit())).then(|| (sheet, zone.to_owned()))
}

/// Places of devices by designation (`K1`).
pub type Xrefs = HashMap<String, Xref>;

/// Places of terminals by device and terminal (`K1`, `A2`): where each terminal is drawn on the
/// sheets, for the hand wires that end on another sheet.
pub type TerminalXrefs = HashMap<(String, String), Xref>;

#[cfg(test)]
mod tests {
    use super::find_ref;

    #[test]
    fn a_reference_is_read_from_the_end_of_a_caption() {
        assert_eq!(find_ref("/2.3"), Some((Some(2), "3".to_owned())));
        assert_eq!(find_ref("/.12"), Some((None, "12".to_owned())));
        assert_eq!(find_ref("13 /14.1"), Some((Some(14), "1".to_owned())));
    }

    #[test]
    fn other_captions_are_no_references() {
        for text in [
            "", "/", "/2", "/2.", "/2.c3", "/x.3", "2.3", "K1", "24V+", "/2.C", "/2.C3", "/2.3C",
        ] {
            assert_eq!(find_ref(text), None, "{text}");
        }
    }
}
