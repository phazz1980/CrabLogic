//! Effective settings of the LAN8720 PHY of the `ethernet` block (chip = "lan8720"):
//! the preset of a known board or the user's own pins. One place for the schema check,
//! the C++ generator and the WASM generator.

use std::collections::BTreeMap;

use crate::value::ParamValue;

/// RMII data pins fixed by the ESP32 EMAC: TXD0, TX_EN, TXD1, RXD0, RXD1, CRS_DV.
pub const RMII_DATA_PINS: [i64; 6] = [19, 21, 22, 25, 26, 27];

/// Where the 50 MHz RMII clock comes from; values match `FLPROG_RMII_CLOCK_*`.
pub const CLOCK_GPIO0_IN: i64 = 0;
pub const CLOCK_GPIO0_OUT: i64 = 1;
pub const CLOCK_GPIO16_OUT: i64 = 2;
pub const CLOCK_GPIO17_OUT: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lan8720 {
    pub phy_addr: i64,
    pub mdc: i64,
    pub mdio: i64,
    /// Pin powering the PHY (or its oscillator); -1 when there is none.
    pub power: i64,
    /// One of `CLOCK_*`.
    pub clock: i64,
}

impl Lan8720 {
    /// Settings of a preset; `custom` (and an unknown preset) reads the own parameters
    /// through `int(name, default)`.
    pub fn of(preset: &str, int: impl Fn(&str, i64) -> i64) -> Self {
        match preset {
            "module" => Self::preset(1, -1, CLOCK_GPIO0_IN),
            "wt32_eth01" => Self::preset(1, 16, CLOCK_GPIO0_IN),
            "olimex_poe" => Self::preset(0, 12, CLOCK_GPIO17_OUT),
            _ => Self {
                phy_addr: int("phy_addr", 1),
                mdc: int("mdc", 23),
                mdio: int("mdio", 18),
                power: int("power_pin", -1),
                clock: int("clock", CLOCK_GPIO0_IN),
            },
        }
    }

    fn preset(phy_addr: i64, power: i64, clock: i64) -> Self {
        Self {
            phy_addr,
            mdc: 23,
            mdio: 18,
            power,
            clock,
        }
    }

    /// Settings of a block instance, `None` when the block uses another chip.
    /// Missing parameters take the block's defaults.
    pub fn from_params(params: &BTreeMap<String, ParamValue>) -> Option<Self> {
        let text = |n: &str, d: &str| match params.get(n) {
            Some(ParamValue::Text(t)) => t.clone(),
            _ => d.to_owned(),
        };
        if text("chip", "w5500") != "lan8720" {
            return None;
        }
        let int = |n: &str, d: i64| match params.get(n) {
            Some(ParamValue::Int(v)) => *v,
            _ => d,
        };
        Some(Self::of(&text("preset", "module"), int))
    }

    /// GPIO the RMII clock enters or leaves by.
    pub fn clock_pin(&self) -> i64 {
        match self.clock {
            CLOCK_GPIO16_OUT => 16,
            CLOCK_GPIO17_OUT => 17,
            _ => 0,
        }
    }

    /// All pins the PHY takes.
    pub fn pins(&self) -> Vec<i64> {
        let mut pins = RMII_DATA_PINS.to_vec();
        pins.extend([self.mdc, self.mdio, self.clock_pin()]);
        if self.power >= 0 {
            pins.push(self.power);
        }
        pins.sort_unstable();
        pins
    }

    /// The oscillator drives GPIO0 from power-on with no pin to hold it off: GPIO0 is a
    /// strapping pin, so the ESP32 may start in download mode.
    pub fn clock_risk(&self) -> bool {
        self.clock == CLOCK_GPIO0_IN && self.power < 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_own_pins() {
        let none = |_: &str, d: i64| d;
        let m = Lan8720::of("module", none);
        assert_eq!((m.phy_addr, m.power, m.clock_pin()), (1, -1, 0));
        assert!(m.clock_risk());
        let w = Lan8720::of("wt32_eth01", none);
        assert_eq!((w.power, w.clock_pin()), (16, 0));
        assert!(!w.clock_risk());
        let o = Lan8720::of("olimex_poe", none);
        assert_eq!((o.phy_addr, o.power, o.clock_pin()), (0, 12, 17));
        let c = Lan8720::of("custom", |n, d| match n {
            "mdc" => 4,
            "clock" => CLOCK_GPIO16_OUT,
            _ => d,
        });
        assert_eq!((c.mdc, c.clock_pin()), (4, 16));
        assert_eq!(c.pins(), vec![4, 16, 18, 19, 21, 22, 25, 26, 27]);
    }

    #[test]
    fn only_for_lan8720() {
        let mut p = BTreeMap::new();
        assert_eq!(Lan8720::from_params(&p), None);
        p.insert("chip".into(), ParamValue::Text("lan8720".into()));
        assert_eq!(
            Lan8720::from_params(&p),
            Some(Lan8720::of("module", |_, d| d))
        );
    }
}
