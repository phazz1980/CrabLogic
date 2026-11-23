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

use crab_core::SheetId;

use crate::canvas::{Align, Canvas, Op, EXTRA, INK, TEXT, TEXT_S, THICK, THIN, WEAK};
use crate::lang::Lang;
use crate::tables::Tables;
use crate::xref::{TerminalXrefs, Xrefs};

/// The module M of GOST IEC 61082-1 5.7.3.2 in units of a quarter of a millimetre: 2.5 mm.
pub const M: f32 = 10.0;
/// Margin between the edge of the paper and the frame: 2.5 mm.
const MARGIN: f32 = 10.0;
/// Height of the strip with the column numbers at the top of the frame.
const RING: f32 = 14.0;
/// The title block at the bottom of the inner line: 6M.
const STAMP_H: f32 = 4.0 * M;

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
    columns: usize,
    inner: Rect,
}

impl Default for Page {
    fn default() -> Self {
        Self::new(Format::A3)
    }
}

impl Page {
    pub fn new(format: Format) -> Self {
        // The frame stands 5 mm from the edges and is divided into columns only: 8 on A4 (36 mm
        // each), 12 on A3 (34 mm).
        let (width, height, columns) = match format {
            Format::A3 => (1680.0, 1184.0, 12),
            Format::A4 => (1188.0, 838.0, 8),
        };
        Self {
            format,
            width,
            height,
            columns,
            inner: Rect {
                x: MARGIN,
                y: MARGIN,
                w: width - 2.0 * MARGIN,
                h: height - 2.0 * MARGIN,
            },
        }
    }

    fn column_width(&self) -> f32 {
        self.inner.w / self.columns as f32
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
        self.inner.y + RING + M
    }

    pub fn y1(&self) -> f32 {
        self.inner.y + self.inner.h - STAMP_H - M
    }

    /// Zone of a point of the sheet: the number of its column from the left (`3`). Points outside
    /// the frame are put in the nearest column.
    pub fn zone(&self, x: f32, _y: f32) -> String {
        let col = ((x - self.inner.x) / self.column_width()).floor();
        let col = (col.max(0.0) as usize).min(self.columns - 1);
        (col + 1).to_string()
    }

