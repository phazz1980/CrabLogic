//! Electrical panel of a project (`panel.toml`): the devices of the cabinet and what they are
//! bound to (`specs/electrical-schematics`). The panel does not change the schema or the
//! firmware: it only feeds the electrical documentation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Entry of the project archive with the panel; written only when the panel is not empty.
pub const PANEL_FILE: &str = "panel.toml";

/// Entry of the project archive with the documentation of the panel as a PDF (made by the editor
/// at saving, only for a panel that is not empty).
pub const PANEL_PDF: &str = "panel.pdf";

/// Supply of the panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Supply {
    /// 24 V DC.
    Dc24,
    /// 230 V AC, one phase.
    #[default]
    Ac230,
    /// 400 V AC, three phases.
    Ac400,
}

impl Supply {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The numbers of poles a switching device may have on this supply: `1` or `2` (with the
    /// neutral, or a pair of phases on 400 V) on a one-phase supply, `1` to `4` on 400 V.
    pub fn pole_range(self) -> (u8, u8) {
        match self {
            Self::Dc24 | Self::Ac230 => (1, 2),
            Self::Ac400 => (1, 4),
        }
    }

    /// The conductors of the supply, in the order of the poles: the names a `bus:<name>` link end
    /// can give (`L1`, `N`); a 24 V supply has its `+24V` and `0V`.
    pub fn lines(self) -> &'static [&'static str] {
        match self {
            Self::Dc24 => &["+24V", "0V"],
            Self::Ac230 => &["L", "N"],
            Self::Ac400 => &["L1", "L2", "L3", "N"],
        }
    }
}

/// Kind of a panel device; the letter code of the designation follows IEC 81346.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// Incoming supply (`-X`).
    Feed,
    /// Circuit breaker (`-Q`).
    Breaker,
    /// Residual-current device (`-Q`).
    Rcd,
    /// Fuses, one in each conductor (`-F`).
    Fuse,
    /// Disconnector: a switch for all conductors without protection (`-Q`).
    Switch,
    /// Power supply unit (`-G`).
    Psu,
    /// Contactor (`-K`).
    Contactor,
    /// Relay (`-K`).
    Relay,
    /// Motor (`-M`).
    Motor,
    /// Other load: heater, lamp, valve (`-E`, `-Y`).
    Load,
    /// Terminal block (`-X`).
    Terminal,
    /// Sensor (`-B`).
    Sensor,
    /// Push button or switch (`-S`).
    Button,
    /// Auxiliary contact of a breaker, RCD, contactor or relay (`of`); drawn with the designation
    /// of that device.
    Aux,
}

/// Form of an auxiliary contact.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactForm {
    /// Normally open: terminals `13-14`, `23-24`, ...
    #[default]
    No,
    /// Normally closed: terminals `21-22`, `31-32`, ...
    Nc,
    /// Changeover: common `k1`, normally closed `k2`, normally open `k4`, e.g. `11-12-14`.
    Changeover,
}

impl ContactForm {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl DeviceKind {
    /// Every kind, in the order of the list of types in the editor.
    pub const ALL: [DeviceKind; 11] = [
        Self::Feed,
        Self::Breaker,
        Self::Rcd,
        Self::Psu,
        Self::Contactor,
        Self::Relay,
        Self::Motor,
        Self::Load,
        Self::Terminal,
        Self::Sensor,
        Self::Button,
    ];

    /// Letter of the designation (`K` for `-K1`).
    pub fn letter(self) -> char {
        match self {
            Self::Feed | Self::Terminal => 'X',
            Self::Breaker | Self::Rcd | Self::Switch => 'Q',
            Self::Fuse => 'F',
            Self::Psu => 'G',
            Self::Contactor | Self::Relay => 'K',
            Self::Motor => 'M',
            Self::Load => 'E',
            Self::Sensor => 'B',
            Self::Button => 'S',
            Self::Aux => 'K',
        }
    }

