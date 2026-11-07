use serde::{Deserialize, Serialize};

/// Value of a block parameter as written in the project file (or JSON for AI).
///
/// The meaning (pin, duration, ...) comes from the parameter kind in `block.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl std::fmt::Display for ParamValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamValue::Bool(b) => write!(f, "{b}"),
            ParamValue::Int(i) => write!(f, "{i}"),
            ParamValue::Float(x) => write!(f, "{x}"),
            ParamValue::Text(s) => f.write_str(s),
        }
    }
}

/// Parses a duration like `500ms`, `5s`, `1.5s`, `2m`, `1h` (also `мс`, `с`, `мин`, `ч`)
/// into whole milliseconds.
///
/// Returns `None` for malformed input, fractional milliseconds or values that do not
/// fit into 32 bits (`millis()` on ESP32 is 32-bit).
pub fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let num: f64 = num.parse().ok()?;
    let factor = match unit.trim() {
        "ms" | "мс" => 1.0,
        "s" | "с" => 1_000.0,
        "m" | "min" | "мин" => 60_000.0,
        "h" | "ч" => 3_600_000.0,
        _ => return None,
    };
    let ms = num * factor;
    let valid = ms.is_finite() && ms >= 0.0 && ms.fract() == 0.0 && ms <= f64::from(u32::MAX);
    valid.then_some(ms as u64)
}

/// Writes whole milliseconds the short way: the largest unit that divides the value
/// exactly (`90000` -> `90s`, `120000` -> `2m`, `1500` -> `1500ms`). The text parses
/// back with [`parse_duration_ms`].
pub fn format_duration_ms(ms: u64) -> String {
    match ms {
        0 => "0ms".to_owned(),
        _ if ms.is_multiple_of(3_600_000) => format!("{}h", ms / 3_600_000),
        _ if ms.is_multiple_of(60_000) => format!("{}m", ms / 60_000),
        _ if ms.is_multiple_of(1_000) => format!("{}s", ms / 1_000),
        _ => format!("{ms}ms"),
    }
}

/// Splits milliseconds into (minutes, seconds, milliseconds) for entry in three fields.
pub fn split_min_sec_ms(ms: u64) -> (u64, u64, u64) {
    (ms / 60_000, ms / 1_000 % 60, ms % 1_000)
}

/// Joins minutes, seconds and milliseconds typed in separate fields into one duration;
/// `None` when it does not fit into 32 bits.
pub fn join_min_sec_ms(min: u64, sec: u64, ms: u64) -> Option<u64> {
    let total = min
        .checked_mul(60_000)?
        .checked_add(sec.checked_mul(1_000)?)?
        .checked_add(ms)?;
    (total <= u64::from(u32::MAX)).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_fields() {
        assert_eq!(format_duration_ms(90_000), "90s");
        assert_eq!(format_duration_ms(120_000), "2m");
        assert_eq!(format_duration_ms(7_200_000), "2h");
        assert_eq!(format_duration_ms(1_500), "1500ms");
        assert_eq!(format_duration_ms(0), "0ms");
        for ms in [0, 1, 999, 1_000, 61_500, 3_600_000, 4_000_000_000] {
            assert_eq!(parse_duration_ms(&format_duration_ms(ms)), Some(ms));
        }
        assert_eq!(split_min_sec_ms(61_500), (1, 1, 500));
        assert_eq!(join_min_sec_ms(1, 1, 500), Some(61_500));
        assert_eq!(join_min_sec_ms(0, 90, 0), Some(90_000));
        assert_eq!(join_min_sec_ms(100_000, 0, 0), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("5s"), Some(5_000));
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("1.5s"), Some(1_500));
        assert_eq!(parse_duration_ms("2m"), Some(120_000));
        assert_eq!(parse_duration_ms("1h"), Some(3_600_000));
        assert_eq!(parse_duration_ms("3 с"), Some(3_000));
        assert_eq!(parse_duration_ms("10мин"), Some(600_000));
        assert_eq!(parse_duration_ms("5"), None);
        assert_eq!(parse_duration_ms("s"), None);
        assert_eq!(parse_duration_ms("0.5ms"), None);
        assert_eq!(parse_duration_ms("5 days"), None);
        assert_eq!(parse_duration_ms("2000h"), None);
    }
}
