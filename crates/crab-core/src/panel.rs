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
            Self::Breaker | Self::Rcd => 'Q',
            Self::Psu => 'G',
            Self::Contactor | Self::Relay => 'K',
            Self::Motor => 'M',
            Self::Load => 'E',
            Self::Sensor => 'B',
            Self::Button => 'S',
        }
    }

    /// Name in the file, e.g. `contactor`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::Breaker => "breaker",
            Self::Rcd => "rcd",
            Self::Psu => "psu",
            Self::Contactor => "contactor",
            Self::Relay => "relay",
            Self::Motor => "motor",
            Self::Load => "load",
            Self::Terminal => "terminal",
            Self::Sensor => "sensor",
            Self::Button => "button",
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
}

impl Device {
    /// Designation as drawn: `-K1`.
    pub fn designation(&self) -> String {
        format!("-{}", self.id)
    }
}

/// The panel of a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    #[serde(default, skip_serializing_if = "Supply::is_default")]
    pub supply: Supply,
    #[serde(default, rename = "device", skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
    /// Numbers given to the wires, kept so that a wire keeps its number when the panel changes:
    /// the key is the ends of the wire (`K1:2>M1:L1`), the value its number. Written by the
    /// documentation generator, not typed by hand.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wire_numbers: BTreeMap<String, u32>,
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

    /// The first free designation for a device of `kind`: `K1`, then `K2`...
    pub fn next_id(&self, kind: DeviceKind) -> String {
        (1..)
            .map(|n| format!("{}{n}", kind.letter()))
            .find(|id| self.device(id).is_none())
            .unwrap_or_default()
    }

    /// Gives the device `old` the designation `new`: the devices that it feeds or switches
    /// (`load`) and the stored numbers of its wires follow it. The designation must be free and a
    /// valid identifier.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        if !crate::ident::is_valid_ident(new) {
            return Err(format!(
                "«{new}»: недопустимое обозначение (буквы, цифры, «_» и «-», без пробелов)"
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

    /// Removes the device; what it fed or switched is left without a feeder, and the stored
    /// numbers of its wires go with it.
    pub fn remove(&mut self, id: &str) {
        self.devices.retain(|d| d.id != id);
        for d in &mut self.devices {
            if d.load == id {
                d.load.clear();
            }
        }
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
}
