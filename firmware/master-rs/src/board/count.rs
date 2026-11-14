//! Pulse counters of driver blocks (`bus.count_*`, `specs/pcnt`): fast counter, frequency, encoder.
//!
//! - Classic ESP32: one PCNT unit per counter. The unit counts every edge in hardware (16 bit, with
//!   the input glitch filter); the ESP-IDF driver adds each overflow to a 32-bit sum in its
//!   interrupt (`accum_count`). Quadrature always counts x4 in hardware; x2 and x1 are the x4 count
//!   divided, so the three modes behave the same on every chip.
//! - ESP32-C3 (no PCNT): a GPIO interrupt per edge updates an atomic count (quadrature: transition
//!   table on both pins). Slower; the schema check warns about it.
//!
//! Edge times for `count_period` (frequency below ~1 kHz): a GPIO interrupt on pin A adds the time
//! since the previous edge to cumulative sums. The interrupt writes them under a sequence number
//! (seqlock): the step reads the pair without a lock and without turning interrupts off, retrying
//! only while an edge is being written. On the ESP32 the interrupt turns itself off when edges
//! come faster than [`FAST_US`] (the count then gives the frequency) and the step turns it back on
//! below [`SLOW_HZ`].
//!
//! Counters are created in module `init` (allocation happens there, not in the step) and freed
//! with the module ([`release`]). All handlers live in IRAM and call only IRAM functions
//! (`esp_timer_get_time`, GPIO with `CONFIG_GPIO_CTRL_FUNC_IN_IRAM`), so a flash write does not
//! stop the counting.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};

use crab_kernel::abi::bus_op::{
    COUNTERS, COUNT_FALLING, COUNT_QUAD_X1, COUNT_QUAD_X2, COUNT_RISING, ERR_ARGS, ERR_BUSY,
};
use esp_idf_svc::sys;

const N: usize = COUNTERS as usize;
/// An interval shorter than this turns the edge timing off (ESP32), µs.
#[cfg(not(target_arch = "riscv32"))]
const FAST_US: u32 = 1000;
/// Below this rate the step turns the edge timing back on, Hz.
#[cfg(not(target_arch = "riscv32"))]
const SLOW_HZ: u64 = 500;
/// The seqlock read gives up after this many tries (an edge every few µs on the C3).
const READ_TRIES: u32 = 64;

/// `COUNT_KIND`: 1 PCNT, 2 GPIO interrupts.
#[cfg(not(target_arch = "riscv32"))]
pub const KIND: i32 = 1;
#[cfg(target_arch = "riscv32")]
pub const KIND: i32 = 2;

/// Quadrature: count change for (previous A/B state << 2 | new state), state = A << 1 | B. A
/// leading B counts up, the same direction as the PCNT channels below.
#[cfg(target_arch = "riscv32")]
const QUAD: [i8; 16] = [0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0];

struct Slot {
    used: AtomicBool,
    pin_a: AtomicI32,
    pin_b: AtomicI32,
    /// `COUNT_*` mode.
    mode: AtomicI32,
    /// Edge timing asked for at open.
    period: AtomicBool,
    /// Hardware unit and channels (ESP32).
    #[cfg_attr(target_arch = "riscv32", allow(dead_code))]
    unit: AtomicPtr<c_void>,
    #[cfg_attr(target_arch = "riscv32", allow(dead_code))]
    chans: [AtomicPtr<c_void>; 2],
    /// Software count (C3): edges or quadrature quarter steps.
    soft: AtomicI32,
    /// Last A/B state (C3 quadrature).
    #[cfg_attr(not(target_arch = "riscv32"), allow(dead_code))]
    quad: AtomicU32,
    /// Filter and the time of the last accepted edge (C3), µs.
    filter_us: AtomicU32,
    #[cfg_attr(not(target_arch = "riscv32"), allow(dead_code))]
    last_edge_us: AtomicU32,
    // Edge timing, written by the interrupt under `seq` (odd while writing).
    seq: AtomicU32,
    timed: AtomicU32,
    sum_us: AtomicU32,
    fast: AtomicBool,
    /// No edge time yet (after open or after the timing was turned back on).
    fresh: AtomicBool,
    last_us: AtomicU32,
    // What `period` returned last time; only the owning module's step touches them.
    seen_timed: AtomicU32,
    seen_sum: AtomicU32,
    seen_count: AtomicI32,
    seen_at_us: AtomicU32,
}

