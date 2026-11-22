//! A bus of a sheet and its ends towards the other sheets (`specs/panel-aux`, task 4).
//!
//! A bus that goes on to another sheet runs to the right frame and ends with its name above the
//! end and the reference `/sheet.zone` after it; the sheet it comes to starts it with the
//! reference of the previous sheet before the line and the name above the start.

use crate::canvas::{Align, Canvas, Rgb, TEXT};
use crate::doc::Page;
use crate::xref::Xref;

/// How far from the right edge of the frame a bus that goes on ends, for the reference.
pub const TAIL: f32 = 70.0;
/// The name above an end of the line.
const NAME_DY: f32 = 9.0;

/// Where a bus is on another sheet: the sheet number (counted from the first of the document),
/// ready for the zone to be worked out here.
#[derive(Clone, Copy)]
pub struct Elsewhere {
    pub sheet: usize,
}

/// The way a bus is drawn.
pub struct Style {
    pub width: f32,
    pub rgb: Rgb,
    /// The name is bold (control buses) or plain.
    pub bold: bool,
    /// The line comes up from this height and turns into the bus at `start` (the entry of the
    /// lines on the first power sheet); its name is written where it comes in, not here.
    pub riser: Option<f32>,
}

/// Draws the bus `name` at height `y` from `start` to `end`. `prev` / `next` are the sheets
/// where the same bus comes from / goes on to.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    page: &Page,
    y: f32,
    (start, end): (f32, f32),
    name: &str,
    style: &Style,
    prev: Option<Elsewhere>,
    next: Option<Elsewhere>,
) {
    let stop = if next.is_some() {
        end.max(page.x1() - TAIL)
    } else {
        end
    };
    match style.riser {
        Some(from) => c.line(
            &[(start, from), (start, y), (stop, y)],
            style.width,
            style.rgb,
        ),
        None => c.line(&[(start, y), (stop, y)], style.width, style.rgb),
    }
    let label = |c: &mut Canvas, x: f32, y: f32, text: &str, align: Align| {
        if style.bold {
            c.bold(x, y, text, TEXT, align, style.rgb);
        } else {
            c.text(x, y, text, TEXT, align, style.rgb);
        }
    };
    match prev {
        None if style.riser.is_some() => {}
        None => label(c, page.x0() + 2.0, y, name, Align::Left),
        Some(from) => {
            // Where the bus left the previous sheet: at its end by the right frame.
            let at = Xref {
                sheet: from.sheet,
                zone: page.zone(page.x1() - TAIL, y),
            };
            label(c, page.x0() + 2.0, y, &at.to_string(), Align::Left);
            label(c, start, y - NAME_DY, name, Align::Left);
        }
    }
    if let Some(to) = next {
        let at = Xref {
            sheet: to.sheet,
            zone: page.zone(start, y),
        };
        label(c, stop, y - NAME_DY, name, Align::Right);
        label(c, stop + 3.0, y, &at.to_string(), Align::Left);
    }
}
