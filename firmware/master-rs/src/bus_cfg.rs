//! Pure rules of `crab.bus` (GPIO, I²C, UART for driver blocks): the claims of a module, argument
//! checks per operation and the I²C pin rule. Kept out of the board code so that they run in
//! desktop tests. The hardware side is `board/bus.rs`.

use crate::eth_cfg::{pin_problem, Chip};
use crab_kernel::abi::bus_op::*;

/// A checked call of a module, ready for the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Claim or lock: already done, the answer is 0.
    Done,
    GpioMode {
        pin: i32,
        mode: i32,
    },
    GpioRead {
        pin: i32,
    },
    GpioWrite {
        pin: i32,
        high: bool,
    },
    I2cOpen {
        sda: i32,
        scl: i32,
        hz: u32,
    },
    I2cWrite {
        addr: i32,
        ptr: i32,
        len: usize,
    },
    I2cRead {
        addr: i32,
        ptr: i32,
        len: usize,
    },
    I2cWriteRead {
        addr: i32,
        wptr: i32,
        wlen: usize,
        rptr: i32,
        rlen: usize,
    },
    UartOpen {
        port: i32,
        tx: i32,
        rx: i32,
        baud: u32,
    },
    UartWrite {
        port: i32,
        ptr: i32,
        len: usize,
    },
    UartRead {
        port: i32,
        ptr: i32,
        cap: usize,
    },
    UartAvailable {
        port: i32,
    },
    Micros,
    DelayUs {
        us: u32,
    },
    SpiOpen {
        sck: i32,
        /// -1: not used.
        mosi: i32,
        /// -1: not used.
        miso: i32,
        hz: u32,
    },
    SpiTransfer {
        /// -1: no chip select.
        cs: i32,
        ptr: i32,
        len: usize,
        mode: u8,
    },
    OwReset {
        pin: i32,
    },
    OwWrite {
        pin: i32,
        ptr: i32,
        len: usize,
    },
    OwRead {
        pin: i32,
        ptr: i32,
        len: usize,
    },
    OwReadBit {
        pin: i32,
    },
    OwWriteBit {
        pin: i32,
        bit: bool,
    },
}

/// Fastest SPI clock a module may ask for, Hz.
pub const SPI_MAX_HZ: u32 = 80_000_000;

/// Whether a pin is one of the input-only pins of the chip (no output, no pulls).
fn input_only(chip: Chip, pin: i32) -> bool {
    chip == Chip::Esp32 && (34..=39).contains(&pin)
}

/// Why the chip does not let a module use the pin (flash, USB, console, no such pin).
fn pin_ok(chip: Chip, pin: i32, as_output: bool) -> bool {
    // GPIO1 and GPIO3 of the classic ESP32 are the console (UART0) that carries the editor link.
    if chip == Chip::Esp32 && matches!(pin, 1 | 3) {
        return false;
    }
    pin_problem(chip, pin, !as_output).is_none()
}

/// UART ports a module may use: UART0 is the console; the C3 has UART1 only.
fn port_ok(chip: Chip, port: i32) -> bool {
    match chip {
        Chip::Esp32c3 => port == 1,
        Chip::Esp32 => matches!(port, 1 | 2),
    }
}

/// What one module claimed. Claims are taken before `LOCK`; afterwards only claimed pins, I²C
/// addresses and UART ports may be used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Claims {
    pins: u64,
    addrs: u128,
    ports: u8,
    locked: bool,
}

impl Claims {
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Takes a claim. `Err` carries the error code of `crab.bus`.
    pub fn claim(&mut self, chip: Chip, kind: i32, value: i32) -> Result<(), i32> {
        if self.locked {
            return Err(ERR_NOT_CLAIMED);
        }
        match kind {
            KIND_PIN => {
                // Input-only pins may be claimed; using them as outputs is refused per call.
                if !pin_ok(chip, value, false) && !pin_ok(chip, value, true) {
                    return Err(ERR_ARGS);
                }
                self.pins |= 1 << value;
            }
            KIND_I2C_ADDR => {
                if !(0x08..=0x77).contains(&value) {
                    return Err(ERR_ARGS);
                }
                self.addrs |= 1 << value;
            }
            KIND_UART => {
                if !port_ok(chip, value) {
                    return Err(ERR_ARGS);
                }
                self.ports |= 1 << value;
            }
            _ => return Err(ERR_ARGS),
        }
        Ok(())
    }