impl Slot {
    const fn new() -> Self {
        Self {
            used: AtomicBool::new(false),
            pin_a: AtomicI32::new(-1),
            pin_b: AtomicI32::new(-1),
            mode: AtomicI32::new(0),
            period: AtomicBool::new(false),
            unit: AtomicPtr::new(core::ptr::null_mut()),
            chans: [
                AtomicPtr::new(core::ptr::null_mut()),
                AtomicPtr::new(core::ptr::null_mut()),
            ],
            soft: AtomicI32::new(0),
            quad: AtomicU32::new(0),
            filter_us: AtomicU32::new(0),
            last_edge_us: AtomicU32::new(0),
            seq: AtomicU32::new(0),
            timed: AtomicU32::new(0),
            sum_us: AtomicU32::new(0),
            fast: AtomicBool::new(false),
            fresh: AtomicBool::new(true),
            last_us: AtomicU32::new(0),
            seen_timed: AtomicU32::new(0),
            seen_sum: AtomicU32::new(0),
            seen_count: AtomicI32::new(0),
            seen_at_us: AtomicU32::new(0),
        }
    }
}

static SLOTS: [Slot; N] = [const { Slot::new() }; N];
static ISR_SERVICE: AtomicBool = AtomicBool::new(false);

fn now_us() -> u32 {
    // SAFETY: plain query, in IRAM.
    unsafe { sys::esp_timer_get_time() as u32 }
}

/// Steps of the x4 count per step of the mode.
fn divisor(mode: i32) -> i32 {
    match mode {
        COUNT_QUAD_X1 => 4,
        COUNT_QUAD_X2 => 2,
        _ => 1,
    }
}

fn isr_service() -> bool {
    if ISR_SERVICE.load(Ordering::Acquire) {
        return true;
    }
    // SAFETY: installed once; `ESP_ERR_INVALID_STATE` means it already was.
    let err = unsafe { sys::gpio_install_isr_service(sys::ESP_INTR_FLAG_IRAM as i32) };
    let ok = err == sys::ESP_OK || err == sys::ESP_ERR_INVALID_STATE;
    ISR_SERVICE.store(ok, Ordering::Release);
    ok
}

/// Adds the time since the previous edge to the sums (both chips).
#[link_section = ".iram1.crab_count_time"]
fn time_edge(s: &Slot, now: u32, pin: i32) {
    let last = s.last_us.swap(now, Ordering::Relaxed);
    if s.fresh.swap(false, Ordering::Relaxed) {
        return;
    }
    let dt = now.wrapping_sub(last);
    s.seq.fetch_add(1, Ordering::Acquire);
    #[cfg(not(target_arch = "riscv32"))]
    if dt < FAST_US {
        s.fast.store(true, Ordering::Relaxed);
        // SAFETY: an IRAM GPIO call (`CONFIG_GPIO_CTRL_FUNC_IN_IRAM`).
        unsafe { sys::gpio_intr_disable(pin) };
        s.seq.fetch_add(1, Ordering::Release);
        return;
    }
    let _ = pin;
    s.timed.fetch_add(1, Ordering::Relaxed);
    s.sum_us.fetch_add(dt, Ordering::Relaxed);
    s.seq.fetch_add(1, Ordering::Release);
}

/// GPIO interrupt of pin A (and B for the C3 quadrature); `arg` is the slot number.
#[link_section = ".iram1.crab_count_edge"]
unsafe extern "C" fn on_edge(arg: *mut c_void) {
    let Some(s) = SLOTS.get(arg as usize) else {
        return;
    };
    let now = now_us();
    let pin_a = s.pin_a.load(Ordering::Relaxed);
    #[cfg(target_arch = "riscv32")]
    {
        let filter = s.filter_us.load(Ordering::Relaxed);
        if filter > 0 {
            let last = s.last_edge_us.load(Ordering::Relaxed);
            if now.wrapping_sub(last) < filter {
                return;
            }
            s.last_edge_us.store(now, Ordering::Relaxed);
        }
        let pin_b = s.pin_b.load(Ordering::Relaxed);
        let mode = s.mode.load(Ordering::Relaxed);
        // SAFETY: IRAM GPIO reads of pins this counter set up.
        let level = |pin: i32| unsafe { sys::gpio_get_level(pin) } as u32 & 1;
        if mode >= COUNT_QUAD_X1 {
            let state = level(pin_a) << 1 | level(pin_b);
            let old = s.quad.swap(state, Ordering::Relaxed);
            let step = QUAD[((old << 2 | state) & 15) as usize];
            s.soft.fetch_add(i32::from(step), Ordering::Relaxed);
            return;
        }
        let up = pin_b < 0 || level(pin_b) != 0;
        s.soft.fetch_add(if up { 1 } else { -1 }, Ordering::Relaxed);
    }
    if s.period.load(Ordering::Relaxed) {
        time_edge(s, now, pin_a);
    }
}

