//! Pure parts of the Ethernet (W5500 or LAN8720) setup: pin checks per chip and the fixed-address texts.
//! Kept out of the board code so that they run in desktop tests.

use alloc::format;
use alloc::string::String;
use core::net::Ipv4Addr;
use core::str::FromStr;

/// The chips the master can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Esp32c3,
    Esp32,
}

/// The four wires of the W5500 module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pins {
    pub sck: i32,
    pub miso: i32,
    pub mosi: i32,
    pub cs: i32,
}

/// Why a pin cannot be used on the chip (the text goes to the journal).
pub fn pin_problem(chip: Chip, pin: i32, input_only_ok: bool) -> Option<&'static str> {
    match chip {
        Chip::Esp32c3 => match pin {
            0..=10 | 20 | 21 => None,
            11..=17 => Some("SPI flash"),
            18 | 19 => Some("USB"),
            _ => Some("no such pin"),
        },
        Chip::Esp32 => match pin {
            0..=5 | 12..=33 => None,
            34..=39 => (!input_only_ok).then_some("input only"),
            6..=11 => Some("SPI flash"),
            _ => Some("no such pin"),
        },
    }
}

/// Checks the pins before the hardware is touched. `Err` carries the journal text.
pub fn check_pins(chip: Chip, p: Pins) -> Result<(), String> {
    for (name, pin, input_ok) in [
        ("sck", p.sck, false),
        ("miso", p.miso, true),
        ("mosi", p.mosi, false),
        ("cs", p.cs, false),
    ] {
        if let Some(why) = pin_problem(chip, pin, input_ok) {
            return Err(format!(
                "pin {pin} ({name}) cannot be used on this chip ({why})"
            ));
        }
    }
    let all = [p.sck, p.miso, p.mosi, p.cs];
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            if all[i] == all[j] {
                return Err(format!("pin {} is used twice", all[i]));
            }
        }
    }
    Ok(())
}

/// RMII data pins fixed by the ESP32 EMAC: TXD0, TX_EN, TXD1, RXD0, RXD1, CRS_DV.
pub const RMII_DATA_PINS: [i32; 6] = [19, 21, 22, 25, 26, 27];

/// LAN8720 on the EMAC; codes 8..=12 of `net` op 6. `clock` is `FLPROG_RMII_CLOCK_*`:
/// 0 in on GPIO0, 1 out on GPIO0, 2 out on GPIO16, 3 out (inverted) on GPIO17.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rmii {
    pub phy_addr: i32,
    pub mdc: i32,
    pub mdio: i32,
    /// Pin powering the PHY or its oscillator; -1 none.
    pub power: i32,
    pub clock: i32,
}

impl Rmii {
    /// Defaults of the block: an ESP32 with a separate LAN8720 module.
    pub const MODULE: Rmii = Rmii {
        phy_addr: 1,
        mdc: 23,
        mdio: 18,
        power: -1,
        clock: 0,
    };

    pub fn clock_pin(&self) -> i32 {
        match self.clock {
            2 => 16,
            3 => 17,
            _ => 0,
        }
    }
}

/// Checks the LAN8720 settings before the hardware is touched. `Err` carries the journal text.
pub fn check_rmii(chip: Chip, r: Rmii) -> Result<(), String> {
    if chip != Chip::Esp32 {
        return Err("LAN8720 needs the classic ESP32 (EMAC); this chip has none".into());
    }
    if !(0..=31).contains(&r.phy_addr) {
        return Err(format!("PHY address {} is out of 0..31", r.phy_addr));
    }
    if !(0..=3).contains(&r.clock) {
        return Err(format!("clock mode {} is unknown", r.clock));
    }
    let mut own = alloc::vec![("mdc", r.mdc), ("mdio", r.mdio), ("clock", r.clock_pin())];
    if r.power >= 0 {
        own.push(("power", r.power));
    }
    for (name, pin) in &own {
        if let Some(why) = pin_problem(chip, *pin, false) {
            return Err(format!(
                "pin {pin} ({name}) cannot be used on this chip ({why})"
            ));
        }
        if RMII_DATA_PINS.contains(pin) {
            return Err(format!("pin {pin} ({name}) is an RMII data pin"));
        }
    }
    for i in 0..own.len() {
        for j in i + 1..own.len() {
            if own[i].1 == own[j].1 {
                return Err(format!("pin {} is used twice", own[i].1));
            }
        }
    }
    Ok(())
}

/// A fixed address: address, gateway and mask length.
pub type Fixed = (Ipv4Addr, Ipv4Addr, u8);