    /// Ends the claims.
    pub fn lock(&mut self) {
        self.locked = true;
    }

    pub fn check_pin(&self, pin: i32) -> bool {
        (0..64).contains(&pin) && self.pins >> pin & 1 != 0
    }

    pub fn check_addr(&self, addr: i32) -> bool {
        (0..128).contains(&addr) && self.addrs >> addr & 1 != 0
    }

    pub fn check_port(&self, port: i32) -> bool {
        (0..8).contains(&port) && self.ports >> port & 1 != 0
    }

    fn pin(&self, chip: Chip, pin: i32, as_output: bool) -> Result<(), i32> {
        if !pin_ok(chip, pin, as_output) || (as_output && input_only(chip, pin)) {
            return Err(ERR_ARGS);
        }
        if self.check_pin(pin) {
            Ok(())
        } else {
            Err(ERR_NOT_CLAIMED)
        }
    }

    fn addr(&self, addr: i32) -> Result<(), i32> {
        if !(0..128).contains(&addr) {
            return Err(ERR_ARGS);
        }
        if self.check_addr(addr) {
            Ok(())
        } else {
            Err(ERR_NOT_CLAIMED)
        }
    }

    fn port(&self, chip: Chip, port: i32) -> Result<(), i32> {
        if !port_ok(chip, port) {
            return Err(ERR_ARGS);
        }
        if self.check_port(port) {
            Ok(())
        } else {
            Err(ERR_NOT_CLAIMED)
        }
    }