/// Opens a counter; its number or a `crab.bus` error. Pins and mode are checked by `bus_cfg`.
pub fn open(pin_a: i32, pin_b: i32, mode: i32, period: bool, filter_us: u32) -> i32 {
    let Some(n) = SLOTS.iter().position(|s| {
        s.used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }) else {
        return ERR_BUSY;
    };
    let s = &SLOTS[n];
    s.pin_a.store(pin_a, Ordering::Relaxed);
    s.pin_b.store(pin_b, Ordering::Relaxed);
    s.mode.store(mode, Ordering::Relaxed);
    s.period.store(period, Ordering::Relaxed);
    s.soft.store(0, Ordering::Relaxed);
    s.filter_us.store(filter_us, Ordering::Relaxed);
    s.timed.store(0, Ordering::Relaxed);
    s.sum_us.store(0, Ordering::Relaxed);
    s.fast.store(false, Ordering::Relaxed);
    s.fresh.store(true, Ordering::Relaxed);
    s.seen_timed.store(0, Ordering::Relaxed);
    s.seen_sum.store(0, Ordering::Relaxed);
    s.seen_count.store(0, Ordering::Relaxed);
    s.seen_at_us.store(now_us(), Ordering::Relaxed);
    let err = setup(n, s, pin_a, pin_b, mode, period, filter_us);
    if err != 0 {
        teardown(n, s);
        s.used.store(false, Ordering::Release);
        return err;
    }
    n as i32
}

/// GPIO input with the pull-up (open-collector sensors), as the PCNT driver sets its pins.
#[cfg(target_arch = "riscv32")]
fn input_pullup(pin: i32) -> bool {
    let cfg = sys::gpio_config_t {
        pin_bit_mask: 1u64 << pin,
        mode: sys::gpio_mode_t_GPIO_MODE_INPUT,
        pull_up_en: sys::gpio_pullup_t_GPIO_PULLUP_ENABLE,
        pull_down_en: sys::gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
        intr_type: sys::gpio_int_type_t_GPIO_INTR_DISABLE,
    };
    // SAFETY: the config lives through the call; the pin was checked and claimed.
    unsafe { sys::gpio_config(&cfg) == sys::ESP_OK }
}

/// GPIO interrupt on `pin` of type `kind` calling [`on_edge`] for slot `n`.
fn hook(n: usize, pin: i32, kind: sys::gpio_int_type_t) -> bool {
    if !isr_service() {
        return false;
    }
    // SAFETY: the pin is set up as an input; the handler and its argument stay valid until
    // `gpio_isr_handler_remove` in `teardown`.
    unsafe {
        sys::gpio_set_intr_type(pin, kind) == sys::ESP_OK
            && sys::gpio_isr_handler_add(pin, Some(on_edge), n as *mut c_void) == sys::ESP_OK
            && sys::gpio_intr_enable(pin) == sys::ESP_OK
    }
}

fn edge_kind(mode: i32) -> sys::gpio_int_type_t {
    match mode {
        COUNT_FALLING => sys::gpio_int_type_t_GPIO_INTR_NEGEDGE,
        COUNT_RISING => sys::gpio_int_type_t_GPIO_INTR_POSEDGE,
        _ => sys::gpio_int_type_t_GPIO_INTR_ANYEDGE,
    }
}

#[cfg(target_arch = "riscv32")]
fn setup(
    n: usize,
    s: &Slot,
    pin_a: i32,
    pin_b: i32,
    mode: i32,
    _period: bool,
    _filter_us: u32,
) -> i32 {
    if !input_pullup(pin_a) || (pin_b >= 0 && !input_pullup(pin_b)) {
        return ERR_ARGS;
    }
    if mode >= COUNT_QUAD_X1 {
        // SAFETY: inputs set up above.
        let state = unsafe {
            (sys::gpio_get_level(pin_a) as u32 & 1) << 1 | sys::gpio_get_level(pin_b) as u32 & 1
        };
        s.quad.store(state, Ordering::Relaxed);
        let any = sys::gpio_int_type_t_GPIO_INTR_ANYEDGE;
        if !hook(n, pin_a, any) || !hook(n, pin_b, any) {
            return ERR_ARGS;
        }
        return 0;
    }
    if !hook(n, pin_a, edge_kind(mode)) {
        return ERR_ARGS;
    }
    0
}

