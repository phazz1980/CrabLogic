//! The watchdog of the scan (`specs/safety-audit`). The interrupt of the cycle timer (`tick.rs`,
//! on the scan core, in IRAM) checks a deadline the scan moves forward before every step: a step
//! (module, driver, AOT code, the master itself) that does not come back in time puts the outputs
//! into their safe state, notes the trip in RTC memory and aborts. ESP-IDF then writes the crash
//! dump and restarts the board; restarts in a row lead to the safe mode (`boot_guard`).
//!
//! The task watchdog of ESP-IDF is not used for this: its timeout is shared with the idle-task
//! checks of core 0, and a timeout of 0.2 s would restart the board on a long flash write there.
//! A hang with the interrupts off is caught by the interrupt watchdog of ESP-IDF.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use crab_kernel::lease::{Fault, PinUse};
use esp_idf_svc::sys;

/// Deadline for a known long piece of work between steps (module start, flash writes), ms.
pub const HOLD_MS: u32 = 5000;
/// Shortest deadline, ms.
const MIN_MS: u32 = 200;
/// Deadline in cycle periods.
const PERIODS: u32 = 100;
/// GPIOs the table covers (ESP32: 0–39, C3: 0–21).
const PINS: usize = 40;

/// Safe state of an output for the trip: 0 none, 1 low, 2 high, 3 hold.
static SAFE: [AtomicU8; PINS] = [const { AtomicU8::new(0) }; PINS];
/// The deadline, ms since boot (wrapping); checked only while armed.
static DEADLINE_MS: AtomicU32 = AtomicU32::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);
/// The allowed step time, ms (from the period).
static LIMIT_MS: AtomicU32 = AtomicU32::new(MIN_MS);
/// Set once the trip started (the abort takes a while).
static TRIPPED: AtomicBool = AtomicBool::new(false);

const TRIP_MAGIC: u32 = 0x5CA1_D065;

/// What the last restart was caused by, kept over the reset.
#[repr(C)]
struct Trip {
    magic: u32,
    /// The deadline that was missed, ms.
    limit_ms: u32,
}

#[link_section = ".rtc_noinit.crab_trip"]
static mut TRIP: Trip = Trip {
    magic: 0,
    limit_ms: 0,
};

fn now_ms() -> u32 {
    // SAFETY: no arguments, no state; in IRAM.
    (unsafe { sys::esp_timer_get_time() } / 1000) as u32
}

/// The deadline for a cycle period: 100 periods, at least 200 ms.
pub fn limit_for(period_ms: u32) -> u32 {
    period_ms.saturating_mul(PERIODS).max(MIN_MS)
}

/// Sets the allowed time for the period of the cycle.
pub fn set_period(period_ms: u32) {
    LIMIT_MS.store(limit_for(period_ms), Ordering::Relaxed);
}

/// The scan is about to step: the next deadline is one limit away. Arms the watchdog.
#[inline]
pub fn feed() {
    DEADLINE_MS.store(
        now_ms().wrapping_add(LIMIT_MS.load(Ordering::Relaxed)),
        Ordering::Relaxed,
    );
    ARMED.store(true, Ordering::Release);
}

/// A known long piece of work between steps (a module start, an image write): `ms` from now.
pub fn hold(ms: u32) {
    let ms = ms.max(LIMIT_MS.load(Ordering::Relaxed));
    DEADLINE_MS.store(now_ms().wrapping_add(ms), Ordering::Relaxed);
}

/// Called by the cycle timer interrupt on every mark.
#[link_section = ".iram1.crab_wdt"]
pub fn check() {
    if !ARMED.load(Ordering::Acquire) {
        return;
    }
    let late = now_ms().wrapping_sub(DEADLINE_MS.load(Ordering::Relaxed)) as i32;
    if late > 0 && !TRIPPED.swap(true, Ordering::Relaxed) {
        trip();
    }
}

#[link_section = ".iram1.crab_wdt_trip"]
fn trip() -> ! {
    for (pin, state) in SAFE.iter().enumerate() {
        let high = match state.load(Ordering::Relaxed) {
            1 => false,
            2 => true,
            _ => continue,
        };
        // SAFETY: the pin was an output of a module; the ROM call takes it back from LEDC (PWM)
        // to the plain output register, then the level is set.
        unsafe {
            sys::esp_rom_gpio_connect_out_signal(pin as u32, sys::SIG_GPIO_OUT_IDX, false, false);
            sys::gpio_set_level(pin as i32, u32::from(high));
        }
    }
    // Volatile: the optimizer must not shrink a field that only ever holds two values (it did,
    // and the garbage of RTC memory after power-on read as a trip).
    // SAFETY: written once, here, before the restart; read at the next boot.
    unsafe {
        let trip = core::ptr::addr_of_mut!(TRIP);
        core::ptr::addr_of_mut!((*trip).limit_ms).write_volatile(LIMIT_MS.load(Ordering::Relaxed));
        core::ptr::addr_of_mut!((*trip).magic).write_volatile(TRIP_MAGIC);
    }
    // SAFETY: the system abort of ESP-IDF: dump, then restart; never returns.
    unsafe { sys::esp_system_abort(c"scan watchdog: the step did not end in time".as_ptr()) }
}

/// The limit of the trip that caused this boot, ms; 0 for none.
static BOOT_TRIP: AtomicU32 = AtomicU32::new(0);

/// Takes the note of a trip from RTC memory; called once at start, before the watchdog is armed.
pub fn take_trip() {
    // SAFETY: read and cleared once, from the runtime thread, before anything else uses it.
    let found = unsafe {
        let trip = core::ptr::addr_of_mut!(TRIP);
        let magic = core::ptr::addr_of!((*trip).magic).read_volatile();
        let limit = core::ptr::addr_of!((*trip).limit_ms).read_volatile();
        core::ptr::addr_of_mut!((*trip).magic).write_volatile(0);
        (magic == TRIP_MAGIC).then_some(limit.clamp(1, 3_600_000))
    };
    BOOT_TRIP.store(found.unwrap_or(0), Ordering::Relaxed);
}

/// The limit of the trip that caused this boot, ms (`info` prints it).
pub fn boot_trip() -> Option<u32> {
    let ms = BOOT_TRIP.load(Ordering::Relaxed);
    (ms != 0).then_some(ms)
}

/// Keeps the safe state of a leased output for the trip.
pub fn leased(pin: u8, use_: PinUse, fault: Fault) {
    let _ = use_;
    if let Some(slot) = SAFE.get(pin as usize) {
        slot.store(
            match fault {
                Fault::Low => 1,
                Fault::High => 2,
                Fault::Hold => 3,
            },
            Ordering::Relaxed,
        );
    }
}

pub fn ended(pin: u8) {
    if let Some(slot) = SAFE.get(pin as usize) {
        slot.store(0, Ordering::Relaxed);
    }
}
