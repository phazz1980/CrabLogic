//! Zero-copy reader (driver side). No allocation, no panics.

use crate::{DisplayKind, FormatError, MIN_VERSION, NONE_U16, VERSION};

type Res<T> = Result<T, FormatError>;

#[derive(Clone)]
struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(d: &'a [u8]) -> Self {
        Rd { d, p: 0 }
    }
    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let e = self.p.checked_add(n).ok_or(FormatError::Truncated)?;
        let s = self.d.get(self.p..e).ok_or(FormatError::Truncated)?;
        self.p = e;
        Ok(s)
    }
    fn u8(&mut self) -> Res<u8> {
        match self.take(1)? {
            [a] => Ok(*a),
            _ => Err(FormatError::Truncated),
        }
    }
    fn u16(&mut self) -> Res<u16> {
        match self.take(2)? {
            [a, b] => Ok(u16::from_le_bytes([*a, *b])),
            _ => Err(FormatError::Truncated),
        }
    }
    fn opt(&mut self) -> Res<Option<u16>> {
        let v = self.u16()?;
        Ok(if v == NONE_U16 { None } else { Some(v) })
    }
    fn f32(&mut self) -> Res<f32> {
        match self.take(4)? {
            [a, b, c, d] => Ok(f32::from_le_bytes([*a, *b, *c, *d])),
            _ => Err(FormatError::Truncated),
        }
    }
    fn str(&mut self) -> Res<&'a str> {
        let n = self.u8()? as usize;
        core::str::from_utf8(self.take(n)?).map_err(|_| FormatError::BadUtf8)
    }
    fn rest(&self) -> &'a [u8] {
        self.d.get(self.p..).unwrap_or(&[])
    }
    fn done(&self) -> bool {
        self.p >= self.d.len()
    }
}

/// Widget payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WidgetKindView<'a> {
    Text {
        text: &'a str,
    },
    Value {
        format: u8,
        decimals: u8,
        unit: &'a str,
    },
    Bar {
        min: f32,
        max: f32,
    },
    Led,
    Icon {
        id: u8,
    },
}

/// One widget of a screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidgetView<'a> {
    pub kind: WidgetKindView<'a>,
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
    pub font: u8,
    /// Index of the driver input slot, if the widget shows a live value.
    pub input: Option<u16>,
}

fn widget_in<'a>(r: &mut Rd<'a>) -> Res<WidgetView<'a>> {
    let k = r.u8()?;
    let x = r.u8()?;
    let y = r.u8()?;
    let w = r.u8()?;
    let h = r.u8()?;
    let font = r.u8()?;
    let input = r.opt()?;
    let kind = match k {
        0 => WidgetKindView::Text { text: r.str()? },
        1 => {
            let format = r.u8()?;
            let decimals = r.u8()?;
            WidgetKindView::Value {
                format,
                decimals,
                unit: r.str()?,
            }
        }
        2 => {
            let min = r.f32()?;
            WidgetKindView::Bar { min, max: r.f32()? }
        }
        3 => WidgetKindView::Led,
        4 => WidgetKindView::Icon { id: r.u8()? },
        _ => return Err(FormatError::BadKind),
    };
    Ok(WidgetView {
        kind,
        x,
        y,
        w,
        h,
        font,
        input,
    })
}

/// Iterator over the widgets of one screen.
#[derive(Clone)]
pub struct WidgetIter<'a> {
    r: Rd<'a>,
    left: u8,
}

impl<'a> Iterator for WidgetIter<'a> {
    type Item = WidgetView<'a>;
    fn next(&mut self) -> Option<WidgetView<'a>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        match widget_in(&mut self.r) {
            Ok(w) => Some(w),
            Err(_) => {
                self.left = 0;
                None
            }
        }
    }
}

/// One screen.
#[derive(Clone, Copy)]
pub struct ScreenView<'a> {
    n: u8,
    body: &'a [u8],
}

impl<'a> ScreenView<'a> {
    /// Number of widgets.
    pub fn len(&self) -> usize {
        self.n as usize
    }
    /// True when the screen has no widgets.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Widgets in drawing order.
    pub fn widgets(&self) -> WidgetIter<'a> {
        WidgetIter {
            r: Rd::new(self.body),
            left: self.n,
        }
    }
}

fn screen_in<'a>(r: &mut Rd<'a>) -> Res<ScreenView<'a>> {
    let len = r.u16()? as usize;
    let mut s = Rd::new(r.take(len)?);
    let n = s.u8()?;
    let body = s.rest();
    // Validate every widget and require the body to be consumed exactly.
    let mut v = Rd::new(body);
    for _ in 0..n {
        widget_in(&mut v)?;
    }
    if !v.done() {
        return Err(FormatError::Trailing);
    }
    Ok(ScreenView { n, body })
}

/// Iterator over screens.
#[derive(Clone)]
pub struct ScreenIter<'a> {
    r: Rd<'a>,
    left: u8,
}

impl<'a> Iterator for ScreenIter<'a> {
    type Item = ScreenView<'a>;
    fn next(&mut self) -> Option<ScreenView<'a>> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        match screen_in(&mut self.r) {
            Ok(s) => Some(s),
            Err(_) => {
                self.left = 0;
                None
            }
        }
    }
}

/// Iterator over the choice captions of a `Choice` item.
#[derive(Clone)]
pub struct ChoiceIter<'a> {
    r: Rd<'a>,
    left: u8,
}

