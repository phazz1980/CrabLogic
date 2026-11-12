//! Level lengths of a single-wire answer (DHT22) measured by the RMT receiver with interrupts on.
//!
//! One RX channel per pin is created on first use and kept (`release` frees it when the module that
//! used the pin goes away). The calling task sleeps on a binary semaphore that the `on_recv_done`
//! callback gives from the RMT interrupt; nothing here turns interrupts off.
//!
//! How the first level is measured: the RMT receiver is armed before the line is released and starts
//! recording at the first edge of the input (the IDF infrared examples rely on the same behaviour:
//! `rmt_receive` waits for a frame of any delay). The release of the open-drain line is that first
//! edge (low to high), so the first recorded level is the high after the release: its length is the
//! time from the release to the falling edge of the sensor answer, as the polled loop measured it.
//! A level longer than the idle threshold ends the reception (`signal_range_max_ns`); like in the
//! polled loop, that level and the rest are not returned.
//!
//! Limits: resolution 1 MHz (1 tick = 1 µs), glitch filter 1 µs. Memory block: ESP32 128 symbols
//! (two blocks of 64, no ping-pong on this chip, so a long signal must fit; 128 levels = 64 symbols
//! plus the end marker), C3 48 symbols (one block, ping-pong copies the halves to our buffer).
//! Channels: ESP32 8 RX channels, so 4 pins at once; C3 2 RX channels, so 2 pins at once. When no
//! channel can be made, [`measure`] returns `None` and the caller falls back to the polled loop.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use esp_idf_svc::sys;

/// Most levels one measurement returns (the `bus.pulse_read` limit).
pub(super) const MAX_LEVELS: usize = 128;
/// A level longer than this ends the measurement, µs.
pub(super) const IDLE_US: u32 = 300;
/// Glitches shorter than this are ignored, ns.
const FILTER_NS: u32 = 1000;
/// Hardware memory of one channel, symbols (two levels each).
#[cfg(target_arch = "riscv32")]
const MEM_SYMBOLS: usize = 48;
#[cfg(not(target_arch = "riscv32"))]
const MEM_SYMBOLS: usize = 128;
/// Our buffer: all levels, the end marker and a spare symbol.
const BUF_SYMBOLS: usize = MAX_LEVELS / 2 + 2;
/// `queueQUEUE_TYPE_BINARY_SEMAPHORE` of FreeRTOS.
const QUEUE_TYPE_BINARY_SEMAPHORE: u8 = 3;

/// One RX channel: the interrupt writes `symbols` and `got` and gives `done`.
struct Channel {
    pin: i32,
    handle: sys::rmt_channel_handle_t,
    done: sys::QueueHandle_t,
    got: AtomicUsize,
    symbols: [u32; BUF_SYMBOLS],
}

// SAFETY: the raw handles are used only under the `CHANNELS` mutex (or by the callback, which
// touches `got` atomically and the semaphore, both safe from an interrupt).
unsafe impl Send for Channel {}

static CHANNELS: Mutex<Vec<Box<Channel>>> = Mutex::new(Vec::new());

/// `on_recv_done`: remembers how many symbols came and wakes the waiting task. Runs in the RMT
/// interrupt.
unsafe extern "C" fn on_recv_done(
    _channel: sys::rmt_channel_handle_t,
    event: *const sys::rmt_rx_done_event_data_t,
    ctx: *mut c_void,
) -> bool {
    // SAFETY: `ctx` is the boxed `Channel` registered below (it lives until `rmt_del_channel`,
    // which waits for the interrupt to be gone); `event` is valid for the call.
    let (channel, symbols) = unsafe { (&*(ctx as *const Channel), (*event).num_symbols) };
    channel.got.store(symbols, Ordering::Release);
    let mut woken: sys::BaseType_t = 0;
    // SAFETY: the semaphore lives as long as the channel; this is the ISR variant.
    unsafe { sys::xQueueGiveFromISR(channel.done, &mut woken) };
    woken != 0
}

