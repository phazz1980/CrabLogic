//! Manual layout of the electrical-panel sheets (`panel-layout.toml`): where the user moved a
//! device symbol or the middle segment of a wire, relative to the automatic place
//! (`specs/panel-editor`). Stored as offsets, so an automatic layout that moves when devices
//! are added keeps the manual part relative to it.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::panel::Panel;

/// Entry of the project archive (and file of the folder) with the manual layout; written only
/// when the layout is not empty.
pub const PANEL_LAYOUT_FILE: &str = "panel-layout.toml";

/// Grid of the panel sheets in units of 1/4 mm: half a module (2.5 mm) is 10 u, so the
/// positions snap to 5 u.
const GRID: f32 = 5.0;

/// Most sheets of one kind that the layout names (a guard against a runaway number).
pub const MAX_SHEETS: usize = 100;

/// Name of a drawing sheet. It does not change when sheets are added: the power sheet `i` and the
/// control sheet `j` are counted inside their group, as the generator makes them; the free sheet
/// `k` is one of the sheets the user adds.
///
/// Text form: `power:0`, `control:1`, `free:2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SheetId {
    Power(usize),
    Control(usize),
    Free(usize),
}

impl SheetId {
    /// The number of the sheet inside its kind (`free:2` gives 2).
    pub fn number(self) -> usize {
        match self {
            Self::Power(n) | Self::Control(n) | Self::Free(n) => n,
        }
    }
}

impl fmt::Display for SheetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Power(n) => write!(f, "power:{n}"),
            Self::Control(n) => write!(f, "control:{n}"),
            Self::Free(n) => write!(f, "free:{n}"),
        }
    }
}

impl FromStr for SheetId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let Some((kind, number)) = text.split_once(':') else {
            return Err(format!(
                "лист «{text}»: нужно имя вида power:0, control:0 или free:0"
            ));
        };
        let number: usize = number
            .parse()
            .map_err(|_| format!("лист «{text}»: номер листа должен быть целым числом"))?;
        match kind {
            "power" => Ok(Self::Power(number)),
            "control" => Ok(Self::Control(number)),
            "free" => Ok(Self::Free(number)),
            _ => Err(format!(
                "лист «{text}»: неизвестный вид листа, нужен power, control или free"
            )),
        }
    }
}

impl Serialize for SheetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SheetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Manual layout of the panel sheets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PanelLayout {
    /// key = "<view>:<device id>" (see `symbol_key`)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbols: BTreeMap<String, SymbolShift>,
    /// key = wire key as `crab_schematic::nets::Wire::key()` gives it (e.g. `K1:2>M1:L1`)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wires: BTreeMap<String, WireShape>,
    /// Supply lines carried on by hand: key = the power sheet (from 0) as text, as TOML keys are
    /// strings; value = the number of the target sheet in the document (the title sheet is 1).
    /// The lines go straight there, not through the sheets between.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bus_to: BTreeMap<String, usize>,
    /// Devices put on a sheet by hand: key = the device id (or a part of it, `K1/coil`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub placed: BTreeMap<String, Placed>,
    /// Captions of the sheets: key = the `SheetId` as text (`free:0`), value = the caption.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub captions: BTreeMap<String, String>,
}

/// Where a device was put on a drawing sheet by hand: the sheet and the place on it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PlacedFile")]
pub struct Placed {
    pub sheet: SheetId,
    pub x: f32,
    pub y: f32,
}

/// The entry of a device as it is read: the new form (`sheet = "control:0"`) and the old one
/// (`sheet = 0` with `power = true` for a power sheet, counted inside its group).
#[derive(Deserialize)]
struct PlacedFile {
    /// Old form only: `true` when the number of `sheet` counts the power sheets.
    #[serde(default)]
    power: bool,
    sheet: SheetRef,
    x: f32,
    y: f32,
}

