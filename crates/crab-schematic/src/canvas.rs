//! Typst calls of one sheet. Coordinates are in units of a quarter of a millimetre, y down, from
//! the top-left corner of the page; the Typst helpers are those of `crab_pdf::PRELUDE` plus
//! [`EXTRA`].

use std::fmt::Write as _;

use crab_pdf::quote;

pub type Rgb = (u8, u8, u8);

pub const INK: Rgb = (0x20, 0x20, 0x20);
pub const WEAK: Rgb = (0x70, 0x70, 0x70);
/// Protective earth.
pub const EARTH: Rgb = (0x2e, 0x8b, 0x3a);

/// 24 V circuits.
pub const LOW: Rgb = (0x2a, 0x5d, 0xb0);

/// Line widths from the series of GOST IEC 61082-1 5.10 (0.25 and 0.5 mm: the ratio of two widths
/// of one kind of line is at least 2:1).
pub const THIN: f32 = 1.0;
pub const THICK: f32 = 2.0;
/// Sizes of the font. The standard (5.11) gives the height of the capital letters: 2.5, 3.5 and
/// 5 mm. The drawing font OpenGost Type B has capitals of 1000/1700 of its size, so the size of
/// the Typst font is that height times 1.7.
pub const TEXT: f32 = 17.0;
pub const TEXT_L: f32 = 24.0;
pub const TEXT_XL: f32 = 34.0;
/// The size of the font per unit of the height of the capitals.
#[cfg(test)]
pub const FONT_PER_CAP: f32 = 1.7;
/// Width of a character of the text, for the rooms the layout keeps free (the font is narrow,
/// about 0.4 of its size on average; the room is kept with some margin).
pub const CHAR_W: f32 = 0.5 * TEXT;
/// Radius of the dot of a terminal or a junction.
pub const DOT: f32 = 2.5;

/// Typst helpers that `crab_pdf::PRELUDE` does not have: dashes, outlines, bold text.
pub const EXTRA: &str = r##"#set text(font: GOST_FONTS, lang: "ru")
#let T(x, y, w, al, sz, c, s) = place(top + left, dx: x * u, dy: (y - sz) * u,
  box(width: w * u, height: 2 * sz * u, align(al + horizon,
    text(size: sz * u, top-edge: "cap-height", bottom-edge: "baseline", fill: rgb(c), s))))
#let WD(c, wd, ..p) = {
  let p = p.pos()
  for i in range(p.len() - 1) {
    place(top + left, line(start: (p.at(i).at(0) * u, p.at(i).at(1) * u),
      end: (p.at(i + 1).at(0) * u, p.at(i + 1).at(1) * u),
      stroke: stroke(paint: rgb(c), thickness: wd * u, cap: "butt", dash: "dashed")))
  }
}
#let RO(x, y, w, h, c, wd) = place(top + left, dx: x * u, dy: y * u,
  rect(width: w * u, height: h * u, fill: none, stroke: wd * u + rgb(c)))
#let CO(x, y, r, c, wd) = place(top + left, dx: (x - r) * u, dy: (y - r) * u,
  circle(radius: r * u, fill: none, stroke: wd * u + rgb(c)))
#let TB(x, y, w, al, sz, c, s) = place(top + left, dx: x * u, dy: (y - sz) * u,
  box(width: w * u, height: 2 * sz * u, align(al + horizon,
    text(size: sz * u, top-edge: "cap-height", bottom-edge: "baseline", stroke: 0.3 * u + rgb(c), fill: rgb(c), s))))
#let TV(x, y, w, al, sz, c, s) = place(top + left, dx: (x - sz) * u, dy: y * u,
  rotate(-90deg, origin: top + left,
    box(width: w * u, height: 2 * sz * u, align(al + horizon,
      text(size: sz * u, top-edge: "cap-height", bottom-edge: "baseline", fill: rgb(c), s)))))
"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// One drawing operation of a sheet, in units of a quarter of a millimetre (y down). The sheets
/// keep the same drawing as the Typst calls, so that the editor can show a sheet without making a
/// PDF.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A polyline; `dashed` for the mechanical links.
    Line {
        points: Vec<(f32, f32)>,
        width: f32,
        dashed: bool,
        rgb: Rgb,
    },
    /// Outline of a rectangle.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        width: f32,
        rgb: Rgb,
    },
    /// Outline of a circle.
    Circle {
        x: f32,
        y: f32,
        r: f32,
        width: f32,
        rgb: Rgb,
    },
    /// Filled dot.
    Dot { x: f32, y: f32, r: f32, rgb: Rgb },
    /// Text vertically centred on `y`; `x` is the left edge, the middle or the right edge by
    /// `align`. `vertical` text is turned by 90° counter-clockwise (read from the right) and `x`
    /// is its middle line.
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        align: Align,
        bold: bool,
        vertical: bool,
        rgb: Rgb,
    },
}

