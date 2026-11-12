//! The hardware of the ESP32-C3 or the classic ESP32 as modules see it: digital, analog and PWM pins,
//! the clock, driver buses and the network; the imports not done yet answer 0.

use super::analog;
use super::bus::Bus;
use super::{eth, modbus, mqtt, net, pcf, retain, sparkplug, uart, web};
use crab_kernel::abi::id;
use crab_kernel::host::{Devices, Mem, Val};
use crab_kernel::lease::{Fault, Hal, PinUse};
use esp_idf_svc::sys;
use std::sync::atomic::{AtomicU32, Ordering};

/// Pins a module may drive on the C3: 11–17 are the SPI flash, 18–19 the USB console.
#[cfg(target_arch = "riscv32")]
fn usable(pin: u8) -> bool {
    matches!(pin, 0..=10 | 20 | 21)
}

/// Pins a module may read on the C3: the same as it may drive.
#[cfg(target_arch = "riscv32")]
fn readable(pin: u8) -> bool {
    usable(pin)
}

/// Pins a module may drive on the ESP32: 1 and 3 are the console (UART0), 6–11 the SPI flash,
/// 34–39 are inputs only.
#[cfg(not(target_arch = "riscv32"))]
fn usable(pin: u8) -> bool {
    matches!(pin, 0 | 2 | 4 | 5 | 12..=19 | 21..=23 | 25..=27 | 32 | 33)
}

/// Pins a module may read on the ESP32: the outputs and the input-only 34–39.
#[cfg(not(target_arch = "riscv32"))]
fn readable(pin: u8) -> bool {
    usable(pin) || (34..=39).contains(&pin)
}

/// Pins already set up as outputs, bit per pin: the setup is done once, not at every write.
/// `gpio_reset_pin` logs a line to the console (on the ESP32 the same UART0 the protocol uses) and
/// takes milliseconds; at every write it made a blink step 5 ms long.
/// Two words: the ESP32 has no 64-bit atomics.
static OUTPUTS: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

fn write_pin(pin: u8, high: bool) {
    if !usable(pin) {
        return;
    }
    let (word, bit) = (&OUTPUTS[usize::from(pin / 32)], 1u32 << (pin % 32));
    // SAFETY: the pin number was checked; the calls only change this pin.
    unsafe {
        if word.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
            // Some ESP32 pins (12–15 are JTAG) boot with another function: take them as GPIO.
            #[cfg(not(target_arch = "riscv32"))]
            sys::gpio_reset_pin(pin as i32);
            sys::gpio_set_direction(pin as i32, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        }
        sys::gpio_set_level(pin as i32, high as u32);
    }
}

/// Pins already set up as inputs (bit per pin), as for [`OUTPUTS`].
static INPUTS: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];

/// Bit 8 of the pin number of `gpio_read` asks for the internal pull-up.
const PULLUP: i32 = 0x100;

/// Level of `pin`; the first read sets it up as an input (with the pull-up if asked), unless a
/// module drives it: then the level it is driven to is read, as `digitalRead` does.
fn read_pin(pin: u8, pullup: bool) -> i32 {
    if !readable(pin) {
        return 0;
    }
    let (word, bit) = (usize::from(pin / 32), 1u32 << (pin % 32));
    let output = OUTPUTS[word].load(Ordering::Relaxed) & bit != 0;
    // SAFETY: the pin number was checked; the calls only change this pin.
    unsafe {
        if !output && INPUTS[word].fetch_or(bit, Ordering::Relaxed) & bit == 0 {
            #[cfg(not(target_arch = "riscv32"))]
            sys::gpio_reset_pin(pin as i32);
            sys::gpio_set_direction(pin as i32, sys::gpio_mode_t_GPIO_MODE_INPUT);
            let pull = if pullup {
                sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY
            } else {
                sys::gpio_pull_mode_t_GPIO_FLOATING
            };
            sys::gpio_set_pull_mode(pin as i32, pull);
        }
        sys::gpio_get_level(pin as i32)
    }
}

/// The pin is set up again at the next write (a driver block reconfigured it through `crab.bus`).
pub fn forget_output(pin: i32) {
    if (0..64).contains(&pin) {
        OUTPUTS[pin as usize / 32].fetch_and(!(1u32 << (pin % 32)), Ordering::Relaxed);
        INPUTS[pin as usize / 32].fetch_and(!(1u32 << (pin % 32)), Ordering::Relaxed);
    }
}