#[cfg(not(target_arch = "riscv32"))]
fn setup(
    n: usize,
    s: &Slot,
    pin_a: i32,
    pin_b: i32,
    mode: i32,
    period: bool,
    filter_us: u32,
) -> i32 {
    use sys::pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_DECREASE as DEC;
    use sys::pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_HOLD as HOLD;
    use sys::pcnt_channel_edge_action_t_PCNT_CHANNEL_EDGE_ACTION_INCREASE as INC;
    use sys::pcnt_channel_level_action_t_PCNT_CHANNEL_LEVEL_ACTION_INVERSE as INVERSE;
    use sys::pcnt_channel_level_action_t_PCNT_CHANNEL_LEVEL_ACTION_KEEP as KEEP;

    let mut cfg = sys::pcnt_unit_config_t {
        low_limit: i16::MIN as i32 + 1,
        high_limit: i16::MAX as i32,
        ..Default::default()
    };
    cfg.flags.set_accum_count(1);
    let mut unit: sys::pcnt_unit_handle_t = core::ptr::null_mut();
    // SAFETY: the config lives through the call.
    if unsafe { sys::pcnt_new_unit(&cfg, &mut unit) } != sys::ESP_OK {
        return ERR_BUSY;
    }
    s.unit.store(unit.cast(), Ordering::Release);
    if filter_us > 0 {
        let glitch = sys::pcnt_glitch_filter_config_t {
            max_glitch_ns: (filter_us * 1000).min(12_700),
        };
        // SAFETY: a new unit, not enabled yet.
        unsafe { sys::pcnt_unit_set_glitch_filter(unit, &glitch) };
    }
    let quad = mode >= COUNT_QUAD_X1;
    // (edge pin, level pin, rising, falling, level high, level low)
    let mut plan = [(pin_a, pin_b, INC, INC, KEEP, KEEP); 2];
    let chans = if quad {
        plan[0] = (pin_a, pin_b, DEC, INC, KEEP, INVERSE);
        plan[1] = (pin_b, pin_a, INC, DEC, KEEP, INVERSE);
        2
    } else {
        let (rise, fall) = match mode {
            COUNT_RISING => (INC, HOLD),
            COUNT_FALLING => (HOLD, INC),
            _ => (INC, INC),
        };
        // Direction pin: high keeps the action, low turns it around.
        let low = if pin_b >= 0 { INVERSE } else { KEEP };
        plan[0] = (pin_a, pin_b, rise, fall, KEEP, low);
        1
    };
    for (i, &(edge, level, rise, fall, high, low)) in plan.iter().take(chans).enumerate() {
        let ch_cfg = sys::pcnt_chan_config_t {
            edge_gpio_num: edge,
            level_gpio_num: level,
            ..Default::default()
        };
        let mut chan: sys::pcnt_channel_handle_t = core::ptr::null_mut();
        // SAFETY: the unit is valid; the config lives through the call.
        if unsafe { sys::pcnt_new_channel(unit, &ch_cfg, &mut chan) } != sys::ESP_OK {
            return ERR_BUSY;
        }
        s.chans[i].store(chan.cast(), Ordering::Release);
        // SAFETY: a new channel of a unit that is not enabled yet.
        unsafe {
            sys::pcnt_channel_set_edge_action(chan, rise, fall);
            sys::pcnt_channel_set_level_action(chan, high, low);
        }
    }
    // Watch points at the limits: the driver's interrupt adds each overflow to the 32-bit sum.
    // SAFETY: the unit is valid and not enabled yet.
    let ok = unsafe {
        sys::pcnt_unit_add_watch_point(unit, cfg.high_limit) == sys::ESP_OK
            && sys::pcnt_unit_add_watch_point(unit, cfg.low_limit) == sys::ESP_OK
            && sys::pcnt_unit_enable(unit) == sys::ESP_OK
            && sys::pcnt_unit_clear_count(unit) == sys::ESP_OK
            && sys::pcnt_unit_start(unit) == sys::ESP_OK
    };
    if !ok {
        return ERR_BUSY;
    }
    if period && !hook(n, pin_a, edge_kind(mode)) {
        return ERR_ARGS;
    }
    0
}