/// Makes the channel of `pin` (the line is claimed by the caller). `None` when the driver refuses.
fn create(pin: i32) -> Option<Box<Channel>> {
    // SAFETY: a plain constructor call; the queue is deleted with the channel.
    let done = unsafe { sys::xQueueGenericCreate(1, 0, QUEUE_TYPE_BINARY_SEMAPHORE) };
    if done.is_null() {
        return None;
    }
    let mut channel = Box::new(Channel {
        pin,
        handle: core::ptr::null_mut(),
        done,
        got: AtomicUsize::new(0),
        symbols: [0; BUF_SYMBOLS],
    });
    let cfg = sys::rmt_rx_channel_config_t {
        gpio_num: pin,
        clk_src: sys::soc_periph_rmt_clk_src_t_RMT_CLK_SRC_DEFAULT,
        resolution_hz: 1_000_000,
        mem_block_symbols: MEM_SYMBOLS,
        ..Default::default()
    };
    let callbacks = sys::rmt_rx_event_callbacks_t {
        on_recv_done: Some(on_recv_done),
    };
    let ctx = (&*channel as *const Channel).cast_mut().cast::<c_void>();
    // SAFETY: the config lives through the call; the callback context is the heap `Channel` whose
    // address does not change when the box moves into the table.
    let ok = unsafe {
        sys::rmt_new_rx_channel(&cfg, &mut channel.handle) == 0
            && sys::rmt_rx_register_event_callbacks(channel.handle, &callbacks, ctx) == 0
            && sys::rmt_enable(channel.handle) == 0
    };
    if !ok {
        destroy(&mut channel);
        return None;
    }
    // `rmt_new_rx_channel` reconfigured the pin as a plain input (that is how it takes the signal
    // through the GPIO matrix): give the open-drain output back and release the line.
    // SAFETY: the pin is claimed and was set up for 1-Wire by the caller.
    unsafe {
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY);
        sys::gpio_set_level(pin, 1);
    }
    Some(channel)
}

/// Frees the driver objects of a channel (also the half-made ones).
fn destroy(channel: &mut Channel) {
    if !channel.handle.is_null() {
        // SAFETY: a handle from `rmt_new_rx_channel`; disable fails harmlessly if not enabled.
        unsafe {
            sys::rmt_disable(channel.handle);
            sys::rmt_del_channel(channel.handle);
        }
        channel.handle = core::ptr::null_mut();
    }
    if !channel.done.is_null() {
        // SAFETY: the interrupt callback cannot run any more (the channel is deleted).
        unsafe { sys::vQueueDelete(channel.done) };
        channel.done = core::ptr::null_mut();
    }
}

/// Frees the channel of `pin`, if there is one (the module that used the pin is gone). The driver
/// resets the pin; the caller does that anyway.
pub(super) fn release(pin: i32) {
    let mut table = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = table.iter().position(|c| c.pin == pin) {
        let mut channel = table.swap_remove(at);
        destroy(&mut channel);
    }
}

fn wait_ticks(ms: u32) -> u32 {
    (ms * sys::configTICK_RATE_HZ).div_ceil(1000) + 1
}

/// Makes the channel of `pin` ahead of the start pulse: making it reconfigures the pin and would
/// cut a start pulse short. False when no channel can be made (the caller falls back).
pub(super) fn prepare(pin: i32) -> bool {
    let mut table = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    if table.iter().any(|c| c.pin == pin) {
        return true;
    }
    match create(pin) {
        Some(channel) => {
            table.push(channel);
            true
        }
        None => false,
    }
}

/// Measures up to `count` levels (µs, in time order) on `pin`. `start` runs after the receiver is
/// armed and must release the line (or start the signal). `None` means no RMT channel is
/// available (`start` was not called); an empty vector means nothing came before the timeout.
pub(super) fn measure(pin: i32, count: usize, start: &mut dyn FnMut()) -> Option<Vec<u16>> {
    let count = count.min(MAX_LEVELS);
    let mut table = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    let at = match table.iter().position(|c| c.pin == pin) {
        Some(at) => at,
        None => {
            table.push(create(pin)?);
            table.len() - 1
        }
    };
    let channel = &mut *table[at];
    channel.got.store(0, Ordering::Release);
    // A stale give (an earlier timeout) must not end this measurement at once.
    // SAFETY: the semaphore is valid; a zero wait only polls it.
    unsafe { sys::xQueueSemaphoreTake(channel.done, 0) };
    let cfg = sys::rmt_receive_config_t {
        signal_range_min_ns: FILTER_NS,
        signal_range_max_ns: IDLE_US * 1000,
        ..Default::default()
    };
    // SAFETY: the buffer lives in the boxed channel, which stays in the table (locked) until the
    // callback has run or the channel is disabled below.
    let armed = unsafe {
        sys::rmt_receive(
            channel.handle,
            channel.symbols.as_mut_ptr().cast::<c_void>(),
            core::mem::size_of_val(&channel.symbols),
            &cfg,
        ) == 0
    };
    if !armed {
        return None;
    }
    start();
    let timeout_ms = 20 + count as u32 / 5;
    // SAFETY: the semaphore is valid; the task sleeps until the interrupt gives it.
    let came = unsafe { sys::xQueueSemaphoreTake(channel.done, wait_ticks(timeout_ms)) } != 0;
    if !came {
        // Abort the reception (valid in the run state) and arm the channel again for next time.
        // SAFETY: the handle is valid; the callback may have fired meanwhile, which only makes
        // `rmt_disable` fail (state already enabled) and `rmt_enable` fail: both are harmless.
        unsafe {
            sys::rmt_disable(channel.handle);
            sys::rmt_enable(channel.handle);
        }
        return Some(Vec::new());
    }
    let symbols = channel.got.load(Ordering::Acquire).min(BUF_SYMBOLS);
    Some(levels(&channel.symbols[..symbols], count))
}

