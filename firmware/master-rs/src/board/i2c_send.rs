//! The thread `crab-i2c`: sends the queued I²C writes of modules (`I2C_SEND`) so the step does
//! not wait for the bus (`specs/fast-cycle`). The queue (`crab_master_rs::send_queue`) and this
//! thread appear with the first send; on the ESP32 the thread runs on core 0, away from the scan.
//! The step never waits here: a busy queue answers "not yet" (0) and the module tries again.

use std::sync::atomic::{AtomicPtr, Ordering};

use crab_kernel::abi::bus_op::ERR_BUS;
use crab_master_rs::send_queue::{SendQueue, MAX};
use esp_idf_svc::sys;

use super::spin::Spin;

const STACK: usize = 3072;
/// The longest sleep of the thread between notifications, FreeRTOS ticks (1 ms each): a lost
/// notification only delays a send.
const WAIT_TICKS: u32 = 100;

/// The queue; `None` until the first module sends. A "try" lock (`board/spin.rs`): the step only
/// tries it, the thread waits for it.
static QUEUE: Spin<Option<Box<SendQueue>>> = Spin::new(None);
/// The FreeRTOS task of the thread (for its stack figure in `list`).
static TASK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

/// A slot for a module: sets the queue and the thread up on the first call. `Err` is a
/// `crab.bus` code: no thread or every slot owned, or 0 ("not yet") while the thread holds the queue.
pub fn claim() -> Result<usize, i32> {
    let Some(mut q) = QUEUE.try_lock() else {
        return Err(0);
    };
    if q.is_none() {
        if !start() {
            return Err(ERR_BUS);
        }
        *q = Some(Box::default());
    }
    q.as_mut().and_then(|q| q.claim()).ok_or(ERR_BUS)
}

/// The module of `slot` went (called when its bus goes).
pub fn release(slot: usize) {
    if let Some(q) = QUEUE.lock_wait().as_mut() {
        q.release(slot);
    }
}

/// Queues a write; see `SendQueue::send`. Never waits: 0 while the thread holds the queue.
#[link_section = ".iram1.crab_i2c_send"]
pub fn send(slot: usize, addr: u8, data: &[u8]) -> i32 {
    let code = match QUEUE.try_lock() {
        Some(mut q) => match q.as_mut() {
            Some(q) => q.send(slot, addr, data),
            None => ERR_BUS,
        },
        None => return 0,
    };
    let task = TASK.load(Ordering::Relaxed);
    if code > 0 && !task.is_null() {
        // SAFETY: the task never ends once started.
        unsafe {
            sys::xTaskGenericNotify(
                task.cast(),
                0,
                1,
                sys::eNotifyAction_eSetBits,
                core::ptr::null_mut(),
            )
        };
    }
    code
}

/// Stack never used by the thread, bytes; `None` before it started.
pub fn stack_left() -> Option<u32> {
    let task = TASK.load(Ordering::Relaxed);
    // SAFETY: the task never ends once started; a null handle is not passed.
    (!task.is_null()).then(|| unsafe { sys::uxTaskGetStackHighWaterMark(task.cast()) })
}

fn start() -> bool {
    super::spawn_background(b"crab-i2c\0", STACK, run)
}

fn run() {
    // SAFETY: the handle of the calling task.
    TASK.store(
        unsafe { sys::xTaskGetCurrentTaskHandle() }.cast(),
        Ordering::Relaxed,
    );
    let mut buf = [0u8; MAX];
    loop {
        let job = QUEUE.lock_wait().as_mut().and_then(|q| q.take(&mut buf));
        let Some((slot, addr, len)) = job else {
            // A send after `take` found nothing sets the bit, so this returns at once.
            // SAFETY: waits on the notification of the calling task.
            unsafe { sys::ulTaskGenericNotifyTake(0, 1, WAIT_TICKS) };
            continue;
        };
        let err = super::bus::queued_i2c_write(addr, &buf[..len]);
        if let Some(q) = QUEUE.lock_wait().as_mut() {
            q.done(slot, err);
        }
    }
}