pub fn millis() -> i32 {
    // SAFETY: no arguments, no state.
    (unsafe { sys::esp_timer_get_time() } / 1000) as i32
}

/// Time since boot that does not wrap (`millis` is the 32-bit clock modules see).
pub fn uptime_ms() -> u64 {
    // SAFETY: no arguments, no state.
    (unsafe { sys::esp_timer_get_time() } / 1000) as u64
}

/// The levels the pins are driven to (GPIO_OUT_REG), bit per pin: what an output is set to,
/// whatever is connected to it. On the ESP32 only pins 0–31 (32 and 33 are in GPIO_OUT1_REG).
pub fn out_register() -> u32 {
    #[cfg(target_arch = "riscv32")]
    const GPIO_OUT_REG: *const u32 = 0x6000_4004 as *const u32;
    #[cfg(not(target_arch = "riscv32"))]
    const GPIO_OUT_REG: *const u32 = 0x3FF4_4004 as *const u32;
    // SAFETY: a readable register of the chip; reading has no side effects.
    unsafe { GPIO_OUT_REG.read_volatile() }
}

/// Puts outputs into their safe state when a lease ends.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardHal;

impl Hal for BoardHal {
    fn safe(&self, pin: u8, use_: PinUse, fault: Fault) {
        let high = fault == Fault::High;
        if use_ == PinUse::Pwm && analog::pwm_safe(pin, high) {
            return;
        }
        write_pin(pin, high);
    }
}

/// The devices of one module: its claims and open buses (`crab.bus`) live and die with it.
#[derive(Debug, Default)]
pub struct BoardDevices {
    bus: Bus,
}

impl Devices for BoardDevices {
    type Hal = BoardHal;

    fn hal(&self) -> BoardHal {
        BoardHal
    }

    fn call(&mut self, import: usize, args: &[Val], mem: Mem<'_>) -> Option<Val> {
        let a = |i: usize| args.get(i).map_or(0, |v| v.i());
        match import {
            id::GPIO_WRITE => {
                write_pin((a(0) & 0xFF) as u8, a(1) != 0);
                None
            }
            id::GPIO_READ => Some(Val::I32(read_pin((a(0) & 0xFF) as u8, a(0) & PULLUP != 0))),
            id::ADC_READ => {
                let pin = (a(0) & 0xFF) as u8;
                Some(Val::I32(if readable(pin) {
                    analog::adc_read(pin)
                } else {
                    0
                }))
            }
            id::PWM_WRITE => {
                let pin = (a(0) & 0xFF) as u8;
                if usable(pin) {
                    // LEDC takes the pin over: a later `gpio_write` sets it up again.
                    forget_output(i32::from(pin));
                    analog::pwm_write(pin, a(1));
                }
                None
            }
            id::MILLIS => Some(Val::I32(millis())),
            id::BUS => {
                let mut mem = mem;
                Some(Val::I32(self.bus.call(
                    a(0),
                    [a(1), a(2), a(3), a(4)],
                    &mut mem,
                )))
            }
            id::PCF_READ => Some(Val::I32(pcf::read(a(0), a(1), a(2)))),
            id::PCF_WRITE => Some(Val::I32(pcf::write(a(0), a(1)))),
            id::UART => {
                let mut mem = mem;
                Some(Val::I32(uart::call(a(0), a(1), a(2), a(3), &mut mem)))
            }
            id::RETAIN => Some(Val::I32(retain::retain(a(0), a(1), a(2)))),
            id::MODBUS => Some(Val::I32(modbus::call(a(0), a(1), a(2), a(3), a(4)))),
            id::RETAIN_TEXT => {
                let mut mem = mem;
                Some(Val::I32(retain::retain_text(
                    a(0),
                    a(1),
                    a(2),
                    a(3),
                    &mut mem,
                )))
            }
            id::NET => {
                let mut mem = mem;
                let op = a(0);
                Some(Val::I32(if (6..=8).contains(&op) {
                    eth::call(op, a(1), a(2), a(3), &mut mem)
                } else if (9..=16).contains(&op) {
                    mqtt::call(op, a(1), a(2), a(3), a(4), &mut mem)
                } else if (17..=24).contains(&op) {
                    web::call(op, a(1), a(2), a(3), a(4), &mut mem)
                } else if (30..=33).contains(&op) {
                    sparkplug::call(op, a(1), a(2), a(3), &mut mem)
                } else {
                    net::call(op, a(1), a(2), a(3), &mut mem)
                }))
            }
            _ => None,
        }
    }
}