/// The level lengths of `RMT` symbols: two per symbol (duration 15 bits at bit 0 and 16, level at
/// bit 15 and 31); a zero length is the end marker, a length over the idle threshold ends the list.
fn levels(symbols: &[u32], count: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(count);
    for word in symbols {
        for length in [word & 0x7FFF, (word >> 16) & 0x7FFF] {
            if length == 0 || length > IDLE_US || out.len() >= count {
                return out;
            }
            out.push(length as u16);
        }
    }
    out
}

/// LEDC channel for the self-test: the last one, because `analog::pwm_write` takes the first free.
#[cfg(target_arch = "riscv32")]
const TEST_CHANNEL: sys::ledc_channel_t = 5;
#[cfg(not(target_arch = "riscv32"))]
const TEST_CHANNEL: sys::ledc_channel_t = 7;
const TEST_MODE: sys::ledc_mode_t = sys::ledc_mode_t_LEDC_LOW_SPEED_MODE;
/// 5 kHz, 30 % duty of 8 bits (77 of 256): 60 µs high, 140 µs low.
const TEST_HZ: u32 = 5000;
const TEST_DUTY: u32 = 77;
/// The signal runs this long, then the line stays low and the reception ends by idle.
const TEST_RUN_US: u64 = 4000;
const TEST_LEVELS: usize = 16;

unsafe extern "C" fn stop_pwm(_arg: *mut c_void) {
    // SAFETY: the channel was configured by the self-test; stopping parks the pin low.
    unsafe { sys::ledc_stop(TEST_MODE, TEST_CHANNEL, 0) };
}

/// Why a pin cannot be used for the self-test (flash, USB, console, input only, no such pin).
fn pin_problem(pin: i32) -> Option<&'static str> {
    use crab_master_rs::eth_cfg::{pin_problem, Chip};
    #[cfg(target_arch = "riscv32")]
    let chip = Chip::Esp32c3;
    #[cfg(not(target_arch = "riscv32"))]
    let chip = Chip::Esp32;
    if chip == Chip::Esp32 && matches!(pin, 1 | 3) {
        return Some("console");
    }
    pin_problem(chip, pin, false)
}

/// The pin as a released open-drain line first, as `bus.pulse_read` finds it.
fn claim_pin(pin: i32) {
    // SAFETY: the pin was checked against the chip; a test on a pin used by a running module is the
    // operator's choice.
    unsafe {
        sys::gpio_reset_pin(pin);
        super::devices::forget_output(pin);
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
        sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY);
        sys::gpio_set_level(pin, 1);
    }
}

/// Starts the 5 kHz / 30 % test signal on `pin` and keeps the input readable.
///
/// # Safety
/// The pin must be claimed by the caller and not driven by anything else.
unsafe fn start_pwm(pin: i32) {
    let timer_cfg = sys::ledc_timer_config_t {
        speed_mode: TEST_MODE,
        duty_resolution: sys::ledc_timer_bit_t_LEDC_TIMER_8_BIT,
        timer_num: sys::ledc_timer_t_LEDC_TIMER_3,
        freq_hz: TEST_HZ,
        ..Default::default()
    };
    let channel_cfg = sys::ledc_channel_config_t {
        gpio_num: pin,
        speed_mode: TEST_MODE,
        channel: TEST_CHANNEL,
        timer_sel: sys::ledc_timer_t_LEDC_TIMER_3,
        duty: TEST_DUTY,
        ..Default::default()
    };
    // SAFETY: configs live through the calls.
    unsafe {
        sys::ledc_timer_config(&timer_cfg);
        sys::ledc_channel_config(&channel_cfg);
        // LEDC sets the pin output-only, which cuts the input the receiver listens to: turn the
        // input back on (this routes the output to plain GPIO) and give the output to LEDC again.
        sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
        sys::esp_rom_gpio_connect_out_signal(
            pin as u32,
            sys::LEDC_LS_SIG_OUT0_IDX + TEST_CHANNEL,
            false,
            false,
        );
    }
}