/// The `sheet` value of an entry: a name, or the number of an old entry.
#[derive(Deserialize)]
#[serde(untagged)]
enum SheetRef {
    Name(String),
    Number(usize),
}

impl TryFrom<PlacedFile> for Placed {
    type Error = String;

    fn try_from(file: PlacedFile) -> Result<Self, Self::Error> {
        let sheet = match file.sheet {
            SheetRef::Name(text) => text.parse::<SheetId>()?,
            // An old entry: the number counts inside its group, `power` tells the group.
            SheetRef::Number(n) if file.power => SheetId::Power(n),
            SheetRef::Number(n) => SheetId::Control(n),
        };
        Ok(Self {
            sheet,
            x: file.x,
            y: file.y,
        })
    }
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
    /// Place of the middle segment, if the user moved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend: Option<f32>,
    /// The wire is drawn as a break with references at both ends instead of a line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub broken: bool,
}

/// Key of a device symbol on a view: `"<view>:<device id>"`. The view name must not contain
/// `:` (the device id is the part after the first `:`).
pub fn symbol_key(view: &str, device: &str) -> String {
    format!("{view}:{device}")
}

/// Name of the coil part of a contactor or relay.
pub const COIL: &str = "coil";

/// Key of a part of a device: `"K1/coil"`. Device ids never contain `/`, so the part is the
/// text after the first `/`.
pub fn part_key(device: &str, part: &str) -> String {
    format!("{device}/{part}")
}