    /// The rectangle `(x, y, w, h)` of a column, over the whole height of the frame; `None` for a
    /// name the page does not have.
    pub fn zone_rect(&self, zone: &str) -> Option<(f32, f32, f32, f32)> {
        let col: usize = zone.parse().ok()?;
        if col == 0 || col > self.columns {
            return None;
        }
        let w = self.column_width();
        Some((
            self.inner.x + (col - 1) as f32 * w,
            self.inner.y,
            w,
            self.inner.h,
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
    /// The name of the drawing sheet (see `SheetId`). `None` for the title and the tables.
    pub id: Option<SheetId>,
    /// Typst calls of the body (see [`Canvas`]).
    pub body: String,
    /// The same drawing as operations, for the editor.
    pub ops: Vec<Op>,
    /// The symbols and the wires of the sheet and the operations they are drawn with.
    pub items: Vec<crate::scene::Item>,
}

impl Sheet {
    /// A sheet with the drawing of `canvas`; `id` is the name of a drawing sheet.
    pub fn new(id: Option<SheetId>, title: String, canvas: Canvas) -> Self {
        let (body, ops, items) = canvas.into_scene();
        Self {
            title,
            id,
            body,
            ops,
            items,
        }
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
    /// Where each terminal drawn on the sheets is (the other ends of the hand wires).
    pub terminal_at: TerminalXrefs,
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
            link_refs(&sheet.body, page, (i + 1, total))
        );
    }
    out
}

/// The captions of the references (`/3.C4`) of a sheet become links to the zone of the sheet they
/// name, so that a click in the PDF goes there. `current` is the number of the sheet and the
/// number of sheets; a reference to a sheet that is not there stays plain text.
fn link_refs(body: &str, page: &Page, (current, total): (usize, usize)) -> String {
    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        out.push_str(&link_line(line, page, (current, total)).unwrap_or_else(|| line.to_owned()));
        out.push('\n');
    }
    out
}

/// The line of a text call with its last argument, the caption, made a link; `None` if the line
/// is not a text call with a reference.
fn link_line(line: &str, page: &Page, (current, total): (usize, usize)) -> Option<String> {
    if !(line.starts_with("T(") || line.starts_with("TB(") || line.starts_with("TV(")) {
        return None;
    }
    let inner = line.strip_suffix(')')?;
    let end = inner.strip_suffix('"')?.len();
    // The opening quote of the caption: the last one that is not escaped.
    let bytes = inner.as_bytes();
    let start = (0..end)
        .rev()
        .find(|&i| bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\'))?;
    let (sheet, zone) = crate::xref::find_ref(&inner[start + 1..end])?;
    let target = sheet.unwrap_or(current);
    if target == 0 || target > total {
        return None;
    }
    let (x, y, _, _) = page.zone_rect(&zone)?;
    Some(format!(
        "{}link((page: {target}, x: {x:.1} * u, y: {y:.1} * u), {}))",
        &inner[..start],
        &inner[start..]
    ))
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
    let inner = page.inner;
    c.rect(inner.x, inner.y, inner.w, inner.h, THICK, INK);

    // Columns: the numbers in a strip at the top of the frame.
    let (right, bottom) = (inner.x + inner.w, inner.y + inner.h);
    let strip = inner.y + RING;
    c.line(&[(inner.x, strip), (right, strip)], THIN, INK);
    let step = page.column_width();
    for i in 0..page.columns {
        let x = inner.x + i as f32 * step;
        if i > 0 {
            c.line(&[(x, inner.y), (x, strip)], THIN, INK);
        }
        let label = (i + 1).to_string();
        c.text(
            x + step / 2.0,
            inner.y + RING / 2.0,
            &label,
            TEXT_S,
            Align::Center,
            INK,
        );
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
        c.text(*x, top + 10.0, caption, TEXT_S, Align::Left, WEAK);
        c.bold(*x, top + 26.0, value, TEXT, Align::Left, INK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_are_numbered_by_column() {
        for (format, last) in [(Format::A3, "12"), (Format::A4, "8")] {
            let p = Page::new(format);
            let i = p.inner;
            assert_eq!(p.zone(i.x + 1.0, i.y + 1.0), "1");
            assert_eq!(p.zone(i.x + i.w - 1.0, i.y + i.h - 1.0), last);
            // Outside the frame: the nearest column; the row does not matter.
            assert_eq!(p.zone(0.0, 0.0), "1");
            assert_eq!(p.zone(5000.0, 5000.0), last);
            assert_eq!(p.zone(i.x + 1.0, i.y + i.h - 1.0), "1");
        }
    }

    #[test]
    fn a_zone_has_a_rectangle_that_contains_its_points() {
        for format in [Format::A3, Format::A4] {
            let p = Page::new(format);
            for name in ["1", "3", "8"] {
                let (x, y, w, h) = p.zone_rect(name).unwrap();
                assert_eq!(p.zone(x + 1.0, y + 1.0), name);
                assert_eq!(p.zone(x + w - 1.0, y + h - 1.0), name);
            }
        }
        let p = Page::new(Format::A4);
        assert!(p.zone_rect("9").is_none() && p.zone_rect("0").is_none());
        assert!(p.zone_rect("").is_none() && p.zone_rect("C3").is_none());
        assert!(Page::new(Format::A3).zone_rect("12").is_some());
    }

    #[test]
    fn frame_has_minimal_margins_and_columns_of_the_sample() {
        for (format, columns, mm) in [(Format::A3, 12, 34.0), (Format::A4, 8, 36.0)] {
            let p = Page::new(format);
            assert_eq!(p.inner.x * 0.25, 2.5);
            assert_eq!(p.inner.y * 0.25, 2.5);
            assert_eq!(p.width - (p.inner.x + p.inner.w), MARGIN);
            assert_eq!(p.height - (p.inner.y + p.inner.h), MARGIN);
            assert_eq!(p.columns, columns);
            assert!((p.column_width() * 0.25 - mm).abs() < 0.6);
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
