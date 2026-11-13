//! Menu engine: navigation by four edge-triggered keys, nested submenus, number / bool / choice
//! editing (local until OK), actions, views, idle timeout; and drawing into a character grid.
//!
//! Integers only: the value being edited is an `i32` in units of `10^-decimals` of its item, a
//! variable is a raw 32-bit cell (`f32` bits for a `float` variable, an `i32` otherwise) that the
//! caller reads through `var(slot, float)`.

use crate::safe::Safe;
use crate::text::{clamp, pow10, scaled_cell, unscaled_bits, whole_cell, Grid, Txt, INVALID};
use crab_ui_format::{ItemKindView, ItemView, MenuView};

/// Nesting depth limit.
const MAX_DEPTH: usize = 8;
/// A committed value overrides the live variable for this many steps (the sheet needs a step or
/// two to write it back), so the screen does not flash the old value.
const SHADOW_STEPS: u8 = 4;
const NONE: u16 = 0xFFFF;

pub const KEY_UP: usize = 0;
pub const KEY_DOWN: usize = 1;
pub const KEY_OK: usize = 2;
pub const KEY_BACK: usize = 3;

/// What the menu reports in one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuOut {
    pub open: bool,
    /// Table index of the highlighted item, -1 when the menu is closed or the level is empty.
    pub item: i32,
    /// One-step pulse: a value was confirmed with OK or an action fired.
    pub changed: bool,
    /// Variable slot of the confirmed item, -1 when nothing was confirmed or the item has no variable.
    pub slot: i32,
    /// The new value for `slot`: the bits of the variable cell (`f32` bits when `float`).
    pub value: u32,
    /// The variable of `slot` is `float`.
    pub float: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Edit {
    pub(crate) item: u16,
    /// Number: units of `10^-decimals` of the item; choice: the variant index.
    pub(crate) value: i32,
}

#[derive(Clone, Copy)]
struct Shadow {
    slot: u16,
    bits: u32,
    ttl: u8,
}

pub struct Menu {
    open: bool,
    pub(crate) depth: usize,
    /// Table indices of the opened submenus, outermost first.
    path: [u16; MAX_DEPTH],
    /// Highlighted item (table index), `NONE` for an empty level.
    pub(crate) cursor: u16,
    pub(crate) edit: Option<Edit>,
    idle_ms: u32,
    prev: [bool; 4],
    shadow: Option<Shadow>,
}

pub(crate) fn item_at<'a>(m: &MenuView<'a>, i: u16) -> Option<ItemView<'a>> {
    m.items().nth(i as usize)
}

