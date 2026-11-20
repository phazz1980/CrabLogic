//! Electrical panel of a project (`panel.toml`): the devices of the cabinet and what they are
//! bound to (`specs/electrical-schematics`). The panel does not change the schema or the
//! firmware: it only feeds the electrical documentation.

use serde::{Deserialize, Serialize};

/// Entry of the project archive with the panel; written only when the panel is not empty.
pub const PANEL_FILE: &str = "panel.toml";

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
"#;

    #[test]
    fn reads_and_writes_back() {
        let panel: Panel = toml::from_str(SAMPLE).unwrap();
        assert_eq!(panel.supply, Supply::Ac400);
        assert_eq!(panel.devices.len(), 3);
        assert_eq!(panel.device("K1").unwrap().load, "M1");
        assert_eq!(panel.device("K1").unwrap().designation(), "-K1");
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
}
