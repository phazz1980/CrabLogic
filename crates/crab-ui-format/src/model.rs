//! Owned model and encoder (PC side).

use crate::{DisplayKind, NONE_U16, VERSION};
use alloc::string::String;
use alloc::vec::Vec;

/// Widget payload.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WidgetKind {
    Text {
        text: String,
    },
    /// `format`: 0 = plain number, other values are interpreted by the driver.
    Value {
        format: u8,
        decimals: u8,
        unit: String,
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
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Widget {
    pub kind: WidgetKind,
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
    /// Font height in pixels: 8 or 16.
    pub font: u8,
    /// Index of the driver input slot (live value), if any.
    pub input: Option<u16>,
}

/// One screen.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Screen {
    pub widgets: Vec<Widget>,
}

/// Menu item payload.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MenuItemKind {
    Submenu,
    Number {
        min: f32,
        max: f32,
        step: f32,
        decimals: u8,
        unit: String,
    },
    Bool,
    Choice {
        choices: Vec<String>,
    },
    Action,
    View,
}

/// One menu item.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MenuItem {
    /// Index of an earlier item, `None` at the root level.
    pub parent: Option<u16>,
    pub kind: MenuItemKind,
    pub caption: String,
    pub var_slot: Option<u16>,
}

/// Menu: items in table order.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Menu {
    pub timeout_s: u16,
    /// Item font on the OLED: 8 or 16 (16 is the default).
    #[cfg_attr(feature = "serde", serde(default = "default_font"))]
    pub font: u8,
    /// Soft-key hint bar at the bottom of the OLED.
    #[cfg_attr(feature = "serde", serde(default = "default_hints"))]
    pub hints: bool,
    pub items: Vec<MenuItem>,
}

#[cfg(feature = "serde")]
fn default_font() -> u8 {
    16
}

#[cfg(feature = "serde")]
fn default_hints() -> bool {
    true
}

impl Default for Menu {
    fn default() -> Self {
        Menu {
            timeout_s: 30,
            font: 16,
            hints: true,
            items: Vec::new(),
        }
    }
}

/// Whole description of one display.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    pub version: u8,
    pub display: DisplayKind,
    pub screens: Vec<Screen>,
    pub menu: Option<Menu>,
}

impl Config {
    /// Empty config of the current version.
    pub fn new(display: DisplayKind) -> Config {
        Config {
            version: VERSION,
            display,
            screens: Vec::new(),
            menu: None,
        }
    }

    /// Encode to the wire format. Limits: 255 screens, 255 widgets per screen,
    /// 255 choices per item, strings of at most 255 bytes (cut on a char boundary);
    /// anything above is dropped, so check sizes beforehand if that matters.
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let screens = &self.screens[..self.screens.len().min(255)];
        o.push(self.version);
        o.push(self.display as u8);
        o.push(screens.len() as u8);
        o.push(u8::from(self.menu.is_some()));
        for s in screens {
            let widgets = &s.widgets[..s.widgets.len().min(255)];
            let at = o.len();
            o.extend_from_slice(&[0, 0, widgets.len() as u8]);
            for w in widgets {
                put_widget(&mut o, w);
            }
            let len = (o.len() - at - 2).min(u16::MAX as usize) as u16;
            o[at..at + 2].copy_from_slice(&len.to_le_bytes());
        }
        if let Some(m) = &self.menu {
            let items = &m.items[..m.items.len().min(NONE_U16 as usize - 1)];
            o.extend_from_slice(&m.timeout_s.to_le_bytes());
            if self.version >= 2 {
                o.push(m.font);
                o.push(u8::from(m.hints));
            }
            o.extend_from_slice(&(items.len() as u16).to_le_bytes());
            for i in items {
                put_item(&mut o, i);
            }
        }
        o
    }
}

fn put_str(o: &mut Vec<u8>, s: &str) {
    let mut n = s.len().min(255);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    o.push(n as u8);
    o.extend_from_slice(&s.as_bytes()[..n]);
}

fn put_opt(o: &mut Vec<u8>, v: Option<u16>) {
    o.extend_from_slice(&v.unwrap_or(NONE_U16).to_le_bytes());
}

fn put_widget(o: &mut Vec<u8>, w: &Widget) {
    let k = match w.kind {
        WidgetKind::Text { .. } => 0,
        WidgetKind::Value { .. } => 1,
        WidgetKind::Bar { .. } => 2,
        WidgetKind::Led => 3,
        WidgetKind::Icon { .. } => 4,
    };
    o.extend_from_slice(&[k, w.x, w.y, w.w, w.h, w.font]);
    put_opt(o, w.input);
    match &w.kind {
        WidgetKind::Text { text } => put_str(o, text),
        WidgetKind::Value {
            format,
            decimals,
            unit,
        } => {
            o.extend_from_slice(&[*format, *decimals]);
            put_str(o, unit);
        }
        WidgetKind::Bar { min, max } => {
            o.extend_from_slice(&min.to_le_bytes());
            o.extend_from_slice(&max.to_le_bytes());
        }
        WidgetKind::Led => {}
        WidgetKind::Icon { id } => o.push(*id),
    }
}

fn put_item(o: &mut Vec<u8>, i: &MenuItem) {
    let k = match i.kind {
        MenuItemKind::Submenu => 0,
        MenuItemKind::Number { .. } => 1,
        MenuItemKind::Bool => 2,
        MenuItemKind::Choice { .. } => 3,
        MenuItemKind::Action => 4,
        MenuItemKind::View => 5,
    };
    o.push(k);
    put_opt(o, i.parent);
    put_opt(o, i.var_slot);
    put_str(o, &i.caption);
    match &i.kind {
        MenuItemKind::Number {
            min,
            max,
            step,
            decimals,
            unit,
        } => {
            for f in [min, max, step] {
                o.extend_from_slice(&f.to_le_bytes());
            }
            o.push(*decimals);
            put_str(o, unit);
        }
        MenuItemKind::Choice { choices } => {
            let c = &choices[..choices.len().min(255)];
            o.push(c.len() as u8);
            for s in c {
                put_str(o, s);
            }
        }
        _ => {}
    }
}