/// Items of the level `parent` (table order).
pub(crate) fn level<'a, 'b>(
    m: &'b MenuView<'a>,
    parent: Option<u16>,
) -> impl Iterator<Item = (u16, ItemView<'a>)> + 'b {
    m.items()
        .enumerate()
        .filter(move |(_, it)| it.parent == parent)
        .map(|(i, it)| (i as u16, it))
}

fn first_of(m: &MenuView, parent: Option<u16>) -> u16 {
    level(m, parent).next().map_or(NONE, |(i, _)| i)
}

/// Neighbouring item of `cur` on the same level, wrapping around.
fn neighbour(m: &MenuView, parent: Option<u16>, cur: u16, forward: bool) -> u16 {
    if forward {
        level(m, parent)
            .map(|(i, _)| i)
            .find(|&i| i > cur)
            .unwrap_or_else(|| first_of(m, parent))
    } else {
        let mut prev = NONE;
        let mut last = NONE;
        for (i, _) in level(m, parent) {
            if i < cur {
                prev = i;
            }
            last = i;
        }
        if prev != NONE {
            prev
        } else {
            last
        }
    }
}

/// A cell of an integer variable.
fn int_bits(v: i32) -> u32 {
    v as u32
}

/// The cell of a variable holding the whole number `n` (a choice index, 0/1 of a switch).
fn whole_bits(n: i32, float: bool) -> u32 {
    if float {
        (n as f32).to_bits()
    } else {
        int_bits(n)
    }
}

/// The cell is "on": `f32` above one half, an integer above zero.
fn cell_on(bits: u32, float: bool) -> bool {
    if float {
        f32::from_bits(bits) > 0.5
    } else {
        (bits as i32) > 0
    }
}

/// The cell of a variable edited as the scaled number `n` of an item with `decimals`.
fn number_bits(n: i32, decimals: u8, float: bool) -> u32 {
    if float {
        unscaled_bits(n, decimals)
    } else {
        // An integer variable has no decimals; with some, the nearest whole number.
        let scale = pow10(decimals) as i32;
        let q = n / scale;
        let r = n % scale;
        int_bits(if r.abs() * 2 >= scale {
            q + n.signum()
        } else {
            q
        })
    }
}

impl Menu {
    pub const fn new() -> Menu {
        Menu {
            open: false,
            depth: 0,
            path: [0; MAX_DEPTH],
            cursor: NONE,
            edit: None,
            idle_ms: 0,
            prev: [false; 4],
            shadow: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn parent(&self) -> Option<u16> {
        if self.depth == 0 {
            None
        } else {
            Some(self.path.rd(self.depth.wrapping_sub(1)))
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.depth = 0;
        self.edit = None;
        self.cursor = NONE;
    }

    /// Current value of a variable slot: a just-confirmed value wins over the live one.
    pub fn value_of(&self, var: &impl Fn(u16, bool) -> u32, slot: u16, float: bool) -> u32 {
        match self.shadow {
            Some(s) if s.slot == slot => s.bits,
            _ => var(slot, float),
        }
    }

    /// One step. `keys` are up, down, ok, back (levels; edges are detected here).
    pub fn step(
        &mut self,
        m: &MenuView,
        dt_ms: i32,
        keys: [bool; 4],
        var: &impl Fn(u16, bool) -> u32,
    ) -> MenuOut {
        let mut out = MenuOut {
            open: false,
            item: -1,
            changed: false,
            slot: -1,
            value: 0,
            float: false,
        };
        if let Some(s) = self.shadow.as_mut() {
            s.ttl = s.ttl.saturating_sub(1);
            if s.ttl == 0 {
                self.shadow = None;
            }
        }
        let prev = self.prev;
        let edge = [0, 1, 2, 3].map(|k| keys.rd(k) && !prev.rd(k));
        self.prev = keys;
        if edge.iter().any(|&e| e) {
            self.idle_ms = 0;
        } else if self.open {
            self.idle_ms = self.idle_ms.saturating_add(dt_ms.max(0) as u32);
            if m.timeout_s > 0 && self.idle_ms >= m.timeout_s as u32 * 1000 {
                self.close();
            }
        }
        if !self.open {
            if edge.rd(KEY_OK) {
                self.open = true;
                self.depth = 0;
                self.cursor = first_of(m, None);
            }
        } else {
            if edge.rd(KEY_UP) {
                self.up_down(m, false);
            }
            if edge.rd(KEY_DOWN) {
                self.up_down(m, true);
            }
            if edge.rd(KEY_OK) {
                self.ok(m, var, &mut out);
            }
            if edge.rd(KEY_BACK) {
                self.back(m);
            }
        }
        out.open = self.open;
        if self.open && self.cursor != NONE {
            out.item = self.cursor as i32;
        }
        out
    }

    fn up_down(&mut self, m: &MenuView, down: bool) {
        if let Some(e) = self.edit.as_mut() {
            let Some(it) = item_at(m, e.item) else { return };
            match it.kind {
                ItemKindView::Number {
                    min,
                    max,
                    step,
                    decimals,
                    ..
                } => {
                    let step = if step > 0 {
                        step
                    } else {
                        pow10(decimals) as i32
                    };
                    let v = if down {
                        e.value.saturating_sub(step)
                    } else {
                        e.value.saturating_add(step)
                    };
                    e.value = clamp(v, min, max);
                }
                ItemKindView::Choice { choices } => {
                    let n = choices.count().max(1) as i32;
                    let v = e.value + if down { 1 } else { -1 };
                    e.value = if v < 0 {
                        n - 1
                    } else if v >= n {
                        0
                    } else {
                        v
                    };
                }
                _ => {}
            }
        } else if self.cursor != NONE {
            self.cursor = neighbour(m, self.parent(), self.cursor, down);
        }
    }

    fn ok(&mut self, m: &MenuView, var: &impl Fn(u16, bool) -> u32, out: &mut MenuOut) {
        if let Some(e) = self.edit.take() {
            if let Some(it) = item_at(m, e.item) {
                let bits = match it.kind {
                    ItemKindView::Number { decimals, .. } => {
                        number_bits(e.value, decimals, it.var_float)
                    }
                    _ => whole_bits(e.value, it.var_float),
                };
                self.confirm(it.var_slot, bits, it.var_float, out);
            }
            return;
        }
        let Some(it) = item_at(m, self.cursor) else {
            return;
        };
        match it.kind {
            ItemKindView::Submenu => {
                if self.depth < MAX_DEPTH {
                    self.path.wr(self.depth, self.cursor);
                    self.depth += 1;
                    self.cursor = first_of(m, Some(self.cursor));
                }
            }
            ItemKindView::Number {
                min, max, decimals, ..
            } => {
                let v = it.var_slot.map_or(min, |s| {
                    scaled_cell(self.value_of(var, s, it.var_float), it.var_float, decimals)
                });
                // NaN and out-of-range values start from the lower limit.
                self.edit = Some(Edit {
                    item: self.cursor,
                    value: if v == INVALID {
                        min
                    } else {
                        clamp(v, min, max)
                    },
                });
            }
            ItemKindView::Choice { choices } => {
                let n = choices.count().max(1) as i32;
                let v = it.var_slot.map_or(0, |s| {
                    whole_cell(self.value_of(var, s, it.var_float), it.var_float)
                });
                self.edit = Some(Edit {
                    item: self.cursor,
                    value: clamp(v, 0, n - 1),
                });
            }
            ItemKindView::Bool => {
                if let Some(s) = it.var_slot {
                    let now = cell_on(self.value_of(var, s, it.var_float), it.var_float);
                    let bits = whole_bits(i32::from(!now), it.var_float);
                    self.confirm(Some(s), bits, it.var_float, out);
                } else {
                    self.confirm(None, 0, false, out);
                }
            }
            ItemKindView::Action => {
                self.confirm(it.var_slot, whole_bits(1, it.var_float), it.var_float, out)
            }
            ItemKindView::View => {}
        }
    }

    fn confirm(&mut self, slot: Option<u16>, bits: u32, float: bool, out: &mut MenuOut) {
        out.changed = true;
        if let Some(s) = slot {
            out.slot = s as i32;
            out.value = bits;
            out.float = float;
            self.shadow = Some(Shadow {
                slot: s,
                bits,
                ttl: SHADOW_STEPS,
            });
        }
    }

    fn back(&mut self, m: &MenuView) {
        if self.edit.take().is_some() {
            return;
        }
        if self.depth == 0 {
            self.close();
        } else {
            self.depth -= 1;
            self.cursor = self.path.rd(self.depth);
            // The highlighted item must live on the level we returned to.
            if item_at(m, self.cursor).is_none() {
                self.cursor = first_of(m, self.parent());
            }
        }
    }

    /// Draws the menu into `g` (cleared first).
    pub fn draw(&self, m: &MenuView, g: &mut Grid, var: &impl Fn(u16, bool) -> u32) {
        let (cols, rows) = (g.cols, g.rows);
        g.reset(cols, rows);
        let parent = self.parent();
        let mut first_row = 0;
        if rows >= 4 {
            let title = parent
                .and_then(|p| item_at(m, p))
                .map_or("Меню", |it| it.caption);
            let mut t = Txt::new();
            t.push_str(title);
            g.put(0, 0, t.chars().part(cols));
            g.inv |= 1;
            first_row = 1;
        }
        let avail = rows - first_row;
        if avail == 0 {
            return;
        }
        let pos = level(m, parent)
            .take_while(|(i, _)| *i != self.cursor)
            .count();
        let top = if pos < avail { 0 } else { pos + 1 - avail };
        for r in 0..avail {
            let Some((idx, it)) = level(m, parent).nth(top + r) else {
                break;
            };
            let row = first_row + r;
            let editing = self.edit.is_some() && idx == self.cursor;
            g.put(0, row, &[if idx == self.cursor { b'>' as u16 } else { 0 }]);
            // Right part: the value (or a submenu mark), then the caption in the room left.
            let mut v = Txt::new();
            self.value_text(&mut v, &it, idx, editing, var);
            if matches!(it.kind, ItemKindView::Submenu) {
                v.push(b'+' as u16);
            }
            let room = cols.saturating_sub(1 + v.len() + usize::from(!v.is_empty()));
            let mut c = Txt::new();
            c.push_str(it.caption);
            g.put(1, row, c.chars().part(room));
            g.put(cols.saturating_sub(v.len()), row, v.chars());
        }
    }

    fn value_text(
        &self,
        t: &mut Txt,
        it: &ItemView,
        idx: u16,
        editing: bool,
        var: &impl Fn(u16, bool) -> u32,
    ) {
        if editing {
            t.push(b'[' as u16);
        }
        self.value_core(t, it, idx, editing, var, false);
        if editing {
            t.push(b']' as u16);
        }
    }

    /// The shown value of an item without edit brackets. `mixed`: «Вкл/Выкл» instead of «ВКЛ/ВЫКЛ».
    pub(crate) fn value_core(
        &self,
        t: &mut Txt,
        it: &ItemView,
        idx: u16,
        editing: bool,
        var: &impl Fn(u16, bool) -> u32,
        mixed: bool,
    ) {
        let float = it.var_float;
        // The live cell of the variable (an item without a variable shows zero).
        let live = |s: Option<u16>| s.map_or(0, |s| self.value_of(var, s, float));
        let edited = match self.edit {
            Some(e) if editing && e.item == idx => Some(e.value),
            _ => None,
        };
        match &it.kind {
            ItemKindView::Number { decimals, unit, .. } => {
                let n = edited.unwrap_or_else(|| scaled_cell(live(it.var_slot), float, *decimals));
                t.push_scaled(n, *decimals);
                t.push_str(unit);
            }
            ItemKindView::Bool => t.push_str(match (cell_on(live(it.var_slot), float), mixed) {
                (true, false) => "ВКЛ",
                (false, false) => "ВЫКЛ",
                (true, true) => "Вкл",
                (false, true) => "Выкл",
            }),
            ItemKindView::Choice { choices } => {
                let n = edited.unwrap_or_else(|| whole_cell(live(it.var_slot), float));
                let n = if n >= 0 { n as usize } else { 0 };
                if let Some(s) = choices.clone().nth(n) {
                    t.push_str(s);
                }
            }
            ItemKindView::View => t.push_auto(live(it.var_slot), float),
            ItemKindView::Submenu | ItemKindView::Action => {}
        }
    }

    /// Cheap fingerprint of everything the graphic menu shows: position, edit state and the
    /// values of the current level. Equal fingerprints mean the same picture.
    pub fn signature(&self, m: &MenuView, var: &impl Fn(u16, bool) -> u32) -> u32 {
        let mut h: u32 = 0x811C_9DC5;
        let mut mix = |v: u32| {
            h = (h ^ v).wrapping_mul(0x0100_0193);
        };
        mix(self.depth as u32);
        mix(self.cursor as u32);
        if let Some(e) = self.edit {
            mix(1 + e.item as u32);
            mix(e.value as u32);
        }
        for (_, it) in level(m, self.parent()) {
            if let Some(s) = it.var_slot {
                mix(self.value_of(var, s, it.var_float));
            }
        }
        h
    }
}

impl Default for Menu {
    fn default() -> Self {
        Menu::new()
    }
}