/// Parses the texts of a fixed address. `Ok(None)` for DHCP; `Err` says what is wrong (the caller
/// falls back to DHCP).
pub fn parse_fixed(
    dhcp: bool,
    ip: &str,
    gateway: &str,
    subnet: &str,
) -> Result<Option<Fixed>, &'static str> {
    if dhcp {
        return Ok(None);
    }
    let parse = |s: &str| Ipv4Addr::from_str(s.trim()).ok();
    let (Some(ip), Some(gw), Some(mask)) = (parse(ip), parse(gateway), parse(subnet)) else {
        return Err("fixed address is incomplete or wrong, using DHCP");
    };
    let bits = u32::from(mask);
    // A mask is ones followed by zeros.
    if bits.leading_ones() + bits.trailing_zeros() != 32 {
        return Err("wrong subnet mask, using DHCP");
    }
    Ok(Some((ip, gw, bits.leading_ones() as u8)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(sck: i32, miso: i32, mosi: i32, cs: i32) -> Pins {
        Pins {
            sck,
            miso,
            mosi,
            cs,
        }
    }

    #[test]
    fn c3_refuses_usb_and_flash_pins() {
        assert!(check_pins(Chip::Esp32c3, pins(18, 4, 5, 6)).is_err());
        assert!(check_pins(Chip::Esp32c3, pins(4, 19, 5, 6)).is_err());
        assert!(check_pins(Chip::Esp32c3, pins(4, 5, 12, 6)).is_err());
        let e = check_pins(Chip::Esp32c3, pins(18, 19, 23, 5)).unwrap_err();
        assert!(e.contains("pin 18") && e.contains("USB"), "{e}");
    }

    #[test]
    fn c3_accepts_free_pins() {
        assert!(check_pins(Chip::Esp32c3, pins(4, 5, 6, 7)).is_ok());
        assert!(check_pins(Chip::Esp32c3, pins(0, 10, 20, 21)).is_ok());
    }

    #[test]
    fn esp32_rules() {
        assert!(check_pins(Chip::Esp32, pins(18, 19, 23, 5)).is_ok());
        assert!(check_pins(Chip::Esp32, pins(6, 19, 23, 5)).is_err());
        assert!(check_pins(Chip::Esp32, pins(18, 36, 23, 5)).is_ok());
        assert!(check_pins(Chip::Esp32, pins(18, 19, 36, 5)).is_err());
        assert!(check_pins(Chip::Esp32, pins(18, 19, 23, 40)).is_err());
    }

    #[test]
    fn duplicate_pins_are_refused() {
        assert!(check_pins(Chip::Esp32c3, pins(4, 4, 6, 7)).is_err());
        assert!(check_pins(Chip::Esp32, pins(18, 19, 23, 18)).is_err());
    }

    #[test]
    fn rmii_rules() {
        assert!(check_rmii(Chip::Esp32, Rmii::MODULE).is_ok());
        let e = check_rmii(Chip::Esp32c3, Rmii::MODULE).unwrap_err();
        assert!(e.contains("EMAC"), "{e}");
        let olimex = Rmii {
            phy_addr: 0,
            power: 12,
            clock: 3,
            ..Rmii::MODULE
        };
        assert!(check_rmii(Chip::Esp32, olimex).is_ok());
        // MDC on an RMII data pin, the power pin on the clock, an input-only pin.
        assert!(check_rmii(
            Chip::Esp32,
            Rmii {
                mdc: 21,
                ..Rmii::MODULE
            }
        )
        .is_err());
        assert!(check_rmii(
            Chip::Esp32,
            Rmii {
                power: 0,
                ..Rmii::MODULE
            }
        )
        .is_err());
        assert!(check_rmii(
            Chip::Esp32,
            Rmii {
                mdio: 36,
                ..Rmii::MODULE
            }
        )
        .is_err());
        assert!(check_rmii(
            Chip::Esp32,
            Rmii {
                clock: 4,
                ..Rmii::MODULE
            }
        )
        .is_err());
        assert!(check_rmii(
            Chip::Esp32,
            Rmii {
                phy_addr: 32,
                ..Rmii::MODULE
            }
        )
        .is_err());
    }

    #[test]
    fn fixed_address_texts() {
        assert_eq!(parse_fixed(true, "", "", ""), Ok(None));
        let got = parse_fixed(false, " 192.168.1.50 ", "192.168.1.1", "255.255.255.0").unwrap();
        assert_eq!(
            got,
            Some((
                Ipv4Addr::new(192, 168, 1, 50),
                Ipv4Addr::new(192, 168, 1, 1),
                24
            ))
        );
        assert!(parse_fixed(false, "192.168.1.50", "", "255.255.255.0").is_err());
        assert!(parse_fixed(false, "1.2.3.4", "1.2.3.1", "255.0.255.0").is_err());
        assert!(parse_fixed(false, "1.2.3.4", "1.2.3.1", "abc").is_err());
    }
}
