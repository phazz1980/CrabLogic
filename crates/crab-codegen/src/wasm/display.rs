//! Displays in modules (`specs/display`): the `crab.config` blob of every display block
//! (`crab-ui-format`) and the layout of the extra driver inputs and outputs the generator
//! and the driver agree on (`specs/display/plan.md`, "Договорённость генератора и драйвера").
//!
//! Inputs of the driver: its own ports (`P`), the live values of the widgets that have an
//! input (`W`), `up/down/ok/back` of the menu (4) and the current values of the menu variables
//! (`V`). Outputs: its own ports (`Q`), `open/item/changed` of the menu (3), `changed_slot`
//! and the new value of every menu variable (`V`).

use std::collections::{BTreeMap, HashMap};

use crab_core::{ParamValue, SignalType};
use crab_ui_format::{
    Config, DisplayKind, Menu, MenuItem, MenuItemKind, Screen, Widget, WidgetKind,
};

use crate::program::{Instance, Program};

/// Names of the icons of `disp_icon`, in the order of the ids of the format.
const ICONS: [&str; 8] = [
    "drop",
    "flame",
    "bell",
    "wifi",
    "check",
    "cross",
    "arrow_up",
    "arrow_down",
];

/// A variable a menu item edits (one slot per variable).
pub(super) struct MenuVar {
    pub name: String,
    /// The item is an action: the variable gets a pulse of one step, not a value.
    pub action: bool,
}

/// The `menu` block wired to a display.
pub(super) struct MenuPlan {
    pub block: String,
    pub vars: Vec<MenuVar>,
}

/// Everything the generator needs to know about one display block.
pub(super) struct DisplayPlan {
    pub config: Config,
    /// Widget blocks with an input, in the order of their input slots.
    pub widgets: Vec<String>,
    pub menu: Option<MenuPlan>,
}

fn text(i: &Instance, name: &str) -> String {
    match i.params.get(name) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => String::new(),
    }
}

fn num(i: &Instance, name: &str, default: f64) -> f64 {
    match i.params.get(name) {
        Some(ParamValue::Int(v)) => *v as f64,
        Some(ParamValue::Float(x)) => *x,
        Some(ParamValue::Bool(b)) => f64::from(u8::from(*b)),
        _ => default,
    }
}

fn byte(i: &Instance, name: &str, default: f64) -> u8 {
    num(i, name, default).clamp(0.0, 255.0) as u8
}

/// Digits after the point of a step (`0.1` -> 1), at most 6.
fn decimals_of(step: f64) -> u8 {
    let s = format!("{step}");
    s.split_once('.').map_or(0, |(_, f)| f.len().min(6)) as u8
}

fn display_kind(d: &Instance) -> Option<DisplayKind> {
    Some(match (d.block.kind.as_str(), text(d, "model").as_str()) {
        ("oled", "ssd1306_128x64") => DisplayKind::Ssd1306_128x64,
        ("oled", "ssd1306_128x32") => DisplayKind::Ssd1306_128x32,
        ("oled", "sh1106_128x64") => DisplayKind::Sh1106_128x64,
        ("lcd", "1602") => DisplayKind::Lcd1602,
        ("lcd", "2004") => DisplayKind::Lcd2004,
        _ => return None,
    })
}

/// The widget of the format and whether it has an input slot.
fn widget_of(w: &Instance) -> Option<(Widget, bool)> {
    let (kind, input) = match w.block.kind.as_str() {
        "disp_text" => (
            WidgetKind::Text {
                text: text(w, "text"),
            },
            false,
        ),
        "disp_value" => {
            // `format` 0: the number and the unit; 1: `unit` holds the whole template with `%v`.
            let template = text(w, "format");
            let unit = text(w, "unit");
            let (format, unit) = if template.is_empty() {
                (0, unit)
            } else if unit.is_empty() || template.contains(&unit) {
                (1, template)
            } else {
                (1, format!("{template}{unit}"))
            };
            (
                WidgetKind::Value {
                    format,
                    decimals: byte(w, "decimals", 1.0).min(6),
                    unit,
                },
                true,
            )
        }
        "disp_bar" => (
            WidgetKind::Bar {
                min: num(w, "min", 0.0) as f32,
                max: num(w, "max", 100.0) as f32,
            },
            true,
        ),
        "disp_led" => (WidgetKind::Led, true),
        "disp_icon" => {
            let name = text(w, "icon");
            let id = ICONS.iter().position(|i| *i == name).unwrap_or(0) as u8;
            (WidgetKind::Icon { id }, false)
        }
        _ => return None,
    };
    let font = match num(w, "font", 8.0) as i64 {
        16 => 16,
        _ => 8,
    };
    Some((
        Widget {
            kind,
            x: byte(w, "x", 0.0),
            y: byte(w, "y", 0.0),
            w: byte(w, "w", 1.0),
            h: byte(w, "h", 1.0),
            font,
            input: None,
        },
        input,
    ))
}