impl<'a> Iterator for ChoiceIter<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<&'a str> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        match self.r.str() {
            Ok(s) => Some(s),
            Err(_) => {
                self.left = 0;
                None
            }
        }
    }
}

/// Menu item payload.
#[derive(Clone)]
pub enum ItemKindView<'a> {
    Submenu,
    Number {
        min: f32,
        max: f32,
        step: f32,
        decimals: u8,
        unit: &'a str,
    },
    Bool,
    Choice {
        choices: ChoiceIter<'a>,
    },
    Action,
    View,
}

/// One menu item.
#[derive(Clone)]
pub struct ItemView<'a> {
    /// Index of the parent item (an earlier one), `None` for the root level.
    pub parent: Option<u16>,
    pub kind: ItemKindView<'a>,
    pub caption: &'a str,
    /// Index of the driver variable slot, if the item is bound to a variable.
    pub var_slot: Option<u16>,
}

fn item_in<'a>(r: &mut Rd<'a>, index: u16) -> Res<ItemView<'a>> {
    let k = r.u8()?;
    let parent = r.opt()?;
    let var_slot = r.opt()?;
    let caption = r.str()?;
    if let Some(p) = parent {
        if p >= index {
            return Err(FormatError::BadParent);
        }
    }
    let kind = match k {
        0 => ItemKindView::Submenu,
        1 => {
            let min = r.f32()?;
            let max = r.f32()?;
            let step = r.f32()?;
            let decimals = r.u8()?;
            ItemKindView::Number {
                min,
                max,
                step,
                decimals,
                unit: r.str()?,
            }
        }
        2 => ItemKindView::Bool,
        3 => {
            let left = r.u8()?;
            let start = r.clone();
            for _ in 0..left {
                r.str()?;
            }
            ItemKindView::Choice {
                choices: ChoiceIter { r: start, left },
            }
        }
        4 => ItemKindView::Action,
        5 => ItemKindView::View,
        _ => return Err(FormatError::BadKind),
    };
    Ok(ItemView {
        parent,
        kind,
        caption,
        var_slot,
    })
}

/// Iterator over menu items (index = position).
#[derive(Clone)]
pub struct ItemIter<'a> {
    r: Rd<'a>,
    index: u16,
    n: u16,
}

impl<'a> Iterator for ItemIter<'a> {
    type Item = ItemView<'a>;
    fn next(&mut self) -> Option<ItemView<'a>> {
        if self.index >= self.n {
            return None;
        }
        match item_in(&mut self.r, self.index) {
            Ok(i) => {
                self.index += 1;
                Some(i)
            }
            Err(_) => {
                self.index = self.n;
                None
            }
        }
    }
}

/// Menu description.
#[derive(Clone, Copy)]
pub struct MenuView<'a> {
    /// Auto-exit timeout in seconds, 0 = never.
    pub timeout_s: u16,
    /// Item font on the OLED (8 or 16).
    pub font: u8,
    /// Soft-key hint bar on the OLED.
    pub hints: bool,
    n_items: u16,
    items: &'a [u8],
}

impl<'a> MenuView<'a> {
    /// Number of items.
    pub fn len(&self) -> usize {
        self.n_items as usize
    }
    /// True when the menu has no items.
    pub fn is_empty(&self) -> bool {
        self.n_items == 0
    }
    /// Items in table order.
    pub fn items(&self) -> ItemIter<'a> {
        ItemIter {
            r: Rd::new(self.items),
            index: 0,
            n: self.n_items,
        }
    }
}

/// Parsed and fully validated config; iterators borrow the input bytes.
#[derive(Clone, Copy)]
pub struct ConfigView<'a> {
    pub display: DisplayKind,
    n_screens: u8,
    screens: &'a [u8],
    pub menu: Option<MenuView<'a>>,
}

impl<'a> ConfigView<'a> {
    /// Validate `data` completely and return a view. Never panics.
    pub fn parse(data: &'a [u8]) -> Result<ConfigView<'a>, FormatError> {
        let mut r = Rd::new(data);
        let version = r.u8()?;
        if !(MIN_VERSION..=VERSION).contains(&version) {
            return Err(FormatError::BadVersion);
        }
        let display = DisplayKind::from_u8(r.u8()?).ok_or(FormatError::BadKind)?;
        let n_screens = r.u8()?;
        let flags = r.u8()?;
        if flags & !1 != 0 {
            return Err(FormatError::BadKind);
        }
        let start = r.p;
        for _ in 0..n_screens {
            screen_in(&mut r)?;
        }
        let screens = data.get(start..r.p).ok_or(FormatError::Truncated)?;
        let menu = if flags & 1 != 0 {
            let timeout_s = r.u16()?;
            let (font, hints) = if version >= 2 {
                (r.u8()?, r.u8()? != 0)
            } else {
                (16, true)
            };
            let n_items = r.u16()?;
            let istart = r.p;
            for i in 0..n_items {
                item_in(&mut r, i)?;
            }
            let items = data.get(istart..r.p).ok_or(FormatError::Truncated)?;
            Some(MenuView {
                timeout_s,
                font,
                hints,
                n_items,
                items,
            })
        } else {
            None
        };
        if !r.done() {
            return Err(FormatError::Trailing);
        }
        Ok(ConfigView {
            display,
            n_screens,
            screens,
            menu,
        })
    }

    /// Number of screens.
    pub fn screen_count(&self) -> usize {
        self.n_screens as usize
    }
    /// Screens in order.
    pub fn screens(&self) -> ScreenIter<'a> {
        ScreenIter {
            r: Rd::new(self.screens),
            left: self.n_screens,
        }
    }
}