    /// Checks one call of the module and, for `CLAIM` and `LOCK`, carries it out. `Err` is the
    /// code `crab.bus` returns.
    pub fn request(
        &mut self,
        chip: Chip,
        op: i32,
        a: i32,
        b: i32,
        c: i32,
        d: i32,
    ) -> Result<Request, i32> {
        match op {
            CLAIM => return self.claim(chip, a, b).map(|()| Request::Done),
            LOCK => {
                self.lock();
                return Ok(Request::Done);
            }
            MICROS => return Ok(Request::Micros),
            DELAY_US => {
                return Ok(Request::DelayUs {
                    us: a.clamp(0, 100) as u32,
                })
            }
            GPIO_MODE | GPIO_READ | GPIO_WRITE | I2C_OPEN | I2C_WRITE | I2C_READ
            | I2C_WRITE_READ | UART_OPEN | UART_WRITE | UART_READ | UART_AVAILABLE | SPI_OPEN
            | SPI_TRANSFER | OW_RESET | OW_WRITE | OW_READ | OW_READ_BIT | OW_WRITE_BIT => {}
            _ => return Err(ERR_ARGS),
        }
        if !self.locked {
            return Err(ERR_NOT_CLAIMED);
        }
        let len = |n: i32, min: i32| {
            if (min..=MAX_LEN).contains(&n) {
                Ok(n as usize)
            } else {
                Err(ERR_ARGS)
            }
        };
        match op {
            GPIO_MODE => {
                if !(0..=4).contains(&b) {
                    return Err(ERR_ARGS);
                }
                // Input-only pins have no output driver and no pulls.
                if input_only(chip, a) && b != 0 {
                    return Err(ERR_ARGS);
                }
                self.pin(chip, a, matches!(b, 1 | 4))?;
                Ok(Request::GpioMode { pin: a, mode: b })
            }
            GPIO_READ => {
                self.pin(chip, a, false)?;
                Ok(Request::GpioRead { pin: a })
            }
            GPIO_WRITE => {
                self.pin(chip, a, true)?;
                Ok(Request::GpioWrite {
                    pin: a,
                    high: b != 0,
                })
            }
            I2C_OPEN => {
                if !(0..=1_000_000).contains(&c) || a == b {
                    return Err(ERR_ARGS);
                }
                self.pin(chip, a, true)?;
                self.pin(chip, b, true)?;
                Ok(Request::I2cOpen {
                    sda: a,
                    scl: b,
                    hz: if c == 0 { 100_000 } else { c as u32 },
                })
            }
            I2C_WRITE => {
                let len = len(c, 0)?;
                self.addr(a)?;
                Ok(Request::I2cWrite {
                    addr: a,
                    ptr: b,
                    len,
                })
            }
            I2C_READ => {
                let len = len(c, 1)?;
                self.addr(a)?;
                Ok(Request::I2cRead {
                    addr: a,
                    ptr: b,
                    len,
                })
            }
            I2C_WRITE_READ => {
                let wlen = len(c & 0xFFFF, 1)?;
                let rlen = len((c >> 16) & 0xFFFF, 1)?;
                self.addr(a)?;
                Ok(Request::I2cWriteRead {
                    addr: a,
                    wptr: b,
                    wlen,
                    rptr: d,
                    rlen,
                })
            }
            UART_OPEN => {
                if !(1..=2_000_000).contains(&d) || a < 0 || b == c {
                    return Err(ERR_ARGS);
                }
                self.port(chip, a)?;
                self.pin(chip, b, true)?;
                self.pin(chip, c, false)?;
                Ok(Request::UartOpen {
                    port: a,
                    tx: b,
                    rx: c,
                    baud: d as u32,
                })
            }
            UART_WRITE => {
                let len = len(c, 0)?;
                self.port(chip, a)?;
                Ok(Request::UartWrite {
                    port: a,
                    ptr: b,
                    len,
                })
            }
            UART_READ => {
                let cap = len(c, 0)?;
                self.port(chip, a)?;
                Ok(Request::UartRead {
                    port: a,
                    ptr: b,
                    cap,
                })
            }
            UART_AVAILABLE => {
                self.port(chip, a)?;
                Ok(Request::UartAvailable { port: a })
            }
            SPI_OPEN => {
                // sck = a, mosi = b, miso = c, hz = d; only mosi and miso may be -1.
                let (sck, mosi, miso) = (a, b, c);
                if d < 0 || sck < 0 || mosi < -1 || miso < -1 {
                    return Err(ERR_ARGS);
                }
                if (mosi == -1 && miso == -1)
                    || sck == mosi
                    || sck == miso
                    || (mosi == miso && mosi >= 0)
                {
                    return Err(ERR_ARGS);
                }
                self.pin(chip, sck, true)?;
                if mosi >= 0 {
                    self.pin(chip, mosi, true)?;
                }
                if miso >= 0 {
                    self.pin(chip, miso, false)?;
                }
                let hz = if d == 0 { 1_000_000 } else { d as u32 };
                Ok(Request::SpiOpen {
                    sck,
                    mosi,
                    miso,
                    hz: hz.min(SPI_MAX_HZ),
                })
            }
            SPI_TRANSFER => {
                let len = len(c, 1)?;
                if !(0..=3).contains(&d) || a < -1 {
                    return Err(ERR_ARGS);
                }
                if a >= 0 {
                    self.pin(chip, a, true)?;
                }
                Ok(Request::SpiTransfer {
                    cs: a,
                    ptr: b,
                    len,
                    mode: d as u8,
                })
            }
            OW_RESET => {
                self.pin(chip, a, true)?;
                Ok(Request::OwReset { pin: a })
            }
            OW_WRITE => {
                let len = len(c, 1)?;
                self.pin(chip, a, true)?;
                Ok(Request::OwWrite {
                    pin: a,
                    ptr: b,
                    len,
                })
            }
            OW_READ => {
                let len = len(c, 1)?;
                self.pin(chip, a, true)?;
                Ok(Request::OwRead {
                    pin: a,
                    ptr: b,
                    len,
                })
            }
            OW_READ_BIT => {
                self.pin(chip, a, true)?;
                Ok(Request::OwReadBit { pin: a })
            }
            _ => {
                self.pin(chip, a, true)?;
                Ok(Request::OwWriteBit {
                    pin: a,
                    bit: b != 0,
                })
            }
        }
    }
}

