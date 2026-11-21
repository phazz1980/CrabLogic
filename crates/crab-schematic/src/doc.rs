//! The document: landscape sheets with a frame, coordinate zones and a title block, written as
//! Typst markup.
//!
//! Layout after GOST IEC 61082-1-2014 (IEC 61082-1:2006): A3 landscape by default (the standard
//! recommends A3 for diagrams), A4 on request; module M = 2.5 mm = 10 units. The frame is an outer
//! and an inner line with a ring between them that carries the zones: columns `1`.. along the top
//! and the bottom, rows `A`.. (without `I` and `O`) down the sides, a step of 10M = 25 mm
//! (5.7.3.4). A place on a sheet is named in the cross references as `/3.C5` (sheet 3, zone C5),
//! see [`crate::xref`].

use std::fmt::Write as _;

use crate::canvas::{Align, Canvas, Op, EXTRA, INK, TEXT, TEXT_L, THICK, THIN, WEAK};
use crate::lang::Lang;
use crate::tables::Tables;
use crate::xref::Xrefs;

/// The module M of GOST IEC 61082-1 5.7.3.2 in units of a quarter of a millimetre: 2.5 mm.
pub const M: f32 = 10.0;
/// Step of the coordinate grid, 10M = 25 mm.
const ZONE: f32 = 10.0 * M;
/// Width of the ring with the zone marks between the outer and the inner line.
const RING: f32 = 20.0;
/// Letters of the rows: capital Latin letters without `I` and `O` (5.7.3.4).
const ROW_LETTERS: &[u8; 11] = b"ABCDEFGHJKL";
/// The title block at the bottom of the inner line: 6M.
const STAMP_H: f32 = 6.0 * M;

/// Size of the sheets (ISO 5457; the standard allows no extended sizes, 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// 420 × 297 mm: recommended for diagrams.
    #[default]
    A3,
    /// 297 × 210 mm.
    A4,
}

impl Format {
    /// `a3` or `a4`, any case.
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_lowercase().as_str() {
            "a3" => Some(Self::A3),
            "a4" => Some(Self::A4),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::A3 => "A3",
            Self::A4 => "A4",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// The geometry of a sheet: the frame, the zones and the free area of the body. All in units of a
/// quarter of a millimetre, from the top-left corner of the page.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub format: Format,
    /// Size of the page; a little less than the paper, so that the box never spills onto a page
    /// of its own.
    width: f32,
    height: f32,
    zones_x: usize,
    zones_y: usize,
    inner: Rect,
}

impl Default for Page {
    fn default() -> Self {
        Self::new(Format::A3)
    }
}

impl Page {
    pub fn new(format: Format) -> Self {
        // The inner line stands 20 mm from the left edge (binding margin) and holds whole zones.
        let (width, height, zones_x, zones_y) = match format {
            Format::A3 => (1680.0, 1184.0, 15, 11),
            Format::A4 => (1188.0, 838.0, 10, 7),
        };
        let (w, h) = (zones_x as f32 * ZONE, zones_y as f32 * ZONE);
        Self {
            format,
            width,
            height,
            zones_x,
            zones_y,
            inner: Rect {
                x: 80.0,
                y: ((height - h) / 2.0 / M).round() * M,
                w,
                h,
            },
        }
    }

    fn outer(&self) -> Rect {
        Rect {
            x: self.inner.x - RING,
            y: self.inner.y - RING,
            w: self.inner.w + 2.0 * RING,
            h: self.inner.h + 2.0 * RING,
        }
    }

    /// Free area of a sheet inside the inner line, above the title block: one module from the
    /// lines.
    pub fn x0(&self) -> f32 {
        self.inner.x + M
    }

    pub fn x1(&self) -> f32 {
        self.inner.x + self.inner.w - M
    }

    pub fn y0(&self) -> f32 {
        self.inner.y + M
    }

    pub fn y1(&self) -> f32 {
        self.inner.y + self.inner.h - STAMP_H - M
    }

    /// Zone of a point of the sheet: `C5` is the third row from the top and the fifth column from
    /// the left. Points outside the inner line are put in the nearest zone.
    pub fn zone(&self, x: f32, y: f32) -> String {
        let col = ((x - self.inner.x) / ZONE).floor();
        let row = ((y - self.inner.y) / ZONE).floor();
        let col = (col.max(0.0) as usize).min(self.zones_x - 1);
        let row = (row.max(0.0) as usize).min(self.zones_y - 1);
        format!("{}{}", ROW_LETTERS[row] as char, col + 1)
    }

    /// The rectangle `(x, y, w, h)` of a zone such as `C5`; `None` for a name the page does not
    /// have.
    pub fn zone_rect(&self, zone: &str) -> Option<(f32, f32, f32, f32)> {
        let mut chars = zone.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        let row = ROW_LETTERS
            .iter()
            .take(self.zones_y)
            .position(|&l| l as char == letter)?;
        let col: usize = chars.as_str().parse().ok()?;
        if col == 0 || col > self.zones_x {
            return None;
        }
        Some((
            self.inner.x + (col - 1) as f32 * ZONE,
            self.inner.y + row as f32 * ZONE,
            ZONE,
            ZONE,
        ))
    }