impl Program<'_> {
    /// Plan of display block `id`; `None` when it is not a display.
    pub(super) fn display_plan(&self, id: &str) -> Option<DisplayPlan> {
        let logic = &self.project.logic;
        let by_id: HashMap<&str, &Instance> = self
            .blocks
            .iter()
            .map(|i| (i.block.id.as_str(), i))
            .collect();
        let display = *by_id.get(id)?;
        let kind = display_kind(display)?;
        let mut config = Config::new(kind);

        // Screens: widgets by their screen number, in the order of declaration.
        let mut screens: BTreeMap<u32, Vec<Widget>> = BTreeMap::new();
        let mut inputs: Vec<(u32, usize, String)> = Vec::new();
        for block in &logic.blocks {
            let Some(w) = by_id.get(block.id.as_str()).copied() else {
                continue;
            };
            if block.tag.is_some() || !block.kind.starts_with("disp_") || text(w, "display") != id {
                continue;
            }
            let screen = num(w, "screen", 1.0) as i64;
            if !(1..=255).contains(&screen) {
                continue;
            }
            let Some((widget, has_input)) = widget_of(w) else {
                continue;
            };
            let list = screens.entry(screen as u32).or_default();
            if has_input {
                inputs.push((screen as u32, list.len(), block.id.clone()));
            }
            list.push(widget);
        }
        // Input slots follow the order of the screens and the widgets on them.
        inputs.sort();
        let mut widgets = Vec::new();
        for (k, (screen, at, block)) in inputs.into_iter().enumerate() {
            screens.get_mut(&screen).expect("screen of a widget")[at].input = Some(k as u16);
            widgets.push(block);
        }
        let n = screens.keys().next_back().copied().unwrap_or(1);
        config.screens = (1..=n)
            .map(|s| Screen {
                widgets: screens.remove(&s).unwrap_or_default(),
            })
            .collect();

        let menu = match self.menu_plan(id, &by_id) {
            Some((block, items, vars)) => {
                config.menu = Some(Menu {
                    timeout_s: num(by_id[block.as_str()], "timeout", 30.0).clamp(0.0, 65535.0)
                        as u16,
                    items,
                });
                Some(MenuPlan { block, vars })
            }
            None => None,
        };
        Some(DisplayPlan {
            config,
            widgets,
            menu,
        })
    }

    /// The menu block of display `id`, its items in table order and the variables they edit.
    #[allow(clippy::type_complexity)]
    fn menu_plan(
        &self,
        id: &str,
        by_id: &HashMap<&str, &Instance>,
    ) -> Option<(String, Vec<MenuItem>, Vec<MenuVar>)> {
        let logic = &self.project.logic;
        let menu = logic.blocks.iter().find(|b| {
            b.kind == "menu"
                && b.tag.is_none()
                && by_id
                    .get(b.id.as_str())
                    .is_some_and(|i| text(i, "display") == id)
        })?;
        let items: Vec<&Instance> = logic
            .blocks
            .iter()
            .filter(|b| b.kind == "menu_item" && b.tag.is_none())
            .filter_map(|b| by_id.get(b.id.as_str()).copied())
            .filter(|i| text(i, "menu") == menu.id)
            .collect();
        let local: HashMap<&str, usize> = items
            .iter()
            .enumerate()
            .map(|(k, i)| (i.block.id.as_str(), k))
            .collect();
        // The last list holds the root level.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); items.len() + 1];
        for (k, item) in items.iter().enumerate() {
            let parent = local
                .get(text(item, "parent").as_str())
                .copied()
                .filter(|&p| p != k)
                .unwrap_or(items.len());
            children[parent].push(k);
        }
        for list in &mut children {
            list.sort_by_key(|&k| (num(items[k], "order", 0.0) as i64, k));
        }

        let mut table = Vec::new();
        let mut vars: Vec<MenuVar> = Vec::new();
        // Depth first, parents before their children.
        let mut stack: Vec<(usize, Option<u16>)> = children[items.len()]
            .iter()
            .rev()
            .map(|&k| (k, None))
            .collect();
        while let Some((k, parent)) = stack.pop() {
            let item = items[k];
            let kind = text(item, "kind");
            let var = text(item, "var");
            let var_ty = logic.var(&var).map(|v| v.ty.signal());
            let var_slot = match (kind.as_str(), var_ty) {
                ("submenu", _) | (_, None) => None,
                _ => {
                    let slot = vars.iter().position(|v| v.name == var).unwrap_or_else(|| {
                        vars.push(MenuVar {
                            name: var.clone(),
                            action: kind == "action",
                        });
                        vars.len() - 1
                    });
                    Some(slot as u16)
                }
            };
            let payload = match kind.as_str() {
                "submenu" => MenuItemKind::Submenu,
                "bool" => MenuItemKind::Bool,
                "action" => MenuItemKind::Action,
                "view" => MenuItemKind::View,
                "choice" => MenuItemKind::Choice {
                    choices: text(item, "choices")
                        .split(';')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                },
                _ => {
                    let step = num(item, "step", 1.0);
                    MenuItemKind::Number {
                        min: num(item, "min", 0.0) as f32,
                        max: num(item, "max", 100.0) as f32,
                        step: step as f32,
                        decimals: if var_ty == Some(SignalType::Int) {
                            0
                        } else {
                            decimals_of(step)
                        },
                        unit: text(item, "unit"),
                    }
                }
            };
            let at = table.len() as u16;
            table.push(MenuItem {
                parent,
                kind: payload,
                caption: text(item, "caption"),
                var_slot,
            });
            for &c in children[k].iter().rev() {
                stack.push((c, Some(at)));
            }
        }
        Some((menu.id.clone(), table, vars))
    }

    /// The `crab.config` blob of every display block of the project, by block id: screens and
    /// widgets of the constructor and the items of its menu.
    pub fn display_configs(&self) -> BTreeMap<String, Vec<u8>> {
        self.blocks
            .iter()
            .filter(|i| i.block.tag.is_none())
            .filter_map(|i| {
                let plan = self.display_plan(&i.block.id)?;
                Some((i.block.id.clone(), plan.config.encode()))
            })
            .collect()
    }

    /// The description of one display block (for the simulator window and the editor).
    pub fn display_config(&self, id: &str) -> Option<Config> {
        self.display_plan(id).map(|p| p.config)
    }
}

/// Blocks of the display constructor that emit no code: widgets and menu items are read by
/// the display they belong to, the menu block is wired to its display.
pub(super) fn is_inert(kind: &str) -> bool {
    kind.starts_with("disp_") || kind == "menu_item" || kind == "menu"
}