/// The SPI controller of drivers is one per board: the first open sets the pins, later opens must use
/// the same. `Ok(true)` means the controller must be set up.
pub fn spi_open_rule(
    current: Option<(i32, i32, i32)>,
    sck: i32,
    mosi: i32,
    miso: i32,
) -> Result<bool, i32> {
    match current {
        None => Ok(true),
        Some(p) if p == (sck, mosi, miso) => Ok(false),
        Some(_) => Err(ERR_ARGS),
    }
}

/// The I²C controller is one per board: the first open sets the pins, later opens must use the same.
/// `current` holds the pins of the open controller; `Ok(true)` means the controller must be set up.
pub fn i2c_open_rule(current: Option<(i32, i32)>, sda: i32, scl: i32) -> Result<bool, i32> {
    match current {
        None => Ok(true),
        Some(p) if p == (sda, scl) => Ok(false),
        Some(_) => Err(ERR_ARGS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C3: Chip = Chip::Esp32c3;
    const ESP: Chip = Chip::Esp32;

    fn locked(chip: Chip, pins: &[i32], addrs: &[i32], ports: &[i32]) -> Claims {
        let mut c = Claims::default();
        for &p in pins {
            c.claim(chip, KIND_PIN, p).unwrap();
        }
        for &a in addrs {
            c.claim(chip, KIND_I2C_ADDR, a).unwrap();
        }
        for &p in ports {
            c.claim(chip, KIND_UART, p).unwrap();
        }
        c.lock();
        c
    }

    #[test]
    fn claims_only_before_lock() {
        let mut c = Claims::default();
        assert_eq!(c.claim(C3, KIND_PIN, 4), Ok(()));
        assert_eq!(c.request(C3, LOCK, 0, 0, 0, 0), Ok(Request::Done));
        assert_eq!(c.claim(C3, KIND_PIN, 5), Err(ERR_NOT_CLAIMED));
        assert_eq!(
            c.request(C3, CLAIM, KIND_PIN, 5, 0, 0),
            Err(ERR_NOT_CLAIMED)
        );
        assert!(c.check_pin(4) && !c.check_pin(5));
    }

    #[test]
    fn claim_checks_the_arguments() {
        let mut c = Claims::default();
        assert_eq!(c.claim(C3, KIND_PIN, 12), Err(ERR_ARGS), "flash pin");
        assert_eq!(c.claim(C3, KIND_PIN, 18), Err(ERR_ARGS), "USB pin");
        assert_eq!(c.claim(C3, KIND_PIN, 99), Err(ERR_ARGS));
        assert_eq!(c.claim(C3, KIND_PIN, -1), Err(ERR_ARGS));
        assert_eq!(c.claim(ESP, KIND_PIN, 1), Err(ERR_ARGS), "console TX");
        assert_eq!(c.claim(ESP, KIND_PIN, 3), Err(ERR_ARGS), "console RX");
        assert_eq!(c.claim(ESP, KIND_PIN, 7), Err(ERR_ARGS), "flash pin");
        assert_eq!(
            c.claim(ESP, KIND_PIN, 35),
            Ok(()),
            "input-only is claimable"
        );
        assert_eq!(c.claim(C3, KIND_I2C_ADDR, 0x78), Err(ERR_ARGS));
        assert_eq!(c.claim(C3, KIND_I2C_ADDR, 0x03), Err(ERR_ARGS));
        assert_eq!(c.claim(C3, KIND_I2C_ADDR, 0x76), Ok(()));
        assert_eq!(
            c.claim(C3, KIND_UART, 0),
            Err(ERR_ARGS),
            "UART0 is the console"
        );
        assert_eq!(
            c.claim(C3, KIND_UART, 2),
            Err(ERR_ARGS),
            "C3 has UART1 only"
        );
        assert_eq!(c.claim(ESP, KIND_UART, 2), Ok(()));
        assert_eq!(c.claim(C3, 9, 1), Err(ERR_ARGS), "unknown kind");
    }

    #[test]
    fn io_needs_the_lock() {
        let mut c = Claims::default();
        c.claim(C3, KIND_PIN, 4).unwrap();
        assert_eq!(c.request(C3, GPIO_READ, 4, 0, 0, 0), Err(ERR_NOT_CLAIMED));
        c.lock();
        assert_eq!(
            c.request(C3, GPIO_READ, 4, 0, 0, 0),
            Ok(Request::GpioRead { pin: 4 })
        );
    }

    #[test]
    fn gpio_rules() {
        let mut c = locked(C3, &[4, 5], &[], &[]);
        assert_eq!(
            c.request(C3, GPIO_MODE, 4, 1, 0, 0),
            Ok(Request::GpioMode { pin: 4, mode: 1 })
        );
        assert_eq!(c.request(C3, GPIO_MODE, 4, 5, 0, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, GPIO_MODE, 6, 0, 0, 0), Err(ERR_NOT_CLAIMED));
        assert_eq!(c.request(C3, GPIO_READ, 6, 0, 0, 0), Err(ERR_NOT_CLAIMED));
        assert_eq!(c.request(C3, GPIO_WRITE, 6, 1, 0, 0), Err(ERR_NOT_CLAIMED));
        assert_eq!(
            c.request(C3, GPIO_WRITE, 12, 1, 0, 0),
            Err(ERR_ARGS),
            "flash pin"
        );
        assert_eq!(
            c.request(C3, GPIO_WRITE, 5, 7, 0, 0),
            Ok(Request::GpioWrite { pin: 5, high: true })
        );
    }

    #[test]
    fn input_only_pins_cannot_drive() {
        let mut c = locked(ESP, &[35, 4], &[], &[]);
        assert!(c.request(ESP, GPIO_MODE, 35, 0, 0, 0).is_ok());
        assert_eq!(c.request(ESP, GPIO_MODE, 35, 1, 0, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(ESP, GPIO_MODE, 35, 2, 0, 0),
            Err(ERR_ARGS),
            "no pull-up"
        );
        assert_eq!(c.request(ESP, GPIO_WRITE, 35, 1, 0, 0), Err(ERR_ARGS));
        assert!(c.request(ESP, GPIO_READ, 35, 0, 0, 0).is_ok());
    }

    #[test]
    fn i2c_rules() {
        let mut c = locked(C3, &[4, 5], &[0x76], &[]);
        assert_eq!(
            c.request(C3, I2C_OPEN, 4, 5, 400_000, 0),
            Ok(Request::I2cOpen {
                sda: 4,
                scl: 5,
                hz: 400_000
            })
        );
        assert_eq!(
            c.request(C3, I2C_OPEN, 4, 5, 0, 0),
            Ok(Request::I2cOpen {
                sda: 4,
                scl: 5,
                hz: 100_000
            })
        );
        assert_eq!(c.request(C3, I2C_OPEN, 4, 4, 100_000, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, I2C_OPEN, 4, 6, 100_000, 0),
            Err(ERR_NOT_CLAIMED)
        );
        assert_eq!(c.request(C3, I2C_OPEN, 4, 5, 2_000_000, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, I2C_WRITE, 0x76, 100, 2, 0),
            Ok(Request::I2cWrite {
                addr: 0x76,
                ptr: 100,
                len: 2
            })
        );
        assert_eq!(
            c.request(C3, I2C_WRITE, 0x77, 100, 2, 0),
            Err(ERR_NOT_CLAIMED)
        );
        assert_eq!(
            c.request(C3, I2C_READ, 0x77, 100, 2, 0),
            Err(ERR_NOT_CLAIMED)
        );
        assert_eq!(c.request(C3, I2C_WRITE, 0x76, 100, 257, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, I2C_READ, 0x76, 100, 0, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, I2C_READ, 0x76, 100, -1, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, I2C_WRITE_READ, 0x76, 10, 1 | (6 << 16), 20),
            Ok(Request::I2cWriteRead {
                addr: 0x76,
                wptr: 10,
                wlen: 1,
                rptr: 20,
                rlen: 6
            })
        );
        assert_eq!(
            c.request(C3, I2C_WRITE_READ, 0x76, 10, 1 | (257 << 16), 20),
            Err(ERR_ARGS)
        );
        assert_eq!(
            c.request(C3, I2C_WRITE_READ, 0x50, 10, 1 | (1 << 16), 20),
            Err(ERR_NOT_CLAIMED)
        );
    }

    #[test]
    fn i2c_pins_of_the_controller() {
        assert_eq!(i2c_open_rule(None, 4, 5), Ok(true));
        assert_eq!(i2c_open_rule(Some((4, 5)), 4, 5), Ok(false));
        assert_eq!(i2c_open_rule(Some((4, 5)), 5, 4), Err(ERR_ARGS));
        assert_eq!(i2c_open_rule(Some((4, 5)), 6, 7), Err(ERR_ARGS));
    }

    #[test]
    fn uart_rules() {
        let mut c = locked(C3, &[6, 7], &[], &[1]);
        assert_eq!(
            c.request(C3, UART_OPEN, 1, 6, 7, 9600),
            Ok(Request::UartOpen {
                port: 1,
                tx: 6,
                rx: 7,
                baud: 9600
            })
        );
        assert_eq!(
            c.request(C3, UART_OPEN, 2, 6, 7, 9600),
            Err(ERR_ARGS),
            "no UART2 on C3"
        );
        assert_eq!(
            c.request(C3, UART_OPEN, 0, 6, 7, 9600),
            Err(ERR_ARGS),
            "UART0"
        );
        assert_eq!(
            c.request(C3, UART_OPEN, 1, 6, 8, 9600),
            Err(ERR_NOT_CLAIMED)
        );
        assert_eq!(c.request(C3, UART_OPEN, 1, 6, 6, 9600), Err(ERR_ARGS));
        assert_eq!(c.request(C3, UART_OPEN, 1, 6, 7, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, UART_WRITE, 1, 64, 10, 0),
            Ok(Request::UartWrite {
                port: 1,
                ptr: 64,
                len: 10
            })
        );
        assert_eq!(c.request(C3, UART_WRITE, 1, 64, 300, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, UART_READ, 1, 64, 16, 0),
            Ok(Request::UartRead {
                port: 1,
                ptr: 64,
                cap: 16
            })
        );
        assert_eq!(
            c.request(C3, UART_AVAILABLE, 1, 0, 0, 0),
            Ok(Request::UartAvailable { port: 1 })
        );
        let mut esp = locked(ESP, &[16, 17], &[], &[1]);
        assert_eq!(
            esp.request(ESP, UART_AVAILABLE, 2, 0, 0, 0),
            Err(ERR_NOT_CLAIMED)
        );
        // RX may be an input-only pin, TX may not.
        let mut esp = locked(ESP, &[16, 34], &[], &[2]);
        assert!(esp.request(ESP, UART_OPEN, 2, 16, 34, 115_200).is_ok());
        assert_eq!(
            esp.request(ESP, UART_OPEN, 2, 34, 16, 115_200),
            Err(ERR_ARGS)
        );
    }

    #[test]
    fn spi_rules() {
        let mut c = locked(C3, &[4, 5, 6, 7], &[], &[]);
        assert_eq!(
            c.request(C3, SPI_OPEN, 4, 5, 6, 0),
            Ok(Request::SpiOpen {
                sck: 4,
                mosi: 5,
                miso: 6,
                hz: 1_000_000
            })
        );
        assert_eq!(
            c.request(C3, SPI_OPEN, 4, -1, 6, 500_000_000),
            Ok(Request::SpiOpen {
                sck: 4,
                mosi: -1,
                miso: 6,
                hz: SPI_MAX_HZ
            })
        );
        assert!(c.request(C3, SPI_OPEN, 4, 5, -1, 1000).is_ok());
        assert_eq!(c.request(C3, SPI_OPEN, 4, -1, -1, 1000), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_OPEN, -1, 5, 6, 1000), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_OPEN, 4, 4, 6, 1000), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_OPEN, 4, 5, 5, 1000), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_OPEN, 4, 5, 6, -1), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_OPEN, 4, 5, 8, 1000), Err(ERR_NOT_CLAIMED));
        assert_eq!(c.request(C3, SPI_OPEN, 4, 12, 6, 1000), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, SPI_TRANSFER, 7, 100, 16, 3),
            Ok(Request::SpiTransfer {
                cs: 7,
                ptr: 100,
                len: 16,
                mode: 3
            })
        );
        assert!(c.request(C3, SPI_TRANSFER, -1, 100, 1, 0).is_ok());
        assert_eq!(c.request(C3, SPI_TRANSFER, 7, 100, 16, 4), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_TRANSFER, 7, 100, 16, -1), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_TRANSFER, 7, 100, 0, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, SPI_TRANSFER, 7, 100, 257, 0), Err(ERR_ARGS));
        assert_eq!(
            c.request(C3, SPI_TRANSFER, 9, 100, 4, 0),
            Err(ERR_NOT_CLAIMED)
        );
        let mut esp = locked(ESP, &[18, 23, 19, 34], &[], &[]);
        assert!(esp.request(ESP, SPI_OPEN, 18, 23, 34, 0).is_ok());
        assert_eq!(
            esp.request(ESP, SPI_OPEN, 18, 34, 19, 0),
            Err(ERR_ARGS),
            "MOSI must be an output"
        );
        assert_eq!(
            esp.request(ESP, SPI_TRANSFER, 34, 100, 1, 0),
            Err(ERR_ARGS),
            "CS must be an output"
        );
    }

    #[test]
    fn spi_pins_of_the_controller() {
        assert_eq!(spi_open_rule(None, 4, 5, 6), Ok(true));
        assert_eq!(spi_open_rule(Some((4, 5, 6)), 4, 5, 6), Ok(false));
        assert_eq!(spi_open_rule(Some((4, 5, 6)), 4, -1, 6), Err(ERR_ARGS));
    }

    #[test]
    fn one_wire_rules() {
        let mut c = locked(C3, &[4], &[], &[]);
        assert_eq!(
            c.request(C3, OW_RESET, 4, 0, 0, 0),
            Ok(Request::OwReset { pin: 4 })
        );
        assert_eq!(
            c.request(C3, OW_WRITE, 4, 64, 2, 0),
            Ok(Request::OwWrite {
                pin: 4,
                ptr: 64,
                len: 2
            })
        );
        assert_eq!(
            c.request(C3, OW_READ, 4, 64, 9, 0),
            Ok(Request::OwRead {
                pin: 4,
                ptr: 64,
                len: 9
            })
        );
        assert_eq!(
            c.request(C3, OW_READ_BIT, 4, 0, 0, 0),
            Ok(Request::OwReadBit { pin: 4 })
        );
        assert_eq!(
            c.request(C3, OW_WRITE_BIT, 4, 5, 0, 0),
            Ok(Request::OwWriteBit { pin: 4, bit: true })
        );
        assert_eq!(c.request(C3, OW_WRITE, 4, 64, 0, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, OW_READ, 4, 64, 257, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, OW_RESET, 5, 0, 0, 0), Err(ERR_NOT_CLAIMED));
        assert_eq!(c.request(C3, OW_RESET, 12, 0, 0, 0), Err(ERR_ARGS));
        assert_eq!(c.request(C3, OW_RESET, -1, 0, 0, 0), Err(ERR_ARGS));
        let mut esp = locked(ESP, &[35], &[], &[]);
        assert_eq!(
            esp.request(ESP, OW_RESET, 35, 0, 0, 0),
            Err(ERR_ARGS),
            "input-only pin cannot pull the line low"
        );
        let mut open = Claims::default();
        assert_eq!(
            open.request(C3, OW_RESET, 4, 0, 0, 0),
            Err(ERR_NOT_CLAIMED),
            "before LOCK"
        );
    }

    #[test]
    fn time_and_unknown_ops() {
        let mut c = Claims::default();
        assert_eq!(c.request(C3, MICROS, 0, 0, 0, 0), Ok(Request::Micros));
        assert_eq!(
            c.request(C3, DELAY_US, 5000, 0, 0, 0),
            Ok(Request::DelayUs { us: 100 })
        );
        assert_eq!(
            c.request(C3, DELAY_US, -5, 0, 0, 0),
            Ok(Request::DelayUs { us: 0 })
        );
        assert_eq!(c.request(C3, 99, 0, 0, 0, 0), Err(ERR_ARGS));
    }
}
