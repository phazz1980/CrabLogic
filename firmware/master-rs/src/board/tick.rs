//! The clock of the scan (`specs/fast-cycle`): steps start on the marks of a grid.
//!
//! The marks come from a hardware timer (GPTimer, 1 MHz, auto-reload) whose interrupt is
//! allocated on the core of the scan, so waking the scan never depends on the other core: the
//! FreeRTOS tick that wakes a delayed task is handled by core 0, and with Wi-Fi busy there the
//! wake came up to a tick late (bench 13.11.2026). The interrupt sets a notification bit of the
//! scan task; marks passed during a long step merge into that bit and are dropped by `skip`.
//! Without the timer (it could not be made) the grid falls back to whole FreeRTOS ticks.

use core::ffi::c_void;

use esp_idf_svc::hal::task;
use esp_idf_svc::sys;

/// Microseconds since boot.
pub fn now_us() -> u64 {
    // SAFETY: no arguments, no state.
    (unsafe { sys::esp_timer_get_time() }) as u64
}

/// Runs in the timer interrupt, on the core of the scan.
#[link_section = ".iram1.crab_tick"]
unsafe extern "C" fn on_mark(
    _timer: sys::gptimer_handle_t,
    _edata: *const sys::gptimer_alarm_event_data_t,
    task: *mut c_void,
) -> bool {
    super::watchdog::check();
    let mut woken: sys::BaseType_t = 0;
    // SAFETY: `task` is the scan task, which never ends while the timer runs.
    unsafe {
        sys::xTaskGenericNotifyFromISR(
            task.cast(),
            0,
            1,
            sys::eNotifyAction_eSetBits,
            core::ptr::null_mut(),
            &mut woken,
        )
    };
    woken != 0
}

/// Wakes the calling task on the marks of the grid.
pub struct Ticker {
    timer: sys::gptimer_handle_t,
    period_ms: u32,
    /// The fallback grid: the next mark in FreeRTOS ticks (wrapping).
    next: u32,
}

impl Ticker {
    /// A ticker for the calling task; the error says why the hardware timer is not used.
    pub fn new(period_ms: u32) -> (Self, Option<String>) {
        let mut t = Self {
            timer: core::ptr::null_mut(),
            period_ms,
            next: 0,
        };
        let error = t.start_timer().err();
        if t.timer.is_null() {
            t.next = ticks_now().wrapping_add(t.ticks());
        }
        (t, error)
    }

    fn start_timer(&mut self) -> Result<(), String> {
        let me = task::current().ok_or("таймер цикла: вызов из прерывания")?;
        let config = sys::gptimer_config_t {
            clk_src: sys::soc_periph_gptimer_clk_src_t_GPTIMER_CLK_SRC_DEFAULT,
            direction: sys::gptimer_count_direction_t_GPTIMER_COUNT_UP,
            resolution_hz: 1_000_000,
            intr_priority: 0,
            flags: Default::default(),
        };
        let mut timer: sys::gptimer_handle_t = core::ptr::null_mut();
        let check = |err: sys::esp_err_t, what: &str| {
            if err == sys::ESP_OK {
                Ok(())
            } else {
                Err(format!("таймер цикла: {what} (ошибка {err})"))
            }
        };
        // SAFETY: the config lives through the call; the handle is kept in `self` and deleted in
        // `drop`.
        check(
            unsafe { sys::gptimer_new_timer(&config, &mut timer) },
            "не создан",
        )?;
        self.timer = timer;
        let callbacks = sys::gptimer_event_callbacks_t {
            on_alarm: Some(on_mark),
        };
        // The interrupt is allocated here, on the core of the calling (scan) task.
        // SAFETY: the callbacks are copied by the driver; the task handle outlives the timer.
        check(
            unsafe { sys::gptimer_register_event_callbacks(timer, &callbacks, me.cast()) },
            "прерывание не подключено",
        )?;
        self.set_alarm()?;
        // SAFETY: the timer is set up.
        check(unsafe { sys::gptimer_enable(timer) }, "не включён")?;
        check(unsafe { sys::gptimer_start(timer) }, "не запущен")
    }

    fn set_alarm(&self) -> Result<(), String> {
        let mut alarm = sys::gptimer_alarm_config_t {
            alarm_count: self.period_ms as u64 * 1000,
            reload_count: 0,
            flags: Default::default(),
        };
        alarm.flags.set_auto_reload_on_alarm(1);
        // SAFETY: `timer` is a live timer; the config lives through the call.
        let err = unsafe { sys::gptimer_set_alarm_action(self.timer, &alarm) };
        if err == sys::ESP_OK {
            Ok(())
        } else {
            Err(format!("таймер цикла: период не задан (ошибка {err})"))
        }
    }

    fn ticks(&self) -> u32 {
        (self.period_ms * sys::configTICK_RATE_HZ / 1000).max(1)
    }

    /// A new period; the grid starts again from now.
    pub fn set_period(&mut self, period_ms: u32) {
        if period_ms == self.period_ms {
            return;
        }
        self.period_ms = period_ms;
        if self.timer.is_null() {
            self.next = ticks_now().wrapping_add(self.ticks());
            return;
        }
        // SAFETY: a live timer; the count restarts so the new period begins now.
        unsafe { sys::gptimer_set_raw_count(self.timer, 0) };
        let _ = self.set_alarm();
        self.skip();
    }

    /// Drops the marks that passed during a step that ran over.
    pub fn skip(&mut self) {
        if self.timer.is_null() {
            let late = ticks_now().wrapping_sub(self.next) as i32;
            if late >= 0 {
                let missed = late as u32 / self.ticks() + 1;
                self.next = self.next.wrapping_add(missed * self.ticks());
            }
        } else {
            task::wait_notification(0);
        }
    }

    /// Waits for the next mark; returns at once if it has already come (after the work between
    /// steps), so that step starts late but on its own mark.
    pub fn wait(&mut self) {
        if self.timer.is_null() {
            let ahead = self.next.wrapping_sub(ticks_now()) as i32;
            if ahead > 0 {
                // SAFETY: blocks the calling task only.
                unsafe { sys::vTaskDelay(ahead as u32) };
                self.next = self.next.wrapping_add(self.ticks());
            } else {
                self.skip();
            }
        } else {
            // Twice the period at most, should the timer ever stop.
            task::wait_notification((2 * self.ticks()).max(2));
        }
    }
}

fn ticks_now() -> u32 {
    // SAFETY: plain query of the scheduler.
    unsafe { sys::xTaskGetTickCount() }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        if self.timer.is_null() {
            return;
        }
        // SAFETY: the timer was made in `new` and is not used after this.
        unsafe {
            sys::gptimer_stop(self.timer);
            sys::gptimer_disable(self.timer);
            sys::gptimer_del_timer(self.timer);
        }
    }
}
