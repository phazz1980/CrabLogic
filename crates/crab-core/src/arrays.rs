//! Arrays and structures: named sets of values declared in the project file next to the variables.
//!
//! An array is `[[array]]` (fixed length, one element type, a table of initial values), a structure is
//! `[[struct]]` with `[[struct.field]]` (named scalar fields). They are not a signal type: the sheets
//! reach them by name through blocks, and only scalars (an index, a value, a field) go over the wires.
//! The length is fixed when the array is made and never grows.

use serde::{Deserialize, Serialize};

use crate::{Logic, ParamValue, SignalType};

/// Longest array on the chips with the larger RAM (ESP32, ESP32-S3).
pub const MAX_LEN: u32 = 256;
/// Longest array on the ESP32-C3.
pub const MAX_LEN_C3: u32 = 64;

/// Type of an array element or of a structure field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElemType {
    Bool,
    Int,
    Float,
    /// Unsigned 8-bit integer (0..=255): the cheap type for buffers.
    Byte,
    String,
}

impl ElemType {
    pub const ALL: [ElemType; 5] = [
        ElemType::Bool,
        ElemType::Int,
        ElemType::Float,
        ElemType::Byte,
        ElemType::String,
    ];

    /// Name as written in the project file.
    pub fn name(self) -> &'static str {
        match self {
            ElemType::Bool => "bool",
            ElemType::Int => "int",
            ElemType::Float => "float",
            ElemType::Byte => "byte",
            ElemType::String => "string",
        }
    }

    /// Signal type of an element on the wires of the sheets.
    pub fn signal(self) -> SignalType {
        match self {
            ElemType::Bool => SignalType::Bool,
            ElemType::Float => SignalType::Float,
            ElemType::String => SignalType::String,
            ElemType::Int | ElemType::Byte => SignalType::Int,
        }
    }

    /// Whether `value` fits as an element of this type (a `byte` is 0..=255).
    pub fn accepts(self, value: &ParamValue) -> bool {
        match (self, value) {
            (ElemType::Byte, ParamValue::Int(v)) => (0..=255).contains(v),
            (ElemType::Byte, _) => false,
            _ => self.signal().accepts(value),
        }
    }
}

/// An array: `len` elements of one type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayVar {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ElemType,
    pub len: u32,
    /// Initial values by element; fewer than `len` leaves the rest at zero (false, an empty string).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ParamValue>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A structure: named scalar fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructVar {
    pub name: String,
    #[serde(rename = "field", default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<StructField>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ElemType,
    /// Value after power-on; zero when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<ParamValue>,
}

/// Longest array of a chip family (`esp32`, `esp32c3`, `esp32s3`): RAM left to the module after
/// the network stack is smallest on the C3.
pub fn max_len(chip: &str) -> u32 {
    if chip == "esp32c3" {
        MAX_LEN_C3
    } else {
        MAX_LEN
    }
}

impl ArrayVar {
    /// Bytes the array takes in the RAM of a module: 4 per `bool`, `int` and `float`, 1 per `byte`;
    /// text is counted by its longest initial value plus the length byte.
    pub fn bytes(&self) -> u32 {
        let each = match self.ty {
            ElemType::Byte => 1,
            ElemType::String => {
                let longest = self.items.iter().fold(0, |m, v| match v {
                    ParamValue::Text(s) => m.max(s.len()),
                    _ => m,
                });
                u32::try_from(longest).unwrap_or(u32::MAX).saturating_add(1)
            }
            _ => 4,
        };
        self.len.saturating_mul(each)
    }
}

impl Logic {
    pub fn array(&self, name: &str) -> Option<&ArrayVar> {
        self.arrays.iter().find(|a| a.name == name)
    }

    pub fn struct_var(&self, name: &str) -> Option<&StructVar> {
        self.structs.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Logic {
        Logic {
            arrays: vec![
                ArrayVar {
                    name: "окно".into(),
                    ty: ElemType::Float,
                    len: 8,
                    items: vec![ParamValue::Float(1.5), ParamValue::Int(2)],
                    description: "скользящее среднее".into(),
                },
                ArrayVar {
                    name: "buf".into(),
                    ty: ElemType::Byte,
                    len: 16,
                    items: Vec::new(),
                    description: String::new(),
                },
            ],
            structs: vec![StructVar {
                name: "рецепт".into(),
                fields: vec![
                    StructField {
                        name: "temp".into(),
                        ty: ElemType::Float,
                        init: Some(ParamValue::Float(60.0)),
                    },
                    StructField {
                        name: "fan".into(),
                        ty: ElemType::Bool,
                        init: None,
                    },
                ],
                description: String::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn arrays_and_structs_round_trip_through_toml() {
        let logic = sample();
        let text = toml::to_string(&logic).unwrap();
        assert!(text.contains("[[array]]"), "{text}");
        assert!(text.contains("[[struct]]"), "{text}");
        assert!(text.contains("[[struct.field]]"), "{text}");
        let back: Logic = toml::from_str(&text).unwrap();
        assert_eq!(back, logic);
    }

    #[test]
    fn a_project_without_arrays_writes_none() {
        let text = toml::to_string(&Logic::default()).unwrap();
        assert!(
            !text.contains("array") && !text.contains("struct"),
            "{text}"
        );
        let back: Logic = toml::from_str("").unwrap();
        assert!(back.arrays.is_empty() && back.structs.is_empty());
    }

    #[test]
    fn element_types_have_signal_and_range() {
        assert_eq!(ElemType::Byte.signal(), SignalType::Int);
        assert!(ElemType::Byte.accepts(&ParamValue::Int(255)));
        assert!(!ElemType::Byte.accepts(&ParamValue::Int(256)));
        assert!(!ElemType::Byte.accepts(&ParamValue::Int(-1)));
        assert!(ElemType::Float.accepts(&ParamValue::Int(3)));
        assert!(!ElemType::Bool.accepts(&ParamValue::Int(1)));
        assert!(ElemType::String.accepts(&ParamValue::Text("a".into())));
        for ty in ElemType::ALL {
            let text = format!("type = \"{}\"", ty.name());
            let parsed: toml::Table = toml::from_str(&text).unwrap();
            let back: ElemType = parsed["type"].clone().try_into().unwrap();
            assert_eq!(back, ty);
        }
    }

    #[test]
    fn length_limit_depends_on_the_chip() {
        assert_eq!(max_len("esp32"), 256);
        assert_eq!(max_len("esp32s3"), 256);
        assert_eq!(max_len("esp32c3"), 64);
    }

    #[test]
    fn lookup_and_ram_size() {
        let logic = sample();
        assert_eq!(logic.array("окно").map(ArrayVar::bytes), Some(32));
        assert_eq!(logic.array("buf").map(ArrayVar::bytes), Some(16));
        assert!(logic.array("нет").is_none());
        assert_eq!(logic.struct_var("рецепт").map(|s| s.fields.len()), Some(2));
    }
}