/// Undoes what `setup` made, whatever it got to.
fn teardown(n: usize, s: &Slot) {
    let pin_a = s.pin_a.load(Ordering::Relaxed);
    let pin_b = s.pin_b.load(Ordering::Relaxed);
    let _ = n;
    for pin in [pin_a, pin_b] {
        if pin >= 0 && ISR_SERVICE.load(Ordering::Acquire) {
            // SAFETY: removing a handler that may not be there is harmless.
            unsafe {
                sys::gpio_intr_disable(pin);
                sys::gpio_isr_handler_remove(pin);
            }
        }
    }
    #[cfg(not(target_arch = "riscv32"))]
    free_unit(s);
    for pin in [pin_a, pin_b] {
        if pin >= 0 {
            // SAFETY: a pin this counter set up.
            unsafe { sys::gpio_reset_pin(pin) };
        }
    }
}

/// Stops and deletes the PCNT unit and channels of a slot (ESP32).
#[cfg(not(target_arch = "riscv32"))]
fn free_unit(s: &Slot) {
    let unit: sys::pcnt_unit_handle_t = s.unit.swap(core::ptr::null_mut(), Ordering::AcqRel).cast();
    if !unit.is_null() {
        // SAFETY: the unit was made by `setup`; stop and disable fail harmlessly if it never ran.
        unsafe {
            sys::pcnt_unit_stop(unit);
            sys::pcnt_unit_disable(unit);
        }
    }
    for chan in &s.chans {
        let chan: sys::pcnt_channel_handle_t =
            chan.swap(core::ptr::null_mut(), Ordering::AcqRel).cast();
        if !chan.is_null() {
            // SAFETY: a channel of the unit, the unit is disabled.
            unsafe { sys::pcnt_del_channel(chan) };
        }
    }
    if !unit.is_null() {
        // SAFETY: no channels left, disabled.
        unsafe { sys::pcnt_del_unit(unit) };
    }
}

/// Frees a counter of a module that goes away.
pub fn release(n: usize) {
    let Some(s) = SLOTS.get(n) else { return };
    if s.used.load(Ordering::Acquire) {
        teardown(n, s);
        s.used.store(false, Ordering::Release);
    }
}

/// The count of counter `n` (opened by the calling module: `bus_cfg` checked it).
pub fn read(n: usize) -> i32 {
    let Some(s) = SLOTS.get(n) else { return 0 };
    let mode = s.mode.load(Ordering::Relaxed);
    let raw = hardware_count(s);
    raw.div_euclid(divisor(mode))
}

#[cfg(target_arch = "riscv32")]
fn hardware_count(s: &Slot) -> i32 {
    s.soft.load(Ordering::Relaxed)
}

#[cfg(not(target_arch = "riscv32"))]
fn hardware_count(s: &Slot) -> i32 {
    let unit: sys::pcnt_unit_handle_t = s.unit.load(Ordering::Acquire).cast();
    let mut value = 0;
    if !unit.is_null() {
        // SAFETY: a running unit; the driver reads the register and its sum in a short critical
        // section.
        unsafe { sys::pcnt_unit_get_count(unit, &mut value) };
    }
    value
}

/// Sets the count of counter `n` to 0.
pub fn clear(n: usize) {
    let Some(s) = SLOTS.get(n) else { return };
    #[cfg(target_arch = "riscv32")]
    s.soft.store(0, Ordering::Relaxed);
    #[cfg(not(target_arch = "riscv32"))]
    {
        let unit: sys::pcnt_unit_handle_t = s.unit.load(Ordering::Acquire).cast();
        if !unit.is_null() {
            // SAFETY: a running unit; clears the register and the sum.
            unsafe { sys::pcnt_unit_clear_count(unit) };
        }
    }
    s.seen_count.store(0, Ordering::Relaxed);
}