/// Drawing of one sheet: the text of the Typst calls and the same drawing as a list of operations.
#[derive(Default)]
pub struct Canvas {
    out: String,
    ops: Vec<Op>,
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> String {
        self.out
    }

    /// The Typst calls and the operations.
    pub fn into_parts(self) -> (String, Vec<Op>) {
        (self.out, self.ops)
    }

    fn points(&mut self, points: &[(f32, f32)]) {
        for (x, y) in points {
            let _ = write!(self.out, ", ({x:.1}, {y:.1})");
        }
        self.out.push_str(")\n");
    }

    pub fn line(&mut self, points: &[(f32, f32)], width: f32, rgb: Rgb) {
        self.ops.push(Op::Line {
            points: points.to_vec(),
            width,
            dashed: false,
            rgb,
        });
        let _ = write!(self.out, "W(\"{}\", {width:.2}", hex(rgb));
        self.points(points);
    }

    pub fn dashed(&mut self, points: &[(f32, f32)], width: f32, rgb: Rgb) {
        self.ops.push(Op::Line {
            points: points.to_vec(),
            width,
            dashed: true,
            rgb,
        });
        let _ = write!(self.out, "WD(\"{}\", {width:.2}", hex(rgb));
        self.points(points);
    }

    /// Outline of a rectangle.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, rgb: Rgb) {
        self.ops.push(Op::Rect {
            x,
            y,
            w,
            h,
            width,
            rgb,
        });
        let _ = writeln!(
            self.out,
            "RO({x:.1}, {y:.1}, {w:.1}, {h:.1}, \"{}\", {width:.2})",
            hex(rgb)
        );
    }

    /// Outline of a circle.
    pub fn circle(&mut self, x: f32, y: f32, r: f32, width: f32, rgb: Rgb) {
        self.ops.push(Op::Circle {
            x,
            y,
            r,
            width,
            rgb,
        });
        let _ = writeln!(
            self.out,
            "CO({x:.1}, {y:.1}, {r:.1}, \"{}\", {width:.2})",
            hex(rgb)
        );
    }

    /// Filled dot (a junction of wires).
    pub fn dot(&mut self, x: f32, y: f32, r: f32, rgb: Rgb) {
        self.ops.push(Op::Dot { x, y, r, rgb });
        let _ = writeln!(self.out, "D({x:.1}, {y:.1}, {r:.1}, \"{}\")", hex(rgb));
    }

    /// Text vertically centred on `y`.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, align: Align, rgb: Rgb) {
        self.put(x, y, text, size, align, rgb, "T");
    }

    pub fn bold(&mut self, x: f32, y: f32, text: &str, size: f32, align: Align, rgb: Rgb) {
        self.put(x, y, text, size, align, rgb, "TB");
    }

    /// Text turned by 90° counter-clockwise, read from the right (5.2): it runs along a vertical
    /// wire. `x` is the middle line of the text; `Left` starts at `y` and goes up, `Center` is
    /// centred on `y`.
    pub fn vtext(&mut self, x: f32, y: f32, text: &str, size: f32, align: Align, rgb: Rgb) {
        self.ops.push(Op::Text {
            x,
            y,
            text: text.to_owned(),
            size,
            align,
            bold: false,
            vertical: true,
            rgb,
        });
        let w = text.chars().count() as f32 * size + 40.0;
        let (y, al) = match align {
            Align::Left => (y, "left"),
            Align::Center => (y + w * 0.5, "center"),
            Align::Right => (y + w, "right"),
        };
        let _ = writeln!(
            self.out,
            "TV({x:.1}, {y:.1}, {w:.1}, {al}, {size}, \"{}\", {})",
            hex(rgb),
            quote(text)
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, x: f32, y: f32, text: &str, size: f32, align: Align, rgb: Rgb, call: &str) {
        self.ops.push(Op::Text {
            x,
            y,
            text: text.to_owned(),
            size,
            align,
            bold: call == "TB",
            vertical: false,
            rgb,
        });
        // The box is wide enough that the text never wraps.
        let w = text.chars().count() as f32 * size + 40.0;
        let (x, al) = match align {
            Align::Left => (x, "left"),
            Align::Center => (x - w * 0.5, "center"),
            Align::Right => (x - w, "right"),
        };
        let _ = writeln!(
            self.out,
            "{call}({x:.1}, {y:.1}, {w:.1}, {al}, {size}, \"{}\", {})",
            hex(rgb),
            quote(text)
        );
    }
}