/// `pulsetest <pin>`: LEDC makes 5 kHz / 30 % on the pin while the RMT receiver listens to the same
/// pin through the GPIO matrix; prints the first levels (expected: 140 60 140 60 ...).
pub(super) fn self_test(pin: i32) {
    if let Some(why) = pin_problem(pin) {
        println!("err pin {pin}: {why}");
        return;
    }
    claim_pin(pin);
    let mut timer: sys::esp_timer_handle_t = core::ptr::null_mut();
    let mut start = || {
        let args = sys::esp_timer_create_args_t {
            callback: Some(stop_pwm),
            name: c"pulsetest".as_ptr(),
            ..Default::default()
        };
        // SAFETY: the args live through the call; the timer handle outlives the measurement and
        // is deleted below; the callback only stops the test channel.
        unsafe {
            start_pwm(pin);
            sys::esp_timer_create(&args, &mut timer);
            if !timer.is_null() {
                sys::esp_timer_start_once(timer, TEST_RUN_US);
            }
        }
    };
    let result = measure(pin, TEST_LEVELS, &mut start);
    // SAFETY: stop (idempotent), delete the timer made in `start`, and give the pin back.
    unsafe {
        sys::ledc_stop(TEST_MODE, TEST_CHANNEL, 0);
        if !timer.is_null() {
            sys::esp_timer_stop(timer);
            sys::esp_timer_delete(timer);
        }
    }
    release(pin);
    // SAFETY: as above; the driver already reset the pin when the channel went.
    unsafe { sys::gpio_reset_pin(pin) };
    match result {
        None => println!("err no RMT channel"),
        Some(levels) if levels.is_empty() => println!("err timeout"),
        Some(levels) => {
            let text: Vec<String> = levels.iter().map(u16::to_string).collect();
            println!("pulses {}", text.join(" "));
            println!("ok");
        }
    }
}

/// Benchmark timer period, µs.
const BENCH_PERIOD_US: u64 = 100;
/// The signal runs this long after each measurement starts, then LEDC parks the line low. Both
/// modes use the same value: 80 levels of the 60/140 µs signal take about 8 ms, so 16 ms leaves
/// the polled loop all its levels, and the RMT (which cannot see an idle on a running signal)
/// ends on the stop at the latest.
const BENCH_STOP_US: u64 = 16_000;
/// Levels per measurement.
const BENCH_LEVELS: usize = 80;
const BENCH_MAX_RUNS: u32 = 1000;
/// Lateness bookkeeping of the benchmark timer. Only the timer task writes, so plain load and
/// store are enough; times are the low 32 bits of the µs clock (wrap-safe differences).
static LATE_EXPECTED: AtomicU32 = AtomicU32::new(0);
static LATE_MAX: AtomicU32 = AtomicU32::new(0);
static LATE_CALLS: AtomicU32 = AtomicU32::new(0);

/// Periodic callback: lateness = now - expected, expected advances by the period each call.
unsafe extern "C" fn late_tick(_arg: *mut c_void) {
    // SAFETY: a plain read of the clock.
    let now = unsafe { sys::esp_timer_get_time() } as u32;
    let calls = LATE_CALLS.load(Ordering::Relaxed);
    let expected = if calls == 0 {
        now
    } else {
        LATE_EXPECTED.load(Ordering::Relaxed)
    };
    let late = now.wrapping_sub(expected) as i32;
    if late > 0 && late as u32 > LATE_MAX.load(Ordering::Relaxed) {
        LATE_MAX.store(late as u32, Ordering::Relaxed);
    }
    LATE_EXPECTED.store(
        expected.wrapping_add(BENCH_PERIOD_US as u32),
        Ordering::Relaxed,
    );
    LATE_CALLS.store(calls + 1, Ordering::Relaxed);
}

