//! Analog inputs (`adc_read`) and PWM outputs (`pwm_write`), as the C++ master does them with
//! `analogRead` and `analogWrite`: 12-bit readings at the widest range, PWM of 8 bits at 1 kHz.

use esp_idf_svc::sys;
use std::sync::Mutex;

/// Duty bits of the PWM (`pwm_write` takes 0..=255).
const PWM_BITS: u32 = 8;
const PWM_HZ: u32 = 1000;
/// LEDC channels in low-speed mode: 8 on the ESP32, 6 on the C3.
#[cfg(target_arch = "riscv32")]
const PWM_CHANNELS: usize = 6;
#[cfg(not(target_arch = "riscv32"))]
const PWM_CHANNELS: usize = 8;

/// An ADC unit opened once; the handle is only used under the mutex.
struct Unit(sys::adc_oneshot_unit_handle_t);
// SAFETY: the handle is a pointer to a driver object; every use is under `ADC`.
unsafe impl Send for Unit {}

struct Adc {
    units: [Option<Unit>; 2],
    /// Configured channels, bit per channel, per unit.
    channels: [u32; 2],
}

static ADC: Mutex<Adc> = Mutex::new(Adc {
    units: [None, None],
    channels: [0, 0],
});

/// Continuous (DMA) reader of ADC1. A oneshot read costs ~39 us on the C3 and blocks the sheet;
/// the continuous driver samples in the background and a read only drains a buffer.
///
/// On the classic ESP32 the continuous driver runs on I2S0 internally; nothing else in the
/// master uses I2S (checked by grep), so there is no conflict.
mod cont {
    use super::sys;
    use crab_master_rs::adc_acc::AdcAccum;

    /// Total sample rate over all channels (allowed: 20 kHz..2 MHz on the ESP32, 611 Hz..83 kHz on
    /// the C3).
    const SAMPLE_HZ: u32 = 20_000;
    /// One DMA frame, in bytes (a multiple of the 4-byte conversion unit of both chips).
    const FRAME_BYTES: u32 = 256;
    /// Driver pool, in bytes: ~25 ms of samples on the C3 (4 bytes each), ~50 ms on the ESP32.
    const POOL_BYTES: u32 = 2048;
    /// Drain buffer size, a multiple of `RESULT_BYTES`.
    const DRAIN_BYTES: usize = 512;
    /// Channels of one pattern (ADC1 has at most 8).
    const MAX_PATTERN: usize = 8;
    /// Bytes of one result: TYPE2 on the C3, TYPE1 on the ESP32.
    #[cfg(target_arch = "riscv32")]
    const RESULT_BYTES: usize = 4;
    #[cfg(not(target_arch = "riscv32"))]
    const RESULT_BYTES: usize = 2;
    #[cfg(target_arch = "riscv32")]
    const FORMAT: sys::adc_digi_output_format_t =
        sys::adc_digi_output_format_t_ADC_DIGI_OUTPUT_FORMAT_TYPE2;
    #[cfg(not(target_arch = "riscv32"))]
    const FORMAT: sys::adc_digi_output_format_t =
        sys::adc_digi_output_format_t_ADC_DIGI_OUTPUT_FORMAT_TYPE1;
    /// Longest wait for the first sample of a new channel: 100 x 100 us, more than one frame
    /// (6.4 ms on the ESP32, 3.2 ms on the C3), since the driver hands over whole frames.
    const FIRST_SAMPLE_TRIES: u32 = 100;

    pub struct Cont {
        handle: sys::adc_continuous_handle_t,
        acc: AdcAccum,
        running: bool,
        /// Continuous mode is given up for good; ADC1 goes through oneshot.
        failed: bool,
        /// Drain buffer: static, no allocation per read.
        buf: [u8; DRAIN_BYTES],
    }

    // SAFETY: the handle is a pointer to a driver object; every use is under the `CONT` mutex.
    unsafe impl Send for Cont {}

    impl Cont {
        pub const fn new() -> Self {
            Self {
                handle: core::ptr::null_mut(),
                acc: AdcAccum::new(),
                running: false,
                failed: false,
                buf: [0; DRAIN_BYTES],
            }
        }

        /// Reads whatever the driver has, without waiting, into the accumulators.
        fn drain(&mut self) {
            if !self.running {
                return;
            }
            // Bounded: more than the whole pool cannot be queued.
            for _ in 0..(POOL_BYTES as usize / DRAIN_BYTES + 2) {
                let mut got: u32 = 0;
                // SAFETY: a started handle; the buffer outlives the call and its length is passed.
                let err = unsafe {
                    sys::adc_continuous_read(
                        self.handle,
                        self.buf.as_mut_ptr(),
                        DRAIN_BYTES as u32,
                        &mut got,
                        0,
                    )
                };
                // Overflow (ESP_ERR_INVALID_STATE) may still return the data that was kept.
                if got > 0 {
                    self.acc.feed(&self.buf[..got as usize], RESULT_BYTES);
                }
                if got == 0 || (err != 0 && err != sys::ESP_ERR_INVALID_STATE as i32) {
                    break; // timeout: the buffer is empty
                }
            }
        }