    /// Name in the file, e.g. `contactor`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::Breaker => "breaker",
            Self::Rcd => "rcd",
            Self::Fuse => "fuse",
            Self::Switch => "switch",
            Self::Psu => "psu",
            Self::Contactor => "contactor",
            Self::Relay => "relay",
            Self::Motor => "motor",
            Self::Load => "load",
            Self::Terminal => "terminal",
            Self::Sensor => "sensor",
            Self::Button => "button",
            Self::Aux => "aux",
        }
    }
}

/// A device of the panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Designation without the dash (`K1` for `-K1`); unique in the panel.
    pub id: String,
    pub kind: DeviceKind,
    /// Rating for the drawing and the bill of materials (`230 В, 9 А`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rating: String,
    /// Part number, typed by hand.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub part: String,
    /// What the device is bound to: an output or input of the project (`block.port`) or the
    /// name of a tag. Empty for devices the schema does not drive (feed, breaker, motor).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bind: String,
    /// Designation of the device this one switches or feeds (`M1` for contactor `K1`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub load: String,
    /// Number of conductors a breaker, RCD, contactor or relay switches (`1`, `2` on 230 V and
    /// 24 V; `1` to `4` on 400 V); the devices under it get only these conductors. Empty: all
    /// conductors of the supply (`4` on 400 V).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poles: Option<u8>,
    /// For an auxiliary contact: the device it belongs to (`Q2`). For a sensor: the motor whose
    /// temperature it measures (a thermistor in the winding).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub of: String,
    /// For an auxiliary contact: normally open or normally closed.
    #[serde(default, skip_serializing_if = "ContactForm::is_default")]
    pub form: ContactForm,
    /// For an auxiliary contact: its terminal numbers by hand (`21-22`). Empty: by the number of
    /// the contact (`13-14`, `21-22`, ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pins: String,
    /// The device that feeds this one when it already feeds another through its `load`: a source
    /// of several devices names the first by `load` and the others by `from` (a branch of the line).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// A device just added, not connected yet: it stays off the sheets until something feeds it
    /// (or it is connected straight to the feed). Devices of old projects are never free.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub free: bool,
    /// For an auxiliary contact: the contactor or relay whose coil circuit it stands in, in series
    /// with the coil. Empty: the contact goes to an input of the board (`bind`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub series: String,
    /// The conductors this device takes on a 400 V supply, in the order of the buses (`["L2"]`,
    /// `["L3","N"]`, `["L1","L3"]`). Empty: by default (spec panel-phase).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conductors: Vec<String>,
}

impl Device {
    /// Designation as drawn: `-K1`.
    pub fn designation(&self) -> String {
        format!("-{}", self.id)
    }
}

/// A wire drawn by hand between two terminals: `device:terminal` at each end (`K1:A2`, `A1:IO5`
/// for a terminal of the board). It replaces the automatic wire at those terminals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireLink {
    pub from: String,
    pub to: String,
}

impl WireLink {
    /// The key of the wire the link makes (`from>to`), as `Wire::key` writes it.
    pub fn key(&self) -> String {
        format!("{}>{}", self.from, self.to)
    }
}

/// The device of an end of a link (`K1:A2` → `K1`).
pub fn link_device(end: &str) -> &str {
    end.split_once(':').map_or(end, |(device, _)| device)
}

/// The supply line of a link end that is a bus (`bus:0V` → `0V`, `bus:L1` → `L1`); `None` for a
/// terminal of a device. Whether the line exists is checked where the wires are worked out.
pub fn link_bus(end: &str) -> Option<&str> {
    end.strip_prefix("bus:").filter(|name| !name.is_empty())
}

/// The panel of a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    #[serde(default, skip_serializing_if = "Supply::is_default")]
    pub supply: Supply,
    #[serde(default, rename = "device", skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
    /// Wires drawn by hand between terminals; they are written after the devices.
    #[serde(default, rename = "link", skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<WireLink>,
    /// Numbers given to the wires, kept so that a wire keeps its number when the panel changes:
    /// the key is the ends of the wire (`K1:2>M1:L1`), the value its number. Written by the
    /// documentation generator, not typed by hand.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wire_numbers: BTreeMap<String, u32>,
}