/// Runs `runs` measurements with `measure` and prints the result line; false when it failed.
fn bench_mode(
    mode: &str,
    runs: u32,
    stop_timer: sys::esp_timer_handle_t,
    measure: &mut dyn FnMut() -> Option<Vec<u16>>,
) -> bool {
    LATE_MAX.store(0, Ordering::Relaxed);
    LATE_CALLS.store(0, Ordering::Relaxed);
    let (mut levels_sum, mut call_sum, mut call_max) = (0u64, 0i64, 0i64);
    for _ in 0..runs {
        // Resume the signal parked by the previous run.
        // SAFETY: the channel was configured by `start_pwm`.
        unsafe {
            sys::ledc_set_duty(TEST_MODE, TEST_CHANNEL, TEST_DUTY);
            sys::ledc_update_duty(TEST_MODE, TEST_CHANNEL);
        }
        // SAFETY: plain reads of the clock around the call.
        let t0 = unsafe { sys::esp_timer_get_time() };
        let result = measure();
        // SAFETY: as above.
        let spent = unsafe { sys::esp_timer_get_time() } - t0;
        let Some(levels) = result else {
            println!("err no RMT channel");
            return false;
        };
        levels_sum += levels.len() as u64;
        call_sum += spent;
        call_max = call_max.max(spent);
        std::thread::sleep(std::time::Duration::from_millis(20));
        // SAFETY: stopping an expired one-shot fails harmlessly.
        unsafe { sys::esp_timer_stop(stop_timer) };
    }
    let n = f64::from(runs);
    println!(
        "bench {mode} n={runs} levels={:.1} call_us={:.0}/{call_max} timer_late_us={} timer_calls={}",
        levels_sum as f64 / n,
        call_sum as f64 / n,
        LATE_MAX.load(Ordering::Relaxed),
        LATE_CALLS.load(Ordering::Relaxed),
    );
    true
}

/// `pulsebench <pin> <n>`: the polled loop (interrupts off) against the RMT receiver on the 5 kHz
/// test signal, `n` measurements of up to 80 levels each, while a 100 µs `esp_timer` records how
/// late it runs (the polled loop blocks it; the RMT must not).
pub(super) fn bench(pin: i32, runs: u32) {
    if let Some(why) = pin_problem(pin) {
        println!("err pin {pin}: {why}");
        return;
    }
    if runs == 0 || runs > BENCH_MAX_RUNS {
        println!("err n must be 1..{BENCH_MAX_RUNS}");
        return;
    }
    claim_pin(pin);
    if !prepare(pin) {
        // SAFETY: the pin was claimed above.
        unsafe { sys::gpio_reset_pin(pin) };
        println!("err no RMT channel");
        return;
    }
    let mut stop_timer: sys::esp_timer_handle_t = core::ptr::null_mut();
    let mut late_timer: sys::esp_timer_handle_t = core::ptr::null_mut();
    let stop_args = sys::esp_timer_create_args_t {
        callback: Some(stop_pwm),
        dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_TASK,
        name: c"pulsebench-stop".as_ptr(),
        ..Default::default()
    };
    let late_args = sys::esp_timer_create_args_t {
        callback: Some(late_tick),
        dispatch_method: sys::esp_timer_dispatch_t_ESP_TIMER_TASK,
        name: c"pulsebench-late".as_ptr(),
        ..Default::default()
    };
    // SAFETY: the args live through the calls; both handles are deleted below; the pin is claimed
    // and its RMT channel exists, so LEDC takes the pin as in the self-test.
    let timers_ok = unsafe {
        start_pwm(pin);
        sys::esp_timer_create(&stop_args, &mut stop_timer) == 0
            && sys::esp_timer_create(&late_args, &mut late_timer) == 0
            && sys::esp_timer_start_periodic(late_timer, BENCH_PERIOD_US) == 0
    };
    if timers_ok {
        let mut arm = || {
            // SAFETY: the timer was created above and lives until the cleanup.
            unsafe { sys::esp_timer_start_once(stop_timer, BENCH_STOP_US) };
        };
        let polled_ok = bench_mode("polled", runs, stop_timer, &mut || {
            arm();
            Some(super::bus::pulse_measure_polled(pin, BENCH_LEVELS))
        });
        if polled_ok
            && bench_mode("rmt", runs, stop_timer, &mut || {
                measure(pin, BENCH_LEVELS, &mut arm)
            })
        {
            println!("ok");
        }
    } else {
        println!("err timers");
    }
    // SAFETY: stop (idempotent), delete what was made above, and give the pin back.
    unsafe {
        for timer in [late_timer, stop_timer] {
            if !timer.is_null() {
                sys::esp_timer_stop(timer);
                sys::esp_timer_delete(timer);
            }
        }
        sys::ledc_stop(TEST_MODE, TEST_CHANNEL, 0);
    }
    release(pin);
    // SAFETY: as above; the driver already reset the pin when the channel went.
    unsafe { sys::gpio_reset_pin(pin) };
}
