//! Manual layout of the electrical-panel sheets (`panel-layout.toml`): where the user moved a
//! device symbol or the middle segment of a wire, relative to the automatic place
//! (`specs/panel-editor`). Stored as offsets, so an automatic layout that moves when devices
//! are added keeps the manual part relative to it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::panel::Panel;

/// Entry of the project archive (and file of the folder) with the manual layout; written only
/// when the layout is not empty.
pub const PANEL_LAYOUT_FILE: &str = "panel-layout.toml";

/// Grid of the panel sheets in units of 1/4 mm: half a module (2.5 mm) is 10 u, so the
/// positions snap to 5 u.
const GRID: f32 = 5.0;

/// Manual layout of the panel sheets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelLayout {
    /// key = "<view>:<device id>" (see `symbol_key`)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<String, SymbolShift>,
    /// key = wire key as `crab_schematic::nets::Wire::key()` gives it (e.g. `K1:2>M1:L1`)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wires: BTreeMap<String, WireShape>,
    /// key = number of a power sheet (from 0) as text (TOML keys are strings): the number of the sheet its supply lines are
    /// carried on to by hand; the sheets between get the lines too.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bus_to: BTreeMap<String, usize>,
    /// Devices put on a control sheet by hand: key = the device id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub placed: BTreeMap<String, Placed>,
}

/// Where a device was put on the control sheets by hand: the sheet (from 0, among the control
/// sheets) and the place on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    /// On a power sheet (`sheet` counts the power sheets) and not on a control sheet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub power: bool,
    pub sheet: usize,
    pub x: f32,
    pub y: f32,
}

/// Offset of a device symbol from its automatic place, in units of 1/4 mm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SymbolShift {
    pub dx: f32,
    pub dy: f32,
    /// For a text: turned by 90°.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vertical: bool,
    /// For a text: the size of the font in units of 1/4 mm; 0 is the size the drawing gives it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl SymbolShift {
    fn is_default(&self) -> bool {
        self.dx == 0.0 && self.dy == 0.0 && !self.vertical && self.size == 0.0
    }
}

/// Place of the middle segment of a wire, in units of 1/4 mm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireShape {
    pub bend: f32,
}

/// Key of a device symbol on a view: `"<view>:<device id>"`. The view name must not contain
/// `:` (the device id is the part after the first `:`).
pub fn symbol_key(view: &str, device: &str) -> String {
    format!("{view}:{device}")
}

/// Snaps a coordinate to the grid; a value that is not finite counts as zero.
fn snap(value: f32) -> f32 {
    if value.is_finite() {
        (value / GRID).round() * GRID
    } else {
        0.0
    }
}

/// Key of a supply line of a power sheet that can be moved up and down by hand: `"bus:<sheet>:<line>"`
/// (the sheet counted from the first power sheet, the line from 0). Stored with the symbols.
pub fn bus_key(sheet: usize, line: usize) -> String {
    format!("bus:{sheet}:{line}")
}

/// Key of a text of a sheet that can be moved by hand (a designation, a rating, the number of a
/// wire...): `"label:<device id>:<what>"`. It is stored with the symbols.
pub fn label_key(device: &str, tag: &str) -> String {
    format!("label:{device}:{tag}")
}

/// Device id of a symbol key (`power:K1`) or of a label key (`label:K1:name`): the part after the
/// first `:`, up to the next one.
fn device_of_symbol(key: &str) -> Option<&str> {
    key.split_once(':')
        .map(|(_, rest)| rest.split_once(':').map_or(rest, |(device, _)| device))
}

/// Device ids of a wire key `A:pin>B:pin`: the part before the `:` of each end.
fn devices_of_wire(key: &str) -> Option<(&str, &str)> {
    let (a, b) = key.split_once('>')?;
    Some((a.split_once(':')?.0, b.split_once(':')?.0))
}