/// Splits a key of a device or of a part: `"K1/coil"` → `("K1", Some("coil"))`, `"K1"` → `("K1", None)`.
/// An empty device or part (`"/coil"`, `"K1/"`) is malformed: it matches no device of the panel.
pub fn split_part(key: &str) -> (&str, Option<&str>) {
    match key.split_once('/') {
        Some((device, part)) => (device, Some(part)),
        None => (key, None),
    }
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

/// Device piece of a key (`K1` or the device of a part `K1/coil`) with the device `old` replaced
/// by `new`; the part stays. `None` when the piece belongs to another device.
fn renamed_piece(piece: &str, old: &str, new: &str) -> Option<String> {
    let (device, part) = split_part(piece);
    if device != old {
        return None;
    }
    Some(match part {
        Some(part) => part_key(new, part),
        None => new.to_owned(),
    })
}

/// Symbol key with the device `old` replaced by `new`. A bus key has no device and stays as it is.
fn renamed_symbol(key: &str, old: &str, new: &str) -> String {
    if key.starts_with("bus:") {
        return key.to_owned();
    }
    if let Some(rest) = key.strip_prefix("label:") {
        return match rest.split_once(':') {
            Some((piece, tag)) => match renamed_piece(piece, old, new) {
                Some(device) => {
                    // The tag of a wire number is `#<wire key>`: the wire key follows the rename too.
                    let tag = match tag.strip_prefix('#') {
                        Some(wire) => format!("#{}", renamed_wire(wire, old, new)),
                        None => tag.to_owned(),
                    };
                    format!("label:{device}:{tag}")
                }
                None => key.to_owned(),
            },
            None => key.to_owned(),
        };
    }
    match key.split_once(':') {
        Some((view, piece)) => match renamed_piece(piece, old, new) {
            Some(device) => format!("{view}:{device}"),
            None => key.to_owned(),
        },
        None => key.to_owned(),
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
            && self.captions.is_empty()
    }

    /// Caption the user gave the sheet; `None` when there is none.
    pub fn caption(&self, sheet: SheetId) -> Option<&str> {
        self.captions
            .get(&sheet.to_string())
            .map(String::as_str)
            .filter(|caption| !caption.is_empty())
    }

    /// Sets the caption of the sheet (trimmed; an empty one removes it). A sheet beyond
    /// `MAX_SHEETS` is ignored.
    pub fn set_caption(&mut self, sheet: SheetId, caption: &str) {
        if sheet.number() >= MAX_SHEETS {
            return;
        }
        let key = sheet.to_string();
        let caption = caption.trim();
        if caption.is_empty() {
            self.captions.remove(&key);
        } else {
            self.captions.insert(key, caption.to_owned());
        }
    }

    /// Removes the free sheet `k`: its caption and the devices put on it by hand go, the later
    /// free sheets move one number down (placed devices and captions). The generator's sheets
    /// cannot be removed, so only free sheets come here.
    pub fn remove_free_sheet(&mut self, k: usize) {
        self.placed
            .retain(|_, placed| placed.sheet != SheetId::Free(k));
        for placed in self.placed.values_mut() {
            if let SheetId::Free(n) = placed.sheet {
                if n > k {
                    placed.sheet = SheetId::Free(n.saturating_sub(1));
                }
            }
        }
        let mut captions = BTreeMap::new();
        for (key, caption) in std::mem::take(&mut self.captions) {
            match key.parse::<SheetId>() {
                Ok(SheetId::Free(n)) if n == k => {}
                Ok(SheetId::Free(n)) if n > k => {
                    captions.insert(SheetId::Free(n.saturating_sub(1)).to_string(), caption);
                }
                _ => {
                    captions.insert(key, caption);
                }
            }
        }
        self.captions = captions;
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
        self.wires.get(key).and_then(|w| w.bend)
    }

    /// Whether the wire is drawn as a break with references.
    pub fn is_broken(&self, key: &str) -> bool {
        self.wires.get(key).is_some_and(|w| w.broken)
    }

    /// Turns the wire into a break with references (or back into a line).
    pub fn set_broken(&mut self, key: &str, broken: bool) {
        if broken {
            self.wires
                .entry(key.to_owned())
                .or_insert(WireShape { bend: None, broken })
                .broken = true;
        } else if let Some(w) = self.wires.get_mut(key) {
            w.broken = false;
            if w.bend.is_none() {
                self.wires.remove(key);
            }
        }
    }

    /// Sets the place of the middle segment, snapped to the grid; `None` removes the entry.
    pub fn set_bend(&mut self, key: &str, bend: Option<f32>) {
        match bend {
            Some(bend) => {
                let broken = self.is_broken(key);
                self.wires.insert(
                    key.to_owned(),
                    WireShape {
                        bend: Some(snap(bend)),
                        broken,
                    },
                );
            }
            None => {
                if let Some(w) = self.wires.get_mut(key).filter(|w| w.broken) {
                    w.bend = None;
                } else {
                    self.wires.remove(key);
                }
            }
        }
    }

    /// Drops the manual offset of one symbol.
    pub fn clear_symbol(&mut self, key: &str) {
        self.symbols.remove(key);
    }

    /// Drops the whole manual layout (symbols and wires). The devices put on the sheets by hand
    /// and the sheet captions stay where they are.
    pub fn clear_all(&mut self) {
        self.symbols.clear();
        self.wires.clear();
        self.bus_to.clear();
    }

    /// Puts the device on the sheet at a place; the place snaps to the grid. The offset of the
    /// symbol from an earlier automatic place is dropped (the control one, and the power one when
    /// the sheet is a power sheet). `device` may be a part key (`K1/coil`). A sheet beyond
    /// `MAX_SHEETS` is ignored.
    pub fn place(&mut self, device: &str, sheet: SheetId, x: f32, y: f32) {
        if sheet.number() >= MAX_SHEETS {
            return;
        }
        self.placed.insert(
            device.to_owned(),
            Placed {
                sheet,
                x: snap(x),
                y: snap(y),
            },
        );
        self.symbols.remove(&symbol_key("control", device));
        if matches!(sheet, SheetId::Power(_)) {
            self.symbols.remove(&symbol_key("power", device));
        }
    }

    /// Takes the device off the sheets. `device` may be a part key (`K1/coil`).
    pub fn unplace(&mut self, device: &str) {
        self.placed.remove(device);
        self.symbols.remove(&symbol_key("control", device));
    }

    /// The document sheet the supply lines of the power sheet `index` (from 0) are carried on to
    /// by hand (see the field `bus_to`).
    pub fn bus_to(&self, index: usize) -> Option<usize> {
        self.bus_to.get(&index.to_string()).copied()
    }

    /// Carries the supply lines of the power sheet `index` on to the document sheet `to`; `None`
    /// takes the manual setting back.
    pub fn set_bus_to(&mut self, index: usize, to: Option<usize>) {
        match to {
            Some(to) => self.bus_to.insert(index.to_string(), to),
            None => self.bus_to.remove(&index.to_string()),
        };
    }

    /// Removes the entries that no longer match the panel: symbols and placed devices of devices
    /// that the panel does not have (a part such as `K1/coil` belongs to its device `K1`), and
    /// wires that mention a device the panel does not have (or whose key is malformed, so it can
    /// never match a wire).
    pub fn prune(&mut self, panel: &Panel) {
        let exists = |id: &str| panel.device(split_part(id).0).is_some();
        self.symbols.retain(|key, _| {
            key.starts_with("bus:")
                || device_of_symbol(key).is_some_and(|d| d.is_empty() || exists(d))
        });
        // Wire keys name devices, never parts (a terminal belongs to the device: `K1:A1`).
        let device = |id: &str| panel.device(id).is_some();
        self.wires
            .retain(|key, _| devices_of_wire(key).is_some_and(|(a, b)| device(a) && device(b)));
        self.placed.retain(|device, _| exists(device));
    }

    /// Makes the entries of device `old` follow its new designation `new` (symbols, labels, wires,
    /// placed devices; the parts of the device such as `K1/coil` too). An entry already stored
    /// under the new key is replaced.
    pub fn rename(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        rekey(&mut self.symbols, |key| renamed_symbol(key, old, new));
        rekey(&mut self.wires, |key| renamed_wire(key, old, new));
        rekey(&mut self.placed, |device| {
            renamed_piece(device, old, new).unwrap_or_else(|| device.to_owned())
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
                    via: String::new(),
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
                    conductors: Vec::new(),
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
        layout.place("K2", SheetId::Free(1), 10.0, 10.0);
        layout.set_caption(SheetId::Control(0), "Drives");
        let text = toml::to_string(&layout).unwrap();
        let again: PanelLayout = toml::from_str(&text).unwrap();
        assert_eq!(again, layout);
        assert_eq!(toml::to_string(&PanelLayout::default()).unwrap(), "");
    }

    #[test]
    fn sheet_names_have_a_text_form_that_round_trips() {
        assert_eq!(SheetId::Power(0).to_string(), "power:0");
        assert_eq!(SheetId::Control(1).to_string(), "control:1");
        assert_eq!(SheetId::Free(2).to_string(), "free:2");
        for id in [SheetId::Power(3), SheetId::Control(0), SheetId::Free(12)] {
            assert_eq!(id.to_string().parse::<SheetId>(), Ok(id));
        }
        assert_eq!(SheetId::Free(4).number(), 4);
        assert!("drive:1".parse::<SheetId>().unwrap_err().contains("drive"));
        assert!("free".parse::<SheetId>().is_err());
        assert!("free:x".parse::<SheetId>().is_err());
    }

    #[test]
    fn placed_is_written_by_name_and_old_entries_read_as_before() {
        let mut layout = PanelLayout::default();
        layout.place("K1", SheetId::Control(0), 10.0, 10.0);
        let text = toml::to_string(&layout).unwrap();
        assert!(text.contains("sheet = \"control:0\""), "{text}");
        assert!(!text.contains("power"), "{text}");
        // A file of the old numbering: `power` tells the power sheets, the number counts inside
        // the group; a file of the version that had no `version` key at all reads the same way.
        let old: PanelLayout = toml::from_str(
            "version = 0
[placed.K2]
power = true
sheet = 1
x = 0.0
y = 0.0

[placed.K3]
power = false
sheet = 2
x = 5.0
y = 0.0
",
        )
        .unwrap();
        assert_eq!(old.placed["K2"].sheet, SheetId::Power(1));
        assert_eq!(old.placed["K3"].sheet, SheetId::Control(2));
        // The new form reads too, and gives back the same entries.
        let back: PanelLayout = toml::from_str(&toml::to_string(&old).unwrap()).unwrap();
        assert_eq!(back, old);
        assert!(toml::from_str::<PanelLayout>(
            "[placed.K2]\nsheet = \"lamp:1\"\nx = 0.0\ny = 0.0\n"
        )
        .is_err());
    }

    #[test]
    fn captions_are_trimmed_by_name_and_an_empty_one_removes() {
        let mut layout = PanelLayout::default();
        layout.set_caption(SheetId::Free(2), "  Drives  ");
        assert_eq!(layout.caption(SheetId::Free(2)), Some("Drives"));
        assert_eq!(layout.caption(SheetId::Free(0)), None);
        assert_eq!(layout.caption(SheetId::Control(2)), None);
        assert!(!layout.is_empty());
        layout.clear_all();
        assert_eq!(layout.caption(SheetId::Free(2)), Some("Drives"));
        layout.set_caption(SheetId::Free(2), "   ");
        assert_eq!(layout.caption(SheetId::Free(2)), None);
        assert!(layout.is_empty());
        // A caption of a sheet beyond the guard is not kept.
        layout.set_caption(SheetId::Free(MAX_SHEETS), "x");
        layout.set_caption(SheetId::Power(usize::MAX), "x");
        assert!(layout.captions.is_empty());
    }

    #[test]
    fn remove_free_sheet_moves_the_later_free_sheets() {
        let mut layout = PanelLayout::default();
        layout.set_caption(SheetId::Power(0), "Power");
        layout.set_caption(SheetId::Control(0), "Control");
        layout.set_caption(SheetId::Free(0), "Zero");
        layout.set_caption(SheetId::Free(1), "One");
        layout.set_caption(SheetId::Free(2), "Two");
        layout.place("K1", SheetId::Free(1), 10.0, 10.0);
        layout.place("K2", SheetId::Free(2), 20.0, 10.0);
        layout.place("K3", SheetId::Control(0), 30.0, 10.0);
        layout.remove_free_sheet(1);
        assert_eq!(layout.caption(SheetId::Power(0)), Some("Power"));
        assert_eq!(layout.caption(SheetId::Control(0)), Some("Control"));
        assert_eq!(layout.caption(SheetId::Free(0)), Some("Zero"));
        assert_eq!(layout.caption(SheetId::Free(1)), Some("Two"));
        assert_eq!(layout.caption(SheetId::Free(2)), None);
        assert!(!layout.placed.contains_key("K1"));
        assert_eq!(layout.placed["K2"].sheet, SheetId::Free(1));
        assert_eq!(layout.placed["K3"].sheet, SheetId::Control(0));
    }

    #[test]
    fn place_drops_the_offsets_of_the_automatic_place() {
        let mut layout = PanelLayout::default();
        layout.set_shift("power:K1", 5.0, 5.0);
        layout.set_shift("control:K1", 10.0, 0.0);
        layout.place("K1", SheetId::Control(0), 0.0, 0.0);
        assert_eq!(layout.shift("power:K1"), (5.0, 5.0));
        assert_eq!(layout.shift("control:K1"), (0.0, 0.0));
        layout.place("K1", SheetId::Power(1), 0.0, 0.0);
        assert_eq!(layout.shift("power:K1"), (0.0, 0.0));
        layout.set_shift("power:K2", 5.0, 0.0);
        layout.place("K2", SheetId::Free(0), 0.0, 0.0);
        assert_eq!(layout.shift("power:K2"), (5.0, 0.0));
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
        layout.place("K1", SheetId::Power(0), 40.0, 20.0);
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
        assert_eq!(layout.placed["K5"].sheet, SheetId::Power(0));
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

    #[test]
    fn part_key_and_split_part_round_trip() {
        assert_eq!(part_key("K1", COIL), "K1/coil");
        assert_eq!(split_part("K1/coil"), ("K1", Some("coil")));
        assert_eq!(split_part("K1"), ("K1", None));
        assert_eq!(split_part(&part_key("K10", "coil")), ("K10", Some("coil")));
    }

    #[test]
    fn prune_keeps_parts_of_existing_devices_only() {
        let panel = panel_with(&["K1"]);
        let mut layout = PanelLayout::default();
        // `place` drops the control offset of the device, so the places come first.
        layout.place(&part_key("K1", COIL), SheetId::Control(0), 10.0, 10.0);
        layout.place(&part_key("Q9", COIL), SheetId::Control(0), 10.0, 10.0);
        layout.set_shift(&symbol_key("control", &part_key("K1", COIL)), 5.0, 0.0);
        layout.set_shift(&label_key(&part_key("K1", COIL), "name"), 5.0, 0.0);
        layout.set_shift(&symbol_key("control", &part_key("Q9", COIL)), 5.0, 0.0);
        layout.set_shift(&label_key(&part_key("Q9", COIL), "name"), 5.0, 0.0);
        layout.prune(&panel);
        assert_eq!(
            layout.symbols.keys().collect::<Vec<_>>(),
            vec!["control:K1/coil", "label:K1/coil:name"]
        );
        assert_eq!(layout.placed.keys().collect::<Vec<_>>(), vec!["K1/coil"]);
    }

    #[test]
    fn rename_moves_part_entries_and_leaves_similar_devices_alone() {
        let mut layout = PanelLayout::default();
        layout.place(&part_key("K1", COIL), SheetId::Control(0), 40.0, 20.0);
        layout.place(&part_key("K10", COIL), SheetId::Control(0), 60.0, 20.0);
        layout.set_shift(&symbol_key("control", &part_key("K1", COIL)), 5.0, 5.0);
        layout.set_shift(&symbol_key("control", &part_key("K10", COIL)), 10.0, 0.0);
        layout.set_shift(&label_key(&part_key("K1", COIL), "name"), 5.0, -5.0);
        layout.rename("K1", "K5");
        assert_eq!(layout.shift("control:K5/coil"), (5.0, 5.0));
        assert_eq!(layout.shift("control:K1/coil"), (0.0, 0.0));
        assert_eq!(layout.shift("control:K10/coil"), (10.0, 0.0));
        assert_eq!(layout.shift("label:K5/coil:name"), (5.0, -5.0));
        assert_eq!(layout.shift("label:K1/coil:name"), (0.0, 0.0));
        assert_eq!(
            layout.placed.keys().collect::<Vec<_>>(),
            vec!["K10/coil", "K5/coil"]
        );
        assert_eq!(layout.placed["K5/coil"].x, 40.0);
    }

    #[test]
    fn rename_all_swaps_devices_with_their_parts() {
        let mut layout = PanelLayout::default();
        layout.place(&part_key("K1", COIL), SheetId::Control(0), 40.0, 20.0);
        layout.place(&part_key("K2", COIL), SheetId::Control(0), 80.0, 20.0);
        layout.set_shift(&symbol_key("control", &part_key("K1", COIL)), 5.0, 0.0);
        layout.set_shift(&symbol_key("control", &part_key("K2", COIL)), 0.0, 10.0);
        layout.rename_all(&[("K1".into(), "K2".into()), ("K2".into(), "K1".into())]);
        assert_eq!(layout.shift("control:K2/coil"), (5.0, 0.0));
        assert_eq!(layout.shift("control:K1/coil"), (0.0, 10.0));
        assert_eq!(layout.placed["K2/coil"].x, 40.0);
        assert_eq!(layout.placed["K1/coil"].x, 80.0);
    }
}
