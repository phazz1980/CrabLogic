//! The hardware of the ESP32-C3 as modules see it. So far: digital pins and the clock;
//! the other imports answer 0 until their drivers move over from the C++ master.

use crab_kernel::abi::id;
use crab_kernel::host::{Devices, Mem, Val};
use crab_kernel::lease::{Fault, Hal, PinUse};
use esp_idf_svc::sys;

/// Pins a module may drive on the C3: 11–17 are the SPI flash, 18–19 the USB console.
fn usable(pin: u8) -> bool {
    matches!(pin, 0..=10 | 20 | 21)
}

fn write_pin(pin: u8, high: bool) {
    if !usable(pin) {
        return;
    }
    // SAFETY: the pin number was checked; the calls only change this pin.
    unsafe {
        sys::gpio_set_direction(pin as i32, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        sys::gpio_set_level(pin as i32, high as u32);
    }
}

fn millis() -> i32 {
    // SAFETY: no arguments, no state.
    (unsafe { sys::esp_timer_get_time() } / 1000) as i32
}

/// Puts outputs into their safe state when a lease ends.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardHal;

impl Hal for BoardHal {
    fn safe(&self, pin: u8, _use: PinUse, fault: Fault) {
        write_pin(pin, fault == Fault::High);
    }
}

/// The devices of one module.
#[derive(Debug, Default)]
pub struct BoardDevices;

impl Devices for BoardDevices {
    type Hal = BoardHal;

    fn hal(&self) -> BoardHal {
        BoardHal
    }

    fn call(&mut self, import: usize, args: &[Val], _mem: Mem<'_>) -> Option<Val> {
        let a = |i: usize| args.get(i).map_or(0, |v| v.i());
        match import {
            id::GPIO_WRITE => {
                write_pin((a(0) & 0xFF) as u8, a(1) != 0);
                None
            }
            id::GPIO_READ => {
                let pin = (a(0) & 0xFF) as u8;
                // SAFETY: the pin number is checked first.
                Some(Val::I32(if usable(pin) {
                    unsafe { sys::gpio_get_level(pin as i32) }
                } else {
                    0
                }))
            }
            id::MILLIS => Some(Val::I32(millis())),
            _ => None,
        }
    }
}