        /// Stops and frees the driver and gives ADC1 back to oneshot for good.
        fn give_up(&mut self, why: &str) {
            println!("adc: continuous mode off ({why}); ADC1 uses oneshot reads");
            if !self.handle.is_null() {
                // SAFETY: a handle made by `adc_continuous_new_handle`; freed once and nulled.
                unsafe {
                    if self.running {
                        sys::adc_continuous_stop(self.handle);
                    }
                    sys::adc_continuous_deinit(self.handle);
                }
                self.handle = core::ptr::null_mut();
            }
            self.running = false;
            self.failed = true;
        }

        /// (Re)starts the driver with every known channel.
        fn configure(&mut self) -> Result<(), &'static str> {
            if self.handle.is_null() {
                let mut cfg = sys::adc_continuous_handle_cfg_t {
                    max_store_buf_size: POOL_BYTES,
                    conv_frame_size: FRAME_BYTES,
                    ..Default::default()
                };
                // A full pool is emptied instead of dropping new frames: after a long pause
                // between reads (a slow sheet) the mean is of fresh samples, not of old ones.
                cfg.flags.set_flush_pool(1);
                let mut handle: sys::adc_continuous_handle_t = core::ptr::null_mut();
                // SAFETY: the config lives through the call; the handle is kept in `self`.
                if unsafe { sys::adc_continuous_new_handle(&cfg, &mut handle) } != 0 {
                    return Err("new_handle");
                }
                self.handle = handle;
            } else if self.running {
                // SAFETY: a started handle.
                unsafe { sys::adc_continuous_stop(self.handle) };
                self.running = false;
            }
            let mut pattern = [sys::adc_digi_pattern_config_t::default(); MAX_PATTERN];
            let mut n = 0;
            for ch in self.acc.channels().take(MAX_PATTERN) {
                pattern[n] = sys::adc_digi_pattern_config_t {
                    atten: sys::adc_atten_t_ADC_ATTEN_DB_12 as u8,
                    channel: ch,
                    unit: sys::adc_unit_t_ADC_UNIT_1 as u8,
                    bit_width: sys::adc_bitwidth_t_ADC_BITWIDTH_12 as u8,
                };
                n += 1;
            }
            let cfg = sys::adc_continuous_config_t {
                pattern_num: n as u32,
                adc_pattern: pattern.as_mut_ptr(),
                sample_freq_hz: SAMPLE_HZ,
                conv_mode: sys::adc_digi_convert_mode_t_ADC_CONV_SINGLE_UNIT_1,
                format: FORMAT,
            };
            // SAFETY: a stopped handle; the pattern array outlives the call (the driver copies it).
            if unsafe { sys::adc_continuous_config(self.handle, &cfg) } != 0 {
                return Err("config");
            }
            // SAFETY: a configured handle.
            if unsafe { sys::adc_continuous_start(self.handle) } != 0 {
                return Err("start");
            }
            self.running = true;
            Ok(())
        }

        /// Mean of the samples of ADC1 `channel` since the previous read, or `None` when
        /// continuous mode is unavailable and the caller must use oneshot.
        pub fn read(&mut self, channel: u8) -> Option<i32> {
            if self.failed {
                return None;
            }
            if self.acc.is_known(channel) {
                self.drain();
            } else {
                if self.acc.channels().count() >= MAX_PATTERN {
                    self.give_up("too many channels");
                    return None;
                }
                self.drain(); // keep what the old pattern collected
                self.acc.add_channel(channel);
                if let Err(why) = self.configure() {
                    self.give_up(why);
                    return None;
                }
                for _ in 0..FIRST_SAMPLE_TRIES {
                    self.drain();
                    if self.acc.has_sample(channel) {
                        break;
                    }
                    // SAFETY: a busy-wait ROM routine, no preconditions.
                    unsafe { sys::esp_rom_delay_us(100) };
                }
            }
            Some(self.acc.take(channel))
        }
    }
}

static CONT: Mutex<cont::Cont> = Mutex::new(cont::Cont::new());

/// Raw 12-bit reading of `pin`, 0 for a pin without ADC or a failed reading (ADC2 while Wi-Fi
/// runs on the ESP32), as `analogRead` answers. ADC1 pins are read through the continuous (DMA)
/// driver: the mean of the samples since the previous read; ADC2 pins (and ADC1 if the
/// continuous driver cannot start) through oneshot reads.
pub fn adc_read(pin: u8) -> i32 {
    let mut unit: sys::adc_unit_t = 0;
    let mut channel: sys::adc_channel_t = 0;
    // SAFETY: plain out-parameters; an unknown pin is an error, not a fault.
    if unsafe { sys::adc_oneshot_io_to_channel(i32::from(pin), &mut unit, &mut channel) } != 0 {
        return 0;
    }
    let index = unit as usize;
    if index > 1 {
        return 0;
    }
    if index == 0 {
        let mut cont = CONT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = cont.read(channel as u8) {
            return value;
        }
        // Continuous mode is off for good (its driver is already freed): use oneshot.
    }
    oneshot_read(index, unit, channel)
}

