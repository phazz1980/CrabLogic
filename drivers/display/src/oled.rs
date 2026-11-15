//! OLED side: widget drawing and the SSD1306 / SH1106 wire protocol.

use crate::fb::{Fb, PAGE_BYTES};
use crate::font::{glyph, ICON_BASE};
use crate::safe::{copy, Safe};
use crate::text::{bar_fill, fmt_value, scaled_f32, Txt};
use crab_ui_format::{DisplayKind, WidgetKindView, WidgetView};

/// Pages (rows of 8 pixels) of the display.
pub fn pages(kind: DisplayKind) -> usize {
    if kind == DisplayKind::Ssd1306_128x32 {
        4
    } else {
        8
    }
}

/// Column offset of the first visible column (SH1106 has 132 columns, the glass shows 2..130).
pub fn column_offset(kind: DisplayKind) -> u8 {
    if kind == DisplayKind::Sh1106_128x64 {
        2
    } else {
        0
    }
}

/// Draws one widget. `v` is the cell of its input (`f32` bits for a value or a bar, an `i32` for
/// a LED, 0 when it has no input), `s` the text of a string widget.
pub fn draw_widget(fb: &mut Fb, w: &WidgetView, v: u32, s: &str) {
    let (x, y) = (w.x as i32, w.y as i32);
    let scale = if w.font >= 16 { 2 } else { 1 };
    match w.kind {
        WidgetKindView::Text { text } => {
            fb.text(x, y, text, scale);
        }
        WidgetKindView::Value {
            format,
            decimals,
            unit,
        } => {
            let mut t = Txt::new();
            fmt_value(&mut t, scaled_f32(v, decimals), format, decimals, unit);
            fb.chars(x, y, t.chars(), scale);
        }
        WidgetKindView::Bar { min, max } => {
            let (bw, bh) = (w.w as i32, w.h as i32);
            if bw < 3 || bh < 3 {
                return;
            }
            fb.rect(x, y, bw, bh);
            let fill = bar_fill(f32::from_bits(v), min, max, bw - 2);
            fb.fill_rect(x + 1, y + 1, fill, bh - 2, true);
        }
        WidgetKindView::Led => {
            let sw = if w.w > 0 { w.w as i32 } else { 8 };
            let sh = if w.h > 0 { w.h as i32 } else { sw };
            // A round indicator, as in the constructor's preview.
            fb.ellipse(x, y, sw, sh, (v as i32) > 0);
        }
        WidgetKindView::Icon { id } => {
            fb.glyph(x, y, &glyph(ICON_BASE + id as u16), scale, true);
        }
        WidgetKindView::String => {
            // As many characters as fit the width (a width of 0 means up to the screen edge).
            let adv = 6 * scale;
            let mut t = Txt::new();
            t.push_str(s);
            let fit = if w.w == 0 {
                t.len()
            } else {
                ((w.w as i32 + scale) / adv).max(0) as usize
            };
            fb.chars(
                x,
                y,
                t.chars().get(..t.len().min(fit)).unwrap_or(&[]),
                scale,
            );
        }
    }
}

/// Initialisation commands (after the control byte 0x00).
pub fn init_commands(kind: DisplayKind, buf: &mut [u8; 32]) -> usize {
    let sh1106 = kind == DisplayKind::Sh1106_128x64;
    let short = kind == DisplayKind::Ssd1306_128x32;
    let mut n = 0;
    let mut put = |bytes: &[u8]| {
        for &b in bytes {
            buf.wr(n, b);
            n += 1;
        }
    };
    put(&[0x00]); // control byte: commands follow
    put(&[0xAE]); // display off
    put(&[0x2E]); // scroll off: the glass keeps what the previous program started
    put(&[0xD5, 0x80]); // clock
    put(&[0xA8, if short { 0x1F } else { 0x3F }]); // multiplex
    put(&[0xD3, 0x00]); // display offset
    put(&[0x40]); // start line 0
    if sh1106 {
        put(&[0xAD, 0x8B]); // DC-DC on
    } else {
        put(&[0x8D, 0x14]); // charge pump on
        put(&[0x20, 0x02]); // page addressing
    }
    put(&[0xA1, 0xC8]); // segment and COM remap (normal orientation)
    put(&[0xDA, if short { 0x02 } else { 0x12 }]); // COM pins
    put(&[0x81, 0xCF]); // contrast
    put(&[0xD9, 0xF1, 0xDB, 0x40]); // pre-charge, VCOM
    put(&[0xA4, 0xA6, 0xAF]); // resume RAM, normal, display on
    n
}

/// One page transfer: set page and column, then 128 data bytes. Returns the length.
pub fn page_transfer(
    kind: DisplayKind,
    page: usize,
    data: &[u8],
    buf: &mut [u8; 7 + PAGE_BYTES],
) -> usize {
    let off = column_offset(kind);
    let head = [
        0x80,
        0xB0 | page as u8,
        0x80,
        off & 0x0F,
        0x80,
        0x10 | (off >> 4),
        0x40,
    ];
    copy(buf, &head);
    if let Some(dst) = buf.get_mut(7..) {
        copy(dst, data.part(PAGE_BYTES));
    }
    7 + PAGE_BYTES
}
