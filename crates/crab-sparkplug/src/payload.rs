//! The Sparkplug B `Payload` as protobuf bytes, written by hand (no dependencies).
//!
//! Wire types: 0 varint, 1 fixed64, 2 length-delimited, 5 fixed32. Fields are written in
//! ascending number order, as every protobuf encoder does.

/// Data type codes of `sparkplug_b.proto` (only those the blocks produce).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Int32 = 3,
    Int64 = 4,
    UInt64 = 8,
    Float = 9,
    Boolean = 11,
    String = 12,
}

/// A metric value; it also decides the data type.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int32(i32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Bool(bool),
    Text(String),
}

impl Value {
    pub fn data_type(&self) -> DataType {
        match self {
            Self::Int32(_) => DataType::Int32,
            Self::Int64(_) => DataType::Int64,
            Self::UInt64(_) => DataType::UInt64,
            Self::Float(_) => DataType::Float,
            Self::Bool(_) => DataType::Boolean,
            Self::Text(_) => DataType::String,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: Option<String>,
    pub alias: Option<u64>,
    pub timestamp: Option<u64>,
    pub value: Value,
}

impl Metric {
    /// A metric known by its name only (the standard `bdSeq`, `Node Control/Rebirth`).
    pub fn named(name: &str, value: Value) -> Self {
        Self {
            name: Some(name.to_owned()),
            alias: None,
            timestamp: None,
            value,
        }
    }

    /// A metric of a birth message: name, alias and the current value.
    pub fn birth(name: &str, alias: u64, value: Value) -> Self {
        Self {
            name: Some(name.to_owned()),
            alias: Some(alias),
            timestamp: None,
            value,
        }
    }

    /// A metric of a data message: the alias announced in the birth and the new value.
    pub fn alias(alias: u64, value: Value) -> Self {
        Self {
            name: None,
            alias: Some(alias),
            timestamp: None,
            value,
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        if let Some(name) = &self.name {
            put_bytes(out, 1, name.as_bytes());
        }
        if let Some(alias) = self.alias {
            put_varint_field(out, 2, alias);
        }
        if let Some(ts) = self.timestamp {
            put_varint_field(out, 3, ts);
        }
        put_varint_field(out, 4, self.value.data_type() as u64);
        match &self.value {
            // `int_value` is a uint32: a negative number goes as its 32-bit two's complement.
            Value::Int32(v) => put_varint_field(out, 10, u64::from(*v as u32)),
            Value::Int64(v) => put_varint_field(out, 11, *v as u64),
            Value::UInt64(v) => put_varint_field(out, 11, *v),
            Value::Float(v) => {
                put_key(out, 12, 5);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Value::Bool(v) => put_varint_field(out, 14, u64::from(*v)),
            Value::Text(s) => put_bytes(out, 15, s.as_bytes()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Payload {
    /// Milliseconds since the epoch.
    pub timestamp: Option<u64>,
    pub metrics: Vec<Metric>,
    pub seq: Option<u64>,
}

impl Payload {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(ts) = self.timestamp {
            put_varint_field(&mut out, 1, ts);
        }
        for m in &self.metrics {
            let mut inner = Vec::new();
            m.encode(&mut inner);
            put_bytes(&mut out, 2, &inner);
        }
        if let Some(seq) = self.seq {
            put_varint_field(&mut out, 3, seq);
        }
        out
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_key(out: &mut Vec<u8>, field: u32, wire: u8) {
    put_varint(out, u64::from(field) << 3 | u64::from(wire));
}

fn put_varint_field(out: &mut Vec<u8>, field: u32, v: u64) {
    put_key(out, field, 0);
    put_varint(out, v);
}

fn put_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    put_key(out, field, 2);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(metric: Metric) -> Vec<u8> {
        Payload {
            timestamp: Some(1),
            metrics: vec![metric],
            seq: Some(0),
        }
        .encode()
    }

    #[test]
    fn boolean_metric_matches_hand_computed_bytes() {
        // name "a" | alias 1 | datatype 11 | boolean_value true; timestamp 1 ... seq 0
        assert_eq!(
            one(Metric::birth("a", 1, Value::Bool(true))),
            [
                0x08, 0x01, 0x12, 0x09, 0x0A, 0x01, 0x61, 0x10, 0x01, 0x20, 0x0B, 0x70, 0x01, 0x18,
                0x00
            ]
        );
    }

    #[test]
    fn float_is_fixed32_little_endian() {
        // 1.0f32 = 00 00 80 3F; key (12 << 3) | 5 = 0x65; datatype Float = 9
        assert_eq!(
            one(Metric::alias(2, Value::Float(1.0))),
            [
                0x08, 0x01, 0x12, 0x09, 0x10, 0x02, 0x20, 0x09, 0x65, 0x00, 0x00, 0x80, 0x3F, 0x18,
                0x00
            ]
        );
    }

    #[test]
    fn text_is_length_delimited() {
        // key (15 << 3) | 2 = 0x7A, datatype String = 12
        assert_eq!(
            one(Metric::alias(3, Value::Text("hi".into()))),
            [0x08, 0x01, 0x12, 0x08, 0x10, 0x03, 0x20, 0x0C, 0x7A, 0x02, b'h', b'i', 0x18, 0x00]
        );
    }

    #[test]
    fn long_varint_and_negative_numbers() {
        let mut v = Vec::new();
        put_varint(&mut v, 300);
        assert_eq!(v, [0xAC, 0x02]);
        // a negative int64 takes 10 bytes
        let bytes = one(Metric::alias(1, Value::Int64(-1)));
        assert!(bytes
            .windows(11)
            .any(|w| w == [0x58, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]));
    }
}