    fn paper(&self) -> &'static str {
        match self.format {
            Format::A3 => "a3",
            Format::A4 => "a4",
        }
    }
}

/// One sheet: its caption for the title block and the drawing of its body.
pub struct Sheet {
    pub title: String,
    /// Typst calls of the body (see [`Canvas`]).
    pub body: String,
    /// The same drawing as operations, for the editor.
    pub ops: Vec<Op>,
}

impl Sheet {
    /// A sheet with the drawing of `canvas`.
    pub fn new(title: String, canvas: Canvas) -> Self {
        let (body, ops) = canvas.into_parts();
        Self { title, body, ops }
    }
}

/// All the sheets of the documentation.
pub struct Document {
    /// Name of the project, for the title block.
    pub name: String,
    pub page: Page,
    pub sheets: Vec<Sheet>,
    /// Where each device of the power sheets is drawn (its contacts).
    pub power_at: Xrefs,
    /// Where each device of the control sheets is drawn (a coil, a sensor, a button).
    pub coil_at: Xrefs,
    /// The tables on the last sheets; also written as CSV (`crablogic panel --csv`).
    pub tables: Tables,
}

/// The Typst markup of the whole document; compile it with `crab_pdf::compile`.
pub fn typst_source(doc: &Document, lang: Lang) -> String {
    let page = &doc.page;
    let mut out = String::from(crab_pdf::PRELUDE);
    out.push_str(EXTRA);
    let _ = writeln!(
        out,
        "#set page(paper: \"{}\", flipped: true, margin: 0pt)",
        page.paper()
    );
    let total = doc.sheets.len();
    for (i, sheet) in doc.sheets.iter().enumerate() {
        if i > 0 {
            out.push_str("#pagebreak()\n");
        }
        let mut c = Canvas::new();
        frame(&mut c, doc, sheet, (i + 1, total), lang);
        let (w, h) = (page.width, page.height);
        let _ = write!(
            out,
            "#box(width: {w} * u, height: {h} * u, {{\n{}{}}})\n",
            c.finish(),
            sheet.body
        );
    }
    out
}

impl Document {
    /// The rectangle `(x, y, w, h)` of a zone of the sheets, see [`Page::zone_rect`].
    pub fn zone_rect(&self, zone: &str) -> Option<(f32, f32, f32, f32)> {
        self.page.zone_rect(zone)
    }

    /// The frame, the zones and the title block of the sheet number `index` (from 0) as operations.
    pub fn frame_ops(&self, index: usize, lang: Lang) -> Vec<Op> {
        let mut c = Canvas::new();
        if let Some(sheet) = self.sheets.get(index) {
            frame(&mut c, self, sheet, (index + 1, self.sheets.len()), lang);
        }
        c.into_parts().1
    }

    /// Size of the page in units of a quarter of a millimetre.
    pub fn page_size(&self) -> (f32, f32) {
        (self.page.width, self.page.height)
    }
}

