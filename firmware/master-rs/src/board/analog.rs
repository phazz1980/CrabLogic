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

/// Raw 12-bit reading of `pin`, 0 for a pin without ADC or a failed reading (ADC2 while Wi-Fi
/// runs on the ESP32), as `analogRead` answers.
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
