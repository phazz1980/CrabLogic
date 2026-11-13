//! Displays, screen widgets and menus (`specs/display`): widgets inside the screen, one
//! I²C address per display, the display of a menu, Cyrillic letters of a text LCD and
//! the variables of menu items.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::*;
use crate::Block;

/// Most different Cyrillic letters one LCD screen can draw (user characters of the
/// HD44780 generator).
pub const LCD_CYRILLIC_LIMIT: usize = 8;

/// Size of the screen of a display block: pixels of an OLED, character cells of an LCD.
/// `None` when the block is not a display or its model is unknown.
pub fn screen_size(block: &Block) -> Option<(i64, i64)> {
    match block.kind.as_str() {
        "oled" => match text_param(block, "model", "ssd1306_128x64") {
            "ssd1306_128x64" | "sh1106_128x64" => Some((128, 64)),
            "ssd1306_128x32" => Some((128, 32)),
            _ => None,
        },
        "lcd" => match text_param(block, "model", "1602") {
            "1602" => Some((16, 2)),
            "2004" => Some((20, 4)),
            _ => None,
        },
        _ => None,
    }
}

fn is_display(block: &Block) -> bool {
    matches!(block.kind.as_str(), "oled" | "lcd")
}

fn text_param<'a>(block: &'a Block, name: &str, default: &'a str) -> &'a str {
    match block.params.get(name) {
        Some(ParamValue::Text(t)) => t,
        _ => default,
    }
}

fn int_param(block: &Block, name: &str, default: i64) -> i64 {
    match block.params.get(name) {
        Some(ParamValue::Int(i)) => *i,
        Some(ParamValue::Float(x)) => *x as i64,
        _ => default,
    }
}

pub(super) fn check_displays(logic: &Logic, report: &mut Report) {
    check_display_addresses(logic, report);
    check_screen_widgets(logic, report);
    check_menus(logic, report);
    check_menu_items(logic, report);
}

/// Two displays at one address on one bus.
fn check_display_addresses(logic: &Logic, report: &mut Report) {
    let mut seen: HashMap<(i64, i64, i64), &str> = HashMap::new();
    for block in logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && is_display(b))
    {
        let default_address = if block.kind == "oled" { 60 } else { 39 };
        let key = (
            int_param(block, "sda", -1),
            int_param(block, "scl", -1),
            int_param(block, "address", default_address),
        );
        if let Some(first) = seen.insert(key, block.id.as_str()) {
            report.block_error(
                DiagKind::DisplayAddress,
                &block.id,
                format!(
                    "дисплей «{}»: адрес 0x{:02X} на этой шине I²C уже занят дисплеем «{first}»",
                    block.id, key.2
                ),
            );
        }
    }
}

/// Widgets of the screens: the display exists and the widget fits the screen; text of an
/// LCD screen needs not more than [`LCD_CYRILLIC_LIMIT`] different Cyrillic letters.
fn check_screen_widgets(logic: &Logic, report: &mut Report) {
    let mut letters: BTreeMap<(String, i64), BTreeSet<char>> = BTreeMap::new();
    for block in logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && b.kind.starts_with("disp_"))
    {
        let id = block.id.as_str();
        let name = text_param(block, "display", "");
        let Some(display) = logic.block(name).filter(|d| is_display(d)) else {
            let message = if name.is_empty() {
                format!("виджет «{id}»: не выбран дисплей")
            } else if logic.block(name).is_some() {
                format!("виджет «{id}»: «{name}» — не дисплей (нужен блок oled или lcd)")
            } else {
                format!("виджет «{id}»: дисплея «{name}» нет в проекте")
            };
            report.block_error(DiagKind::DisplayWidget, id, message);
            continue;
        };
        let (width, height) = screen_size(display).unwrap_or((0, 0));
        let (x, y) = (int_param(block, "x", 0), int_param(block, "y", 0));
        let (w, h) = (int_param(block, "w", 1), int_param(block, "h", 1));
        if width > 0 && (x < 0 || y < 0 || x + w > width || y + h > height) {
            let unit = if display.kind == "oled" {
                "пикселей"
            } else {
                "знакомест"
            };
            report.block_error(
                DiagKind::DisplayWidget,
                id,
                format!(
                    "виджет «{id}»: область x={x}, y={y}, {w}×{h} выходит за экран дисплея «{}» ({width}×{height} {unit})",
                    display.id
                ),
            );
        }
        if display.kind == "lcd" {
            let screen = int_param(block, "screen", 1);
            let entry = letters.entry((display.id.clone(), screen)).or_default();
            let texts: &[&str] = match block.kind.as_str() {
                "disp_text" => &["text"],
                "disp_value" => &["format", "unit"],
                _ => &[],
            };
            for t in texts {
                entry.extend(
                    text_param(block, t, "")
                        .chars()
                        .filter(|c| ('\u{0400}'..='\u{04FF}').contains(c)),
                );
            }
        }
    }
    for ((display, screen), set) in letters {
        if set.len() > LCD_CYRILLIC_LIMIT {
            report.push(
                DiagKind::DisplayCyrillic,
                Severity::Warning,
                Some(&display),
                None,
                format!(
                    "дисплей «{display}», экран {screen}: {} разных русских букв, а знакогенератор LCD рисует не больше {LCD_CYRILLIC_LIMIT}; лишние буквы не будут показаны",
                    set.len()
                ),
            );
        }
    }
}

/// The menu block points at a display.
fn check_menus(logic: &Logic, report: &mut Report) {
    for block in logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && b.kind == "menu")
    {
        let id = block.id.as_str();
        let name = text_param(block, "display", "");
        let message = if name.is_empty() {
            Some(format!("меню «{id}»: не выбран дисплей"))
        } else {
            match logic.block(name) {
                None => Some(format!("меню «{id}»: дисплея «{name}» нет в проекте")),
                Some(d) if !is_display(d) => Some(format!(
                    "меню «{id}»: «{name}» — не дисплей (нужен блок oled или lcd)"
                )),
                Some(_) => None,
            }
        };
        if let Some(message) = message {
            report.block_error(DiagKind::MenuDisplay, id, message);
        }
    }
}

/// The variable of a menu item has the type its kind needs.
fn check_menu_items(logic: &Logic, report: &mut Report) {
    for block in logic
        .blocks
        .iter()
        .filter(|b| b.tag.is_none() && b.kind == "menu_item")
    {
        let id = block.id.as_str();
        let kind = text_param(block, "kind", "number");
        let var = text_param(block, "var", "");
        let (wanted, what) = match kind {
            "number" => (&[SignalType::Int, SignalType::Float][..], "int или float"),
            "bool" | "action" => (&[SignalType::Bool][..], "bool"),
            "choice" => (&[SignalType::Int][..], "int"),
            _ => continue,
        };
        let message = if var.is_empty() {
            Some(format!("пункт меню «{id}»: не выбрана переменная"))
        } else {
            match logic.var(var) {
                None => Some(format!("пункт меню «{id}»: переменной «{var}» нет в проекте")),
                Some(v) if !wanted.contains(&v.ty.signal()) => Some(format!(
                    "пункт меню «{id}»: переменная «{var}» типа {}, а пункту вида «{kind}» нужна {what}",
                    v.ty.name()
                )),
                Some(_) => None,
            }
        };
        if let Some(message) = message {
            report.block_error(DiagKind::MenuItemVar, id, message);
        }
    }
}