/// Whether `s` is a valid designation of a device: the characters of an identifier and dots
/// (`K1`, `X1.1`); a dot stands between two other characters.
pub fn is_valid_designation(s: &str) -> bool {
    !s.starts_with('.')
        && !s.ends_with('.')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !s.is_empty()
}

impl Panel {
    /// A panel with nothing in it is not stored.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The device with this designation (`K1`).
    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// The first free designation of an auxiliary contact of `of`: `Q2_1`, then `Q2_2`...
    pub fn next_aux_id(&self, of: &str) -> String {
        (1..)
            .map(|n| format!("{of}_{n}"))
            .find(|id| self.device(id).is_none())
            .unwrap_or_default()
    }

    /// The auxiliary contacts of `device`, in the order of the panel.
    pub fn aux_of<'a>(&'a self, device: &'a str) -> impl Iterator<Item = &'a Device> + 'a {
        self.devices
            .iter()
            .filter(move |d| d.kind == DeviceKind::Aux && d.of == device)
    }

    /// Whether `d` is a contact of a relay: such a contact switches a load of its own, as a
    /// one-pole device of the power chain (`specs/panel-relay-contacts`).
    pub fn is_relay_contact(&self, d: &Device) -> bool {
        d.kind == DeviceKind::Aux
            && self
                .device(&d.of)
                .is_some_and(|r| r.kind == DeviceKind::Relay)
    }

    /// The temperature sensors (thermistors) of the motor `device`: sensors whose `of` names it.
    pub fn sensors_of<'a>(&'a self, device: &'a str) -> impl Iterator<Item = &'a Device> + 'a {
        self.devices
            .iter()
            .filter(move |d| d.kind == DeviceKind::Sensor && d.of == device)
    }

    /// The first free designation for a device of `kind`: `K1`, then `K2`...
    pub fn next_id(&self, kind: DeviceKind) -> String {
        (1..)
            .map(|n| format!("{}{n}", kind.letter()))
            .find(|id| self.device(id).is_none())
            .unwrap_or_default()
    }

    /// The designation for a device of `kind` put on the drawing sheet number `sheet` (of the
    /// document): `K3.1`, then `K3.2`... in the order the devices are put there; the number after
    /// the point is one more than the largest one already taken on that sheet.
    pub fn next_sheet_id(&self, kind: DeviceKind, sheet: usize) -> String {
        let prefix = format!("{}{sheet}.", kind.letter());
        let last = self
            .devices
            .iter()
            .filter_map(|d| d.id.strip_prefix(&prefix)?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("{prefix}{}", last + 1)
    }

    /// Gives the device `old` the designation `new`: the devices that it feeds or switches
    /// (`load`) and the stored numbers of its wires follow it. The designation must be free and a
    /// valid identifier.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        if !is_valid_designation(new) {
            return Err(format!(
                "«{new}»: недопустимое обозначение (буквы, цифры, «_», «-» и «.», без пробелов)"
            ));
        }
        if self.device(new).is_some() {
            return Err(format!("обозначение «{new}» уже занято"));
        }
        let Some(device) = self.devices.iter_mut().find(|d| d.id == old) else {
            return Err(format!("нет устройства «{old}»"));
        };
        device.id = new.to_owned();
        for d in &mut self.devices {
            if d.load == old {
                d.load = new.to_owned();
            }
            // Auxiliary contacts follow the device they belong to and the coil they stand with.
            if d.of == old {
                d.of = new.to_owned();
            }
            if d.series == old {
                d.series = new.to_owned();
            }
            if d.from == old {
                d.from = new.to_owned();
            }
        }
        for link in &mut self.links {
            for end in [&mut link.from, &mut link.to] {
                if let Some((device, terminal)) = end.split_once(':') {
                    if device == old {
                        *end = format!("{new}:{terminal}");
                    }
                }
            }
        }
        // The keys are `K1:2>M1:L1`: the device is the part before the colon of each end.
        let numbers = std::mem::take(&mut self.wire_numbers);
        for (key, number) in numbers {
            let renamed = key
                .split('>')
                .map(|end| match end.split_once(':') {
                    Some((device, pin)) if device == old => format!("{new}:{pin}"),
                    _ => end.to_owned(),
                })
                .collect::<Vec<_>>()
                .join(">");
            self.wire_numbers.insert(renamed, number);
        }
        Ok(())
    }

    /// Renames several devices at once (`old`, `new`), also when the new names are the old names
    /// of others (swaps). The new names must be valid and different from each other and from the
    /// devices left alone; nothing changes if one is wrong.
    pub fn rename_all(&mut self, names: &[(String, String)]) -> Result<(), String> {
        let names: Vec<&(String, String)> = names.iter().filter(|(o, n)| o != n).collect();
        let mut taken: Vec<&str> = self
            .devices
            .iter()
            .filter(|d| !names.iter().any(|(o, _)| *o == d.id))
            .map(|d| d.id.as_str())
            .collect();
        for (old, new) in &names {
            if self.device(old).is_none() {
                return Err(format!("нет устройства «{old}»"));
            }
            if !is_valid_designation(new) {
                return Err(format!("«{new}»: недопустимое обозначение"));
            }
            if taken.contains(&new.as_str()) {
                return Err(format!("обозначение «{new}» уже занято"));
            }
            taken.push(new);
        }
        let mut done = self.clone();
        for (i, (old, _)) in names.iter().enumerate() {
            done.rename(old, &format!("tmp_rename_{i}"))?;
        }
        for (i, (_, new)) in names.iter().enumerate() {
            done.rename(&format!("tmp_rename_{i}"), new)?;
        }
        *self = done;
        Ok(())
    }

    /// Removes the device; what it fed or switched is left without a feeder, and the stored
    /// numbers of its wires go with it.
    pub fn remove(&mut self, id: &str) {
        // The auxiliary contacts of the device go with it.
        let aux: Vec<String> = self.aux_of(id).map(|d| d.id.clone()).collect();
        self.devices.retain(|d| d.id != id && !aux.contains(&d.id));
        for d in &mut self.devices {
            if d.load == id {
                d.load.clear();
            }
            if d.series == id {
                d.series.clear();
            }
            if d.from == id {
                d.from.clear();
            }
            // A sensor of a removed motor stays in the panel as a plain sensor.
            if d.of == id && d.kind == DeviceKind::Sensor {
                d.of.clear();
            }
        }
        self.links
            .retain(|l| link_device(&l.from) != id && link_device(&l.to) != id);
        self.wire_numbers.retain(|key, _| {
            !key.split('>')
                .any(|end| end.split_once(':').is_some_and(|(device, _)| device == id))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
supply = "ac400"

[[device]]
id = "Q1"
kind = "breaker"
rating = "C16, 3P"

[[device]]
id = "K1"
kind = "contactor"
bind = "pump.out"
load = "M1"

[[device]]
id = "M1"
kind = "motor"
rating = "0,75 кВт"

[wire_numbers]
"K1:2>M1:L1" = 7
"#;

    #[test]
    fn reads_and_writes_back() {
        let panel: Panel = toml::from_str(SAMPLE).unwrap();
        assert_eq!(panel.supply, Supply::Ac400);
        assert_eq!(panel.devices.len(), 3);
        assert_eq!(panel.device("K1").unwrap().load, "M1");
        assert_eq!(panel.device("K1").unwrap().designation(), "-K1");
        assert_eq!(panel.wire_numbers["K1:2>M1:L1"], 7);
        let text = toml::to_string_pretty(&panel).unwrap();
        assert_eq!(toml::from_str::<Panel>(&text).unwrap(), panel);
    }

    #[test]
    fn empty_panel_is_not_stored() {
        assert!(Panel::default().is_empty());
        let panel: Panel = toml::from_str("").unwrap();
        assert!(panel.is_empty());
        let panel = Panel {
            supply: Supply::Dc24,
            ..Panel::default()
        };
        assert!(!panel.is_empty());
    }

    #[test]
    fn every_kind_has_a_code_that_reads_back() {
        for kind in DeviceKind::ALL {
            let text = format!("id = \"X1\"\nkind = \"{}\"", kind.code());
            let device: Device = toml::from_str(&text).unwrap();
            assert_eq!(device.kind, kind);
            assert!(kind.letter().is_ascii_uppercase());
        }
    }

    #[test]
    fn unknown_kind_is_an_error() {
        let text = "[[device]]\nid = \"X1\"\nkind = \"toaster\"";
        assert!(toml::from_str::<Panel>(text).is_err());
    }

    #[test]
    fn a_device_put_on_a_sheet_is_named_by_the_sheet_in_the_order_it_is_put() {
        assert_eq!(Panel::default().next_sheet_id(DeviceKind::Relay, 3), "K3.1");
        let panel: Panel = toml::from_str(
            r#"
[[device]]
id = "K3.1"
kind = "relay"
[[device]]
id = "K3.4"
kind = "relay"
[[device]]
id = "K5.9"
kind = "relay"
[[device]]
id = "Q3.7"
kind = "breaker"
"#,
        )
        .unwrap();
        assert_eq!(panel.next_sheet_id(DeviceKind::Contactor, 3), "K3.5");
        assert_eq!(panel.next_sheet_id(DeviceKind::Relay, 4), "K4.1");
    }

    #[test]
    fn next_id_is_the_first_free_one() {
        let mut panel = Panel::default();
        assert_eq!(panel.next_id(DeviceKind::Contactor), "K1");
        for id in ["K1", "K2", "K4"] {
            panel.devices.push(Device {
                id: id.into(),
                kind: DeviceKind::Relay,
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
            });
        }
        assert_eq!(panel.next_id(DeviceKind::Contactor), "K3");
        assert_eq!(panel.next_id(DeviceKind::Breaker), "Q1");
    }

    fn pair() -> Panel {
        toml::from_str(
            r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
[wire_numbers]
"Q1:2>K1:1" = 3
"K1:2>M1:L" = 4
"X1:L>Q1:1" = 1
"#,
        )
        .unwrap()
    }

    #[test]
    fn rename_follows_loads_and_wire_numbers() {
        let mut panel = pair();
        panel.rename("K1", "K7").unwrap();
        assert!(panel.device("K1").is_none() && panel.device("K7").is_some());
        assert_eq!(panel.device("Q1").unwrap().load, "K7");
        assert_eq!(panel.wire_numbers["Q1:2>K7:1"], 3);
        assert_eq!(panel.wire_numbers["K7:2>M1:L"], 4);
        assert_eq!(panel.wire_numbers["X1:L>Q1:1"], 1);
        assert_eq!(panel.wire_numbers.len(), 3);
        // The other end of a key is renamed as well.
        panel.rename("M1", "M2").unwrap();
        assert_eq!(panel.wire_numbers["K7:2>M2:L"], 4);
        assert_eq!(panel.device("K7").unwrap().load, "M2");
    }

    #[test]
    fn branches_follow_renames_and_removals() {
        let mut panel: Panel = toml::from_str(
            r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
[[device]]
id = "K2"
kind = "contactor"
"#,
        )
        .unwrap();
        // Q1 already feeds K1: K2 becomes a second device on it.
        panel.branch_power("K2", Some("Q1")).unwrap();
        assert!(panel.branch_power("Q1", Some("K2")).is_err());
        assert_eq!(panel.device("K2").unwrap().from, "Q1");
        assert_eq!(panel.device("Q1").unwrap().load, "K1");
        panel.rename("Q1", "Q7").unwrap();
        assert_eq!(panel.device("K2").unwrap().from, "Q7");
        // Fed the usual way, the device lets its branch go.
        panel.reconnect_power("K2", None).unwrap();
        assert!(panel.device("K2").unwrap().from.is_empty());
        panel.branch_power("K2", Some("Q7")).unwrap();
        panel.remove("Q7");
        assert!(panel.device("K2").unwrap().from.is_empty());
    }

    #[test]
    fn rename_all_swaps_and_refuses_clashes() {
        let mut panel = pair();
        panel
            .rename_all(&[("K1".into(), "K2".into()), ("Q1".into(), "K1".into())])
            .unwrap();
        assert_eq!(panel.device("K1").unwrap().kind, DeviceKind::Breaker);
        assert_eq!(panel.device("K2").unwrap().kind, DeviceKind::Contactor);
        let before = panel.clone();
        assert!(panel.rename_all(&[("K2".into(), "M1".into())]).is_err());
        assert!(panel
            .rename_all(&[("K2".into(), "Z1".into()), ("K1".into(), "Z1".into())])
            .is_err());
        assert_eq!(panel, before);
    }

    #[test]
    fn rename_refuses_a_taken_or_bad_designation() {
        let mut panel = pair();
        assert!(panel.rename("K1", "Q1").is_err());
        assert!(panel.rename("K1", "K 1").is_err());
        assert!(panel.rename("K1", "").is_err());
        assert!(panel.rename("Z9", "Z8").is_err());
        assert!(panel.rename("K1", "K1").is_ok());
        assert_eq!(panel, pair());
    }

    #[test]
    fn remove_drops_the_device_its_links_and_its_wire_numbers() {
        let mut panel = pair();
        panel.remove("K1");
        assert!(panel.device("K1").is_none());
        assert_eq!(panel.device("Q1").unwrap().load, "");
        assert_eq!(panel.wire_numbers.len(), 1);
        assert!(panel.wire_numbers.contains_key("X1:L>Q1:1"));
    }

    #[test]
    fn auxiliary_contacts_round_trip_and_old_files_read_unchanged() {
        let text = r#"
[[device]]
id = "Q2"
kind = "breaker"

[[device]]
id = "Q2_1"
kind = "aux"
of = "Q2"
form = "nc"

[[device]]
id = "K2_1"
kind = "aux"
of = "K2"
series = "K1"
"#;
        let panel: Panel = toml::from_str(text).unwrap();
        let q = panel.device("Q2_1").unwrap();
        assert_eq!(
            (q.kind, q.of.as_str(), q.form),
            (DeviceKind::Aux, "Q2", ContactForm::Nc)
        );
        let k = panel.device("K2_1").unwrap();
        assert_eq!((k.form, k.series.as_str()), (ContactForm::No, "K1"));
        assert_eq!(panel.device("Q2").unwrap().of, "");
        assert_eq!(
            toml::from_str::<Panel>(&toml::to_string(&panel).unwrap()).unwrap(),
            panel
        );
        // The defaults are not written.
        let out = toml::to_string(&panel).unwrap();
        assert_eq!(out.matches("form").count(), 1, "{out}");
        assert_eq!(panel.aux_of("Q2").count(), 1);
        assert_eq!(panel.next_aux_id("Q2"), "Q2_2");
        assert_eq!(panel.next_aux_id("Q9"), "Q9_1");
    }

    #[test]
    fn changeover_contact_round_trips() {
        let text = r#"
[[device]]
id = "Q3_1"
kind = "aux"
of = "Q3"
form = "changeover"
"#;
        let panel: Panel = toml::from_str(text).unwrap();
        let q = panel.device("Q3_1").unwrap();
        assert_eq!(q.form, ContactForm::Changeover);
        let out = toml::to_string(&panel).unwrap();
        assert!(out.contains("form = \"changeover\""), "{out}");
        assert_eq!(toml::from_str::<Panel>(&out).unwrap(), panel);
    }
}
