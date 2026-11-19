//! `crab.restart` (`specs/restart`): a module asks for a restart of the board; the scan carries it
//! out between two steps, after the outputs of every module went to their safe state.

use core::sync::atomic::{AtomicU32, Ordering};

use crab_kernel::abi::restart_op;
use crab_kernel::host::Mem;
use esp_idf_svc::sys;

/// The due time of the request, ms of the boot clock, wrapping; 0 means no request.
static DUE_MS: AtomicU32 = AtomicU32::new(0);

fn now_ms() -> u32 {
    (super::tick::now_us() / 1000) as u32
}

/// `crab.restart(op, a, b)`.
pub fn call(op: i32, a: i32, b: i32, mem: &mut Mem<'_>) -> i32 {
    match op {
        restart_op::REQUEST => {
            let delay = a.clamp(0, restart_op::MAX_DELAY_MS) as u32;
            // 0 is "none": a due time that lands on it moves one millisecond on.
            let due = now_ms().wrapping_add(delay).max(1);
            let old = DUE_MS.load(Ordering::Relaxed);
            // The sooner of the two stays; a difference below zero is "old is earlier".
            if old == 0 || (due.wrapping_sub(old) as i32) < 0 {
                DUE_MS.store(due, Ordering::Relaxed);
            }
            1
        }
        restart_op::CANCEL => i32::from(DUE_MS.swap(0, Ordering::Relaxed) != 0),
        restart_op::REASON => {
            let text = reason();
            let n = text.len().min(b.max(0) as usize);
            if mem.write(a, &text.as_bytes()[..n]) {
                n as i32
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// True when a request is due.
pub fn due() -> bool {
    let due = DUE_MS.load(Ordering::Relaxed);
    due != 0 && (now_ms().wrapping_sub(due) as i32) >= 0
}

/// Why the board last started.
pub fn reason() -> &'static str {
    // SAFETY: a plain query of ESP-IDF.
    match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => "poweron",
        sys::esp_reset_reason_t_ESP_RST_EXT => "pin",
        sys::esp_reset_reason_t_ESP_RST_SW => "software",
        sys::esp_reset_reason_t_ESP_RST_PANIC => "panic",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "int_wdt",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "task_wdt",
        sys::esp_reset_reason_t_ESP_RST_WDT => "wdt",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "brownout",
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "deepsleep",
        _ => "other",
    }
}

/// Restarts the board; does not return.
pub fn now() -> ! {
    // SAFETY: no arguments; ESP-IDF runs its shutdown handlers and resets the chip.
    unsafe { sys::esp_restart() }
}