/// `COUNT_PERIOD`: (timed edges since the last call, sum of their intervals in µs).
pub fn period(n: usize) -> (i32, i32) {
    let Some(s) = SLOTS.get(n) else { return (0, 0) };
    if !s.period.load(Ordering::Relaxed) {
        return (0, 0);
    }
    let mut snap = None;
    for _ in 0..READ_TRIES {
        let before = s.seq.load(Ordering::Acquire);
        if before & 1 != 0 {
            continue;
        }
        let timed = s.timed.load(Ordering::Relaxed);
        let sum = s.sum_us.load(Ordering::Relaxed);
        let fast = s.fast.load(Ordering::Relaxed);
        if s.seq.load(Ordering::Acquire) == before {
            snap = Some((timed, sum, fast));
            break;
        }
    }
    let Some((timed, sum, fast)) = snap else {
        return (0, 0);
    };
    let seen_timed = s.seen_timed.swap(timed, Ordering::Relaxed);
    let seen_sum = s.seen_sum.swap(sum, Ordering::Relaxed);
    #[cfg(not(target_arch = "riscv32"))]
    if slow_since_last(s) && fast {
        // Slow again: time the edges, starting from the next one.
        s.fresh.store(true, Ordering::Relaxed);
        s.fast.store(false, Ordering::Relaxed);
        // SAFETY: the pin of this counter has its handler.
        unsafe { sys::gpio_intr_enable(s.pin_a.load(Ordering::Relaxed)) };
    }
    if fast {
        return (0, 0);
    }
    let edges = timed.wrapping_sub(seen_timed);
    let total = sum.wrapping_sub(seen_sum);
    if edges == 0 || edges > i32::MAX as u32 || total > i32::MAX as u32 {
        return (0, 0);
    }
    (edges as i32, total as i32)
}

/// Whether the count shows the edges came slower than [`SLOW_HZ`] since the last call.
#[cfg(not(target_arch = "riscv32"))]
fn slow_since_last(s: &Slot) -> bool {
    let now = now_us();
    let count = hardware_count(s);
    let elapsed = now.wrapping_sub(s.seen_at_us.swap(now, Ordering::Relaxed));
    let edges = count.wrapping_sub(s.seen_count.swap(count, Ordering::Relaxed));
    u64::from(edges.unsigned_abs()) * 1_000_000 < SLOW_HZ * u64::from(elapsed.max(1))
}

/// Sets the input enable bit (`FUN_IE`, bit 9) of the pin's IO_MUX register, leaving its output
/// signal alone (`gpio_set_direction` would route the output back to plain GPIO).
#[cfg(feature = "selftest")]
fn pad_input_on(pin: i32) -> bool {
    // SAFETY: a read of the ESP-IDF table of IO_MUX register addresses.
    let reg = unsafe {
        usize::try_from(pin)
            .ok()
            .and_then(|i| sys::GPIO_PIN_MUX_REG.get(i).copied())
    };
    let Some(reg) = reg.filter(|&r| r != 0) else {
        return false;
    };
    let reg = reg as *mut u32;
    // SAFETY: an IO_MUX register of a valid pin; read-modify-write of one bit.
    unsafe { reg.write_volatile(reg.read_volatile() | 1 << 9) };
    true
}

/// `selftest pwm <pin> <hz> [<pin2>]`: a square wave on `pin` (and on `pin2` a quarter period
/// later, for an encoder) with the pad input on, so a counter on the same pin counts it without a
/// wire. Test builds only.
#[cfg(feature = "selftest")]
pub fn selftest_pwm(pin: i32, hz: u32, pin2: Option<i32>) -> bool {
    const MODE: sys::ledc_mode_t = sys::ledc_mode_t_LEDC_LOW_SPEED_MODE;
    // The divider must fit: try the finest duty first, coarser ones for high frequencies.
    let Some(bits) = (2..=14u32).rev().find(|&bits| {
        let timer = sys::ledc_timer_config_t {
            speed_mode: MODE,
            duty_resolution: bits as sys::ledc_timer_bit_t,
            timer_num: sys::ledc_timer_t_LEDC_TIMER_1,
            freq_hz: hz.max(1),
            clk_cfg: sys::soc_periph_ledc_clk_src_legacy_t_LEDC_AUTO_CLK,
            ..Default::default()
        };
        // SAFETY: the config lives through the call.
        unsafe { sys::ledc_timer_config(&timer) == sys::ESP_OK }
    }) else {
        return false;
    };
    // Half duty; the second channel starts a quarter period later.
    let mut ok = true;
    for (i, (p, hpoint)) in [(Some(pin), 0), (pin2, 1i32 << (bits - 2))]
        .into_iter()
        .enumerate()
    {
        let Some(p) = p else { continue };
        let ch = sys::ledc_channel_config_t {
            gpio_num: p,
            speed_mode: MODE,
            channel: (4 + i) as sys::ledc_channel_t,
            timer_sel: sys::ledc_timer_t_LEDC_TIMER_1,
            duty: 1 << (bits - 1),
            hpoint,
            ..Default::default()
        };
        // SAFETY: the config lives through the call; the input path is turned back on after the
        // channel made the pin an output.
        ok &= unsafe { sys::ledc_channel_config(&ch) == sys::ESP_OK && pad_input_on(p) };
    }
    ok
}
