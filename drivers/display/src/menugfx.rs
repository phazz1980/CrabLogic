//! Graphic menu for the OLED: title with a line, items in the 8 or 16 pixel font with a square
//! marker, values on the right, a scrollbar and a bar of four inverted soft-key hints.
//!
//! Layout (128x64, pixels): title text at y 0..7, line at y 8, items from y 10 (16 px, hints on), 12 (16 px, no hints) or 11,
//! hint cells at y 56..63. 128x32: title, line, two 8-pixel items, no hints.

use crate::fb::Fb;
use crate::font::{glyph, CH_CHECK, CH_CROSS, CH_DOWN, CH_LEFT, CH_RIGHT, CH_UP};
use crate::menu::{item_at, level, Menu};
use crate::text::Txt;
use crab_ui_format::{ItemKindView, MenuView};

/// Marker column and caption start (pixels).
const CAPTION_X: i32 = 8;
/// Right edge (exclusive) of the content without / with the scrollbar.
const RIGHT: i32 = 128;
const RIGHT_SCROLL: i32 = 124;
/// Top of the hint cells and their height.
const HINT_Y: i32 = 56;
const HINT_H: i32 = 8;

/// Where the items go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Item font height, 8 or 16.
    pub font: u8,
    pub hints: bool,
    /// Top of the first item and the distance between items.
    pub first_y: i32,
    pub pitch: i32,
    /// Items that fit.
    pub visible: usize,
}

/// Layout for a display of `pages` pages (8 rows each) and the menu style. A 128x32 display has
/// no hint bar and always uses the 8-pixel font.
pub fn layout(pages: usize, font: u8, hints: bool) -> Layout {
    if pages < 8 {
        let visible = ((pages as i32 * 8 - 11) / 10).max(1) as usize;
        return Layout {
            font: 8,
            hints: false,
            first_y: 11,
            pitch: 10,
            visible,
        };
    }
    match (font >= 16, hints) {
        (true, true) => Layout {
            font: 16,
            hints,
            first_y: 10,
            pitch: 15,
            visible: 3,
        },
        (true, false) => Layout {
            font: 16,
            hints,
            first_y: 12,
            pitch: 17,
            visible: 3,
        },
        (false, _) => Layout {
            font: 8,
            hints,
            first_y: 11,
            pitch: 10,
            visible: if hints { 4 } else { 5 },
        },
    }
}

/// Soft-key labels left to right (back, up, down, enter), or while editing (cancel, +, -, apply).
pub fn hint_labels(editing: bool) -> [u16; 4] {
    if editing {
        [CH_CROSS, b'+' as u16, b'-' as u16, CH_CHECK]
    } else {
        [CH_LEFT, CH_UP, CH_DOWN, CH_RIGHT]
    }
}

/// Index of the first visible item so that item number `pos` of the level is on the screen.
pub fn top_for(pos: usize, visible: usize) -> usize {
    if pos < visible {
        0
    } else {
        pos + 1 - visible
    }
}

fn text_width(n: usize, scale: i32) -> i32 {
    if n == 0 {
        0
    } else {
        n as i32 * 6 * scale - scale
    }
}

impl Menu {
    /// Draws the menu into the cleared framebuffer.
    pub fn draw_fb(&self, m: &MenuView, fb: &mut Fb, var: &impl Fn(u16) -> f64) {
        let lay = layout(fb.pages, m.font, m.hints);
        let parent = self.parent();
        // Title and the line under it.
        let title = parent
            .and_then(|p| item_at(m, p))
            .map_or("Меню", |it| it.caption);
        let mut t = Txt::new();
        t.push_str(title);
        let fit = ((RIGHT - 2 + 1) / 6) as usize;
        fb.chars(1, 0, t.chars().get(..t.len().min(fit)).unwrap_or(&[]), 1);
        fb.fill_rect(0, 8, RIGHT, 1, true);

        let total = level(m, parent).count();
        let pos = level(m, parent)
            .take_while(|(i, _)| *i != self.cursor)
            .count();
        let top = top_for(pos, lay.visible);
        let scroll = total > lay.visible;
        let right = if scroll { RIGHT_SCROLL } else { RIGHT };
        let scale = if lay.font >= 16 { 2 } else { 1 };
        let adv = 6 * scale;
        for r in 0..lay.visible {
            let Some((idx, it)) = level(m, parent).nth(top + r) else {
                break;
            };
            let y = lay.first_y + r as i32 * lay.pitch;
            let selected = idx == self.cursor;
            let editing = selected && self.edit.is_some();
            if selected {
                let side = if scale == 2 { 5 } else { 4 };
                fb.fill_rect(0, y + (7 * scale - side) / 2, side, side, true);
            }
            // Value (or the submenu mark) on the right.
            let mut v = Txt::new();
            let is_sub = matches!(it.kind, ItemKindView::Submenu);
            if !is_sub {
                self.value_core(&mut v, &it, idx, editing, var, true);
            }
            let (vw, vx) = if is_sub {
                (5 * scale, right - 5 * scale)
            } else {
                let w = text_width(v.len(), scale);
                let end = if editing { right - 2 } else { right };
                (w, end - w)
            };
            if is_sub {
                fb.glyph(vx, y, &glyph(CH_RIGHT), scale, true);
            } else if !v.is_empty() {
                fb.chars(vx, y, v.chars(), scale);
            }
            if editing {
                // 1 px frame around the value, 2 px padding on the sides.
                fb.rect(vx - 2, y - 1, vw + 4, 7 * scale + 2);
            }
            // Caption in the room left.
            let gap = if v.is_empty() && !is_sub {
                0
            } else if editing {
                5
            } else {
                4
            };
            let room = (if v.is_empty() && !is_sub {
                right
            } else {
                vx - gap
            }) - CAPTION_X;
            let n = ((room + scale) / adv).max(0) as usize;
            let mut c = Txt::new();
            c.push_str(it.caption);
            fb.chars(
                CAPTION_X,
                y,
                c.chars().get(..c.len().min(n)).unwrap_or(&[]),
                scale,
            );
        }
        if scroll {
            let area = lay.visible as i32 * lay.pitch;
            let (y0, n, vis) = (lay.first_y, total as i32, lay.visible as i32);
            fb.fill_rect(127, y0, 1, area, true);
            let th = (area * vis / n).max(4);
            let ty = y0 + (area - th) * top as i32 / (n - vis).max(1);
            fb.fill_rect(126, ty, 2, th, true);
        }
        if lay.hints {
            draw_hints(fb, self.edit.is_some());
        }
    }
}

/// Four equal white cells with dark glyphs, 1 px apart.
fn draw_hints(fb: &mut Fb, editing: bool) {
    for (i, ch) in hint_labels(editing).into_iter().enumerate() {
        let x = 1 + i as i32 * 32;
        fb.fill_rect(x, HINT_Y, 31, HINT_H, true);
        fb.glyph(x + 13, HINT_Y, &glyph(ch), 1, false);
    }
}