/// The frame with its zones and the title block: project, caption of the sheet, number of the sheet.
fn frame(
    c: &mut Canvas,
    doc: &Document,
    sheet: &Sheet,
    (number, total): (usize, usize),
    lang: Lang,
) {
    let page = &doc.page;
    let (outer, inner) = (page.outer(), page.inner);
    c.rect(outer.x, outer.y, outer.w, outer.h, THIN, INK);
    c.rect(inner.x, inner.y, inner.w, inner.h, THICK, INK);

    // Zones: the numbers at the top and the bottom, the letters at the sides.
    let (right, bottom) = (inner.x + inner.w, inner.y + inner.h);
    for i in 0..page.zones_x {
        let x = inner.x + i as f32 * ZONE;
        if i > 0 {
            c.line(&[(x, outer.y), (x, inner.y)], THIN, INK);
            c.line(&[(x, bottom), (x, outer.y + outer.h)], THIN, INK);
        }
        let label = (i + 1).to_string();
        let mid = x + ZONE / 2.0;
        c.text(mid, outer.y + RING / 2.0, &label, TEXT, Align::Center, INK);
        c.text(mid, bottom + RING / 2.0, &label, TEXT, Align::Center, INK);
    }
    for (j, letter) in ROW_LETTERS.iter().take(page.zones_y).enumerate() {
        let y = inner.y + j as f32 * ZONE;
        if j > 0 {
            c.line(&[(outer.x, y), (inner.x, y)], THIN, INK);
            c.line(&[(right, y), (outer.x + outer.w, y)], THIN, INK);
        }
        let label = (*letter as char).to_string();
        let mid = y + ZONE / 2.0;
        c.text(outer.x + RING / 2.0, mid, &label, TEXT, Align::Center, INK);
        c.text(right + RING / 2.0, mid, &label, TEXT, Align::Center, INK);
    }

    // The title block: project, caption, format and number of the sheet.
    let top = bottom - STAMP_H;
    c.line(&[(inner.x, top), (right, top)], THICK, INK);
    let (split_a, split_b, split_c) = (right - 500.0, right - 280.0, right - 160.0);
    for x in [split_a, split_b, split_c] {
        c.line(&[(x, top), (x, bottom)], THIN, INK);
    }
    let cells = [
        (
            inner.x + M,
            lang.pick("Проект", "Project"),
            doc.name.clone(),
        ),
        (
            split_a + M,
            lang.pick("Наименование", "Title"),
            sheet.title.clone(),
        ),
        (
            split_b + M,
            lang.pick("Формат", "Format"),
            page.format.name().to_owned(),
        ),
        (
            split_c + M,
            lang.pick("Лист", "Sheet"),
            format!("{number} / {total}"),
        ),
    ];
    for (x, caption, value) in &cells {
        c.text(*x, top + 15.0, caption, TEXT, Align::Left, WEAK);
        c.bold(*x, top + 40.0, value, TEXT_L, Align::Left, INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_are_numbered_across_and_lettered_down() {
        let p = Page::new(Format::A3);
        let i = p.inner;
        assert_eq!(p.zone(i.x + 1.0, i.y + 1.0), "A1");
        assert_eq!(p.zone(i.x + i.w - 1.0, i.y + i.h - 1.0), "L15");
        assert_eq!(p.zone(i.x + i.w / 2.0 + 1.0, i.y + i.h / 2.0 + 1.0), "F8");
        // Outside the inner line: the nearest zone.
        assert_eq!(p.zone(0.0, 0.0), "A1");
        assert_eq!(p.zone(5000.0, 5000.0), "L15");
        assert_eq!(Page::new(Format::A4).zone(5000.0, 5000.0), "G10");
    }

    #[test]
    fn a_zone_has_a_rectangle_that_contains_its_points() {
        for format in [Format::A3, Format::A4] {
            let p = Page::new(format);
            for name in ["A1", "C5", "G10"] {
                let (x, y, w, h) = p.zone_rect(name).unwrap();
                assert_eq!(p.zone(x + 1.0, y + 1.0), name);
                assert_eq!(p.zone(x + w - 1.0, y + h - 1.0), name);
            }
        }
        let p = Page::new(Format::A4);
        assert!(p.zone_rect("L1").is_none() && p.zone_rect("I3").is_none());
        assert!(p.zone_rect("A0").is_none() && p.zone_rect("A11").is_none());
        assert!(p.zone_rect("").is_none() && p.zone_rect("5C").is_none());
    }

    #[test]
    fn grid_follows_the_standard() {
        // 5.7.3.4: the step of the coordinate grid is 10M, 16M or 20M; here 10M = 25 mm.
        assert_eq!(ZONE, 10.0 * M);
        assert_eq!(ZONE * 0.25, 25.0);
        // The rows are lettered without I and O.
        assert!(!ROW_LETTERS.contains(&b'I') && !ROW_LETTERS.contains(&b'O'));
        for format in [Format::A3, Format::A4] {
            let p = Page::new(format);
            assert!(p.zones_y <= ROW_LETTERS.len());
            // The zones tile the inner line exactly.
            assert_eq!(p.inner.w, p.zones_x as f32 * ZONE);
            assert_eq!(p.inner.h, p.zones_y as f32 * ZONE);
            // The frame lies on the sheet, the inner line starts on the grid of the module.
            let o = p.outer();
            assert!(o.x >= 0.0 && o.y >= 0.0);
            assert!(o.x + o.w < p.width && o.y + o.h < p.height);
            assert_eq!(p.inner.x % M, 0.0);
            assert_eq!(p.inner.y % M, 0.0);
        }
        // 5.7.3.2: M = 2.5 mm.
        assert_eq!(M * 0.25, 2.5);
    }

    #[test]
    fn body_lies_inside_the_inner_line_above_the_title_block() {
        for format in [Format::A3, Format::A4] {
            let p = Page::new(format);
            let i = p.inner;
            assert!(p.x0() > i.x && p.x1() < i.x + i.w);
            assert!(p.y0() > i.y && p.y1() < i.y + i.h - STAMP_H);
        }
    }

    #[test]
    fn line_widths_and_text_sizes_are_from_the_series() {
        use crate::canvas::{TEXT, TEXT_L, TEXT_XL, THICK, THIN};
        // 5.10: 0.25 and 0.5 mm (the ratio of two widths is at least 2:1).
        for width in [THIN, THICK] {
            assert!([0.2, 0.25, 0.35, 0.5, 0.7, 1.0].contains(&(width * 0.25)));
        }
        const { assert!(THICK / THIN >= 2.0) };
        // 5.11: heights of the capitals 2.5, 3.5 and 5 mm; the size of the font is 1.7 times that.
        for (size, cap) in [(TEXT, 2.5), (TEXT_L, 3.5), (TEXT_XL, 5.0)] {
            assert!(
                (size * 0.25 / crate::canvas::FONT_PER_CAP - cap).abs() < 0.1,
                "{size}"
            );
        }
    }
}