fn oneshot_read(index: usize, unit: sys::adc_unit_t, channel: sys::adc_channel_t) -> i32 {
    let mut adc = ADC.lock().unwrap_or_else(|e| e.into_inner());
    if adc.units[index].is_none() {
        let cfg = sys::adc_oneshot_unit_init_cfg_t {
            unit_id: unit,
            ..Default::default()
        };
        let mut handle: sys::adc_oneshot_unit_handle_t = core::ptr::null_mut();
        // SAFETY: the config lives through the call; the handle is kept for good.
        if unsafe { sys::adc_oneshot_new_unit(&cfg, &mut handle) } != 0 {
            return 0;
        }
        adc.units[index] = Some(Unit(handle));
    }
    let handle = adc.units[index]
        .as_ref()
        .map(|u| u.0)
        .expect("opened above");
    if adc.channels[index] >> channel & 1 == 0 {
        let cfg = sys::adc_oneshot_chan_cfg_t {
            atten: sys::adc_atten_t_ADC_ATTEN_DB_12,
            bitwidth: sys::adc_bitwidth_t_ADC_BITWIDTH_12,
        };
        // SAFETY: a valid handle and channel of that unit.
        if unsafe { sys::adc_oneshot_config_channel(handle, channel, &cfg) } != 0 {
            return 0;
        }
        adc.channels[index] |= 1 << channel;
    }
    let mut raw = 0;
    // SAFETY: a valid handle and a configured channel.
    if unsafe { sys::adc_oneshot_read(handle, channel, &mut raw) } != 0 {
        return 0;
    }
    raw
}

struct Pwm {
    timer: bool,
    /// Pin of each LEDC channel.
    pins: [Option<u8>; PWM_CHANNELS],
}

static PWM: Mutex<Pwm> = Mutex::new(Pwm {
    timer: false,
    pins: [None; PWM_CHANNELS],
});

const MODE: sys::ledc_mode_t = sys::ledc_mode_t_LEDC_LOW_SPEED_MODE;

/// Sets the PWM of `pin` to `value` of 255 (255 is fully on, as `analogWrite`). The first write
/// takes a free LEDC channel; when none is left, the write is ignored.
pub fn pwm_write(pin: u8, value: i32) {
    let mut pwm = PWM.lock().unwrap_or_else(|e| e.into_inner());
    if !pwm.timer {
        let cfg = sys::ledc_timer_config_t {
            speed_mode: MODE,
            duty_resolution: PWM_BITS,
            timer_num: sys::ledc_timer_t_LEDC_TIMER_0,
            freq_hz: PWM_HZ,
            ..Default::default()
        };
        // SAFETY: the config lives through the call.
        if unsafe { sys::ledc_timer_config(&cfg) } != 0 {
            return;
        }
        pwm.timer = true;
    }
    let channel = match pwm.pins.iter().position(|p| *p == Some(pin)) {
        Some(channel) => channel,
        None => {
            let Some(free) = pwm.pins.iter().position(Option::is_none) else {
                return;
            };
            let cfg = sys::ledc_channel_config_t {
                gpio_num: i32::from(pin),
                speed_mode: MODE,
                channel: free as sys::ledc_channel_t,
                timer_sel: sys::ledc_timer_t_LEDC_TIMER_0,
                ..Default::default()
            };
            // SAFETY: the config lives through the call; the pin was checked by the caller.
            if unsafe { sys::ledc_channel_config(&cfg) } != 0 {
                return;
            }
            pwm.pins[free] = Some(pin);
            free
        }
    };
    let max = 1u32 << PWM_BITS;
    let duty = match value {
        v if v >= 255 => max,
        v if v <= 0 => 0,
        v => v as u32,
    };
    // SAFETY: a configured channel.
    unsafe {
        sys::ledc_set_duty(MODE, channel as sys::ledc_channel_t, duty);
        sys::ledc_update_duty(MODE, channel as sys::ledc_channel_t);
    }
}

/// Safe state of a PWM pin whose module is gone: fully on, or the channel freed and the pin a low
/// output (as the C++ master). Returns whether the pin had a channel.
pub fn pwm_safe(pin: u8, high: bool) -> bool {
    if high {
        let has = PWM
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pins
            .contains(&Some(pin));
        if has {
            pwm_write(pin, 255);
        }
        return has;
    }
    let mut pwm = PWM.lock().unwrap_or_else(|e| e.into_inner());
    let Some(channel) = pwm.pins.iter().position(|p| *p == Some(pin)) else {
        return false;
    };
    // SAFETY: a configured channel; idle level low.
    unsafe {
        sys::ledc_stop(MODE, channel as sys::ledc_channel_t, 0);
    }
    pwm.pins[channel] = None;
    true
}