/// Symbol key with the device `old` replaced by `new`. A bus key has no device and stays as it is.
fn renamed_symbol(key: &str, old: &str, new: &str) -> String {
    if key.starts_with("bus:") {
        return key.to_owned();
    }
    if let Some(rest) = key.strip_prefix("label:") {
        return match rest.split_once(':') {
            Some((device, tag)) if device == old => {
                // The tag of a wire number is `#<wire key>`: the wire key follows the rename too.
                let tag = match tag.strip_prefix('#') {
                    Some(wire) => format!("#{}", renamed_wire(wire, old, new)),
                    None => tag.to_owned(),
                };
                format!("label:{new}:{tag}")
            }
            _ => key.to_owned(),
        };
    }
    match key.split_once(':') {
        Some((view, device)) if device == old => format!("{view}:{new}"),
        _ => key.to_owned(),
    }
}

/// Wire key with the device `old` replaced by `new` at each end; the pins stay.
fn renamed_wire(key: &str, old: &str, new: &str) -> String {
    key.split('>')
        .map(|end| match end.split_once(':') {
            Some((device, pin)) if device == old => format!("{new}:{pin}"),
            _ => end.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(">")
}

/// Re-keys the map with `renamed`. An entry that moves replaces the one already stored under its
/// new key.
fn rekey<V>(map: &mut BTreeMap<String, V>, renamed: impl Fn(&str) -> String) {
    let mut kept = BTreeMap::new();
    let mut moved = Vec::new();
    for (key, value) in std::mem::take(map) {
        let new_key = renamed(&key);
        if new_key == key {
            kept.insert(key, value);
        } else {
            moved.push((new_key, value));
        }
    }
    *map = kept;
    map.extend(moved);
}

impl PanelLayout {
    /// A layout with nothing in it is not stored.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
            && self.wires.is_empty()
            && self.bus_to.is_empty()
            && self.placed.is_empty()
    }

    /// Offset of the symbol from its automatic place; `(0, 0)` when it has none.
    pub fn shift(&self, key: &str) -> (f32, f32) {
        self.symbols.get(key).map_or((0.0, 0.0), |s| (s.dx, s.dy))
    }

    /// Sets the offset of the symbol, snapped to the grid. A zero offset removes the entry.
    pub fn set_shift(&mut self, key: &str, dx: f32, dy: f32) {
        let (dx, dy) = (snap(dx), snap(dy));
        let mut entry = self.symbols.get(key).copied().unwrap_or_default();
        (entry.dx, entry.dy) = (dx, dy);
        self.put(key, entry);
    }

    fn put(&mut self, key: &str, entry: SymbolShift) {
        if entry.is_default() {
            self.symbols.remove(key);
        } else {
            self.symbols.insert(key.to_owned(), entry);
        }
    }

    /// Turn and font size of a text: `(vertical, size)`, size 0 for the one of the drawing.
    pub fn label_style(&self, key: &str) -> (bool, f32) {
        self.symbols
            .get(key)
            .map_or((false, 0.0), |s| (s.vertical, s.size))
    }

    /// Sets the turn and the font size of a text; the shift stays.
    pub fn set_label_style(&mut self, key: &str, vertical: bool, size: f32) {
        let mut entry = self.symbols.get(key).copied().unwrap_or_default();
        entry.vertical = vertical;
        entry.size = if size.is_finite() && size > 0.0 {
            size.round().clamp(4.0, 80.0)
        } else {
            0.0
        };
        self.put(key, entry);
    }

    /// Place of the middle segment of the wire, if the user moved it.
    pub fn bend(&self, key: &str) -> Option<f32> {
        self.wires.get(key).map(|w| w.bend)
    }

    /// Sets the place of the middle segment, snapped to the grid; `None` removes the entry.
    pub fn set_bend(&mut self, key: &str, bend: Option<f32>) {
        match bend {
            Some(bend) => {
                self.wires
                    .insert(key.to_owned(), WireShape { bend: snap(bend) });
            }
            None => {
                self.wires.remove(key);
            }
        }
    }

    /// Drops the manual offset of one symbol.
    pub fn clear_symbol(&mut self, key: &str) {
        self.symbols.remove(key);
    }

    /// Drops the whole manual layout (symbols and wires). The devices put on the sheets by hand
    /// stay where they are.
    pub fn clear_all(&mut self) {
        self.symbols.clear();
        self.wires.clear();
        self.bus_to.clear();
    }

    /// Puts the device on a control sheet at a place; the place snaps to the grid.
    pub fn place(&mut self, device: &str, sheet: usize, x: f32, y: f32) {
        self.placed.insert(
            device.to_owned(),
            Placed {
                power: false,
                sheet,
                x: snap(x),
                y: snap(y),
            },
        );
        // The offset from an earlier place means nothing now.
        self.symbols.remove(&symbol_key("control", device));
    }

    /// Puts the device on a power sheet (counted among the power sheets) at a place.
    pub fn place_power(&mut self, device: &str, sheet: usize, x: f32, y: f32) {
        self.place(device, sheet, x, y);
        if let Some(p) = self.placed.get_mut(device) {
            p.power = true;
        }
        self.symbols.remove(&symbol_key("power", device));
    }

    /// Takes the device off the control sheets.
    pub fn unplace(&mut self, device: &str) {
        self.placed.remove(device);
        self.symbols.remove(&symbol_key("control", device));
    }

    /// The sheet the supply lines of the power sheet `index` (from 0) are carried on to by hand.
    pub fn bus_to(&self, index: usize) -> Option<usize> {
        self.bus_to.get(&index.to_string()).copied()
    }

    /// Carries the supply lines of the power sheet `index` on to the sheet number `to`; `None`
    /// takes the manual setting back.
    pub fn set_bus_to(&mut self, index: usize, to: Option<usize>) {
        match to {
            Some(to) => self.bus_to.insert(index.to_string(), to),
            None => self.bus_to.remove(&index.to_string()),
        };
    }

    /// Removes the entries that no longer match the panel: symbols of devices that the panel
    /// does not have, and wires that mention a device the panel does not have (or whose key is
    /// malformed, so it can never match a wire).
    pub fn prune(&mut self, panel: &Panel) {
        let exists = |id: &str| panel.device(id).is_some();
        self.symbols.retain(|key, _| {
            key.starts_with("bus:")
                || device_of_symbol(key).is_some_and(|d| d.is_empty() || exists(d))
        });
        self.wires
            .retain(|key, _| devices_of_wire(key).is_some_and(|(a, b)| exists(a) && exists(b)));
        self.placed.retain(|device, _| exists(device));
    }

    /// Makes the entries of device `old` follow its new designation `new` (symbols, labels, wires,
    /// placed devices). An entry already stored under the new key is replaced.
    pub fn rename(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        rekey(&mut self.symbols, |key| renamed_symbol(key, old, new));
        rekey(&mut self.wires, |key| renamed_wire(key, old, new));
        rekey(&mut self.placed, |device| {
            if device == old {
                new.to_owned()
            } else {
                device.to_owned()
            }
        });
    }

    /// Renames several devices at once, also swaps (`K1`↔`K2`), as `Panel::rename_all` does.
    /// Each device first gets a temporary name, so no name is taken on the way.
    pub fn rename_all(&mut self, names: &[(String, String)]) {
        let names: Vec<&(String, String)> = names.iter().filter(|(o, n)| o != n).collect();
        for (i, (old, _)) in names.iter().enumerate() {
            self.rename(old, &format!("tmp_rename_{i}"));
        }
        for (i, (_, new)) in names.iter().enumerate() {
            self.rename(&format!("tmp_rename_{i}"), new);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::{ContactForm, Device, DeviceKind};

    fn panel_with(ids: &[&str]) -> Panel {
        Panel {
            devices: ids
                .iter()
                .map(|id| Device {
                    id: (*id).into(),
                    kind: DeviceKind::Contactor,
                    rating: String::new(),
                    part: String::new(),
                    bind: String::new(),
                    load: String::new(),
                    poles: None,
                    of: String::new(),
                    form: ContactForm::default(),
                    pins: String::new(),
                    from: String::new(),
                    free: false,
                    series: String::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn shifts_snap_to_the_grid() {
        let mut layout = PanelLayout::default();
        layout.set_shift("sheet:K1", 7.4, -12.6);
        assert_eq!(layout.shift("sheet:K1"), (5.0, -15.0));
        assert_eq!(layout.shift("sheet:M1"), (0.0, 0.0));
        layout.set_shift("sheet:K1", f32::NAN, 2.4);
        assert_eq!(layout.shift("sheet:K1"), (0.0, 0.0));
        assert!(layout.is_empty());
    }

    #[test]
    fn zero_shift_removes_the_entry() {
        let mut layout = PanelLayout::default();
        layout.set_shift("sheet:K1", 10.0, 0.0);
        assert!(!layout.is_empty());
        layout.set_shift("sheet:K1", 2.0, -1.0);
        assert!(layout.is_empty());
    }

    #[test]
    fn bends_snap_and_none_removes() {
        let mut layout = PanelLayout::default();
        layout.set_bend("K1:2>M1:L1", Some(-7.0));
        assert_eq!(layout.bend("K1:2>M1:L1"), Some(-5.0));
        layout.set_bend("K1:2>M1:L1", None);
        assert_eq!(layout.bend("K1:2>M1:L1"), None);
        assert!(layout.is_empty());
    }

    #[test]
    fn clear_symbol_and_clear_all() {
        let mut layout = PanelLayout::default();
        layout.set_shift("a:K1", 5.0, 5.0);
        layout.set_shift("a:K2", 5.0, 5.0);
        layout.set_bend("K1:2>M1:L1", Some(5.0));
        layout.clear_symbol("a:K1");
        assert_eq!(layout.shift("a:K1"), (0.0, 0.0));
        assert_eq!(layout.shift("a:K2"), (5.0, 5.0));
        layout.clear_all();
        assert!(layout.is_empty());
    }

    #[test]
    fn prune_drops_unknown_devices() {
        let panel = panel_with(&["K1", "M1"]);
        let mut layout = PanelLayout::default();
        layout.set_shift("a:K1", 5.0, 0.0);
        layout.set_shift("a:G9", 5.0, 0.0);
        layout.set_shift("no-colon", 5.0, 0.0);
        layout.set_bend("K1:2>M1:L1", Some(5.0));
        layout.set_bend("K1:2>G9:L1", Some(5.0));
        layout.set_bend("broken", Some(5.0));
        layout.prune(&panel);
        assert_eq!(layout.symbols.keys().collect::<Vec<_>>(), vec!["a:K1"]);
        assert_eq!(layout.wires.keys().collect::<Vec<_>>(), vec!["K1:2>M1:L1"]);
    }

    #[test]
    fn toml_round_trip() {
        let mut layout = PanelLayout::default();
        layout.set_shift("a:K1", 5.0, -10.0);
        layout.set_bend("K1:2>M1:L1", Some(15.0));
        let text = toml::to_string(&layout).unwrap();
        let again: PanelLayout = toml::from_str(&text).unwrap();
        assert_eq!(again, layout);
        assert_eq!(toml::to_string(&PanelLayout::default()).unwrap(), "");
    }

    #[test]
    fn labels_are_pruned_with_their_device() {
        let panel = panel_with(&["K1"]);
        let mut layout = PanelLayout::default();
        layout.set_shift(&label_key("K1", "name"), 10.0, 0.0);
        layout.set_shift(&label_key("K1", "#K1:2>M1:L1"), 10.0, 0.0);
        layout.set_shift(&label_key("Q9", "name"), 10.0, 0.0);
        layout.prune(&panel);
        assert_eq!(layout.symbols.len(), 2);
        assert_eq!(layout.shift("label:K1:name"), (10.0, 0.0));
    }

    #[test]
    fn bus_to_survives_a_toml_round_trip() {
        let mut layout = PanelLayout::default();
        layout.set_bus_to(0, Some(4));
        layout.set_label_style("label:K1:name", true, 20.0);
        let text = toml::to_string(&layout).unwrap();
        let back: PanelLayout = toml::from_str(&text).unwrap();
        assert_eq!(back, layout);
        assert_eq!(back.bus_to(0), Some(4));
    }

    #[test]
    fn rename_moves_symbols_labels_wires_and_placed_devices() {
        let mut layout = PanelLayout::default();
        layout.place("K1", 0, 40.0, 20.0);
        layout.set_shift(&symbol_key("power", "K1"), 5.0, 5.0);
        layout.set_shift(&symbol_key("power", "K10"), 10.0, 0.0);
        layout.set_shift(&label_key("K1", "name"), 5.0, -5.0);
        layout.set_bend("K1:2>M1:L1", Some(15.0));
        layout.set_bend("K10:2>K1:3", Some(-10.0));
        layout.set_bend("K1:4>K10:1", Some(20.0));
        layout.rename("K1", "K5");
        assert_eq!(layout.shift("power:K5"), (5.0, 5.0));
        assert_eq!(layout.shift("power:K1"), (0.0, 0.0));
        assert_eq!(layout.shift("power:K10"), (10.0, 0.0));
        assert_eq!(layout.shift(&label_key("K5", "name")), (5.0, -5.0));
        assert_eq!(layout.shift(&label_key("K1", "name")), (0.0, 0.0));
        assert_eq!(layout.bend("K5:2>M1:L1"), Some(15.0));
        assert_eq!(layout.bend("K1:2>M1:L1"), None);
        assert_eq!(layout.bend("K10:2>K5:3"), Some(-10.0));
        assert_eq!(layout.bend("K5:4>K10:1"), Some(20.0));
        assert_eq!(layout.placed.keys().collect::<Vec<_>>(), vec!["K5"]);
        assert_eq!(layout.placed["K5"].sheet, 0);
    }

    #[test]
    fn rename_replaces_an_entry_stored_under_the_new_key() {
        let mut layout = PanelLayout::default();
        layout.set_shift("power:K1", 5.0, 0.0);
        layout.set_shift("power:K5", 0.0, 10.0);
        layout.rename("K1", "K5");
        assert_eq!(layout.shift("power:K5"), (5.0, 0.0));
        assert_eq!(layout.symbols.len(), 1);
    }

    #[test]
    fn bus_keys_are_not_renamed() {
        let mut layout = PanelLayout::default();
        layout.set_shift(&bus_key(0, 1), 0.0, 5.0);
        // A bus key has no device: even a name equal to its sheet number leaves it alone.
        layout.rename("0", "K9");
        assert_eq!(layout.shift(&bus_key(0, 1)), (0.0, 5.0));
        layout.rename("K1", "K9");
        assert_eq!(layout.shift(&bus_key(0, 1)), (0.0, 5.0));
    }

    #[test]
    fn rename_all_swaps_and_keeps_each_shift() {
        let mut layout = PanelLayout::default();
        layout.set_shift("power:K1", 5.0, 0.0);
        layout.set_shift("power:K2", 0.0, 10.0);
        layout.set_bend("K1:2>K2:1", Some(5.0));
        layout.rename_all(&[("K1".into(), "K2".into()), ("K2".into(), "K1".into())]);
        assert_eq!(layout.shift("power:K2"), (5.0, 0.0));
        assert_eq!(layout.shift("power:K1"), (0.0, 10.0));
        assert_eq!(layout.bend("K2:2>K1:1"), Some(5.0));
        assert_eq!(layout.bend("K1:2>K2:1"), None);
    }
}
