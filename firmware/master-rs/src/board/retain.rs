//! Values that survive a restart (`retain`, `retain_text`), in the NVS namespace `crab_mod` under
//! keys `k<key as 8 hex digits>`, the same as the C++ master: values it stored are read back.
//!
//! Reads (op 0) happen when a module starts and are synchronous. Writes (op 1) never touch the
//! flash in the step: the value goes into a bounded table of pending writes (`PENDING`) and the
//! thread `crab-nvs` does `nvs_set_*` and `nvs_commit`.
//!
//! - A newer value for a key that is still pending replaces the older one (coalescing), so a key
//!   that changes faster than the thread writes costs one write per round, not one per change.
//! - A read of a key that is pending returns the pending value, so a module restarted before the
//!   thread got to the write still reads what the old one saved. An entry leaves the table only
//!   after its commit (and only when nothing newer replaced it meanwhile).
//! - The table has `INTS` words and `TEXTS` texts. When it is full and the key is not in it, the
//!   write is not queued: it answers 0, the drop is counted and the thread reports the count to
//!   the journal. The module treats its value as saved all the same, so the flash keeps the older
//!   value until the variable changes again; at the write rate of the modules (a value is saved
//!   after it settled, at most once per period) a table of this size is not reached.
//! - The thread starts with the first successful open of the namespace (a module start). If it
//!   cannot start (no memory), writes fall back to the old synchronous way: data over speed.

use super::devices::uptime_ms;
use super::spin::Spin;
use crab_kernel::host::Mem;
use crab_master_rs::write_window::WriteWindow;
use esp_idf_svc::sys;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

/// Longest retained text, bytes; it is stored with one marker byte, so an empty text is a value too.
const MAX_TEXT: usize = 128;
/// Pending words and texts (distinct keys).
const INTS: usize = 32;
const TEXTS: usize = 4;
/// Stack of `crab-nvs`: `nvs_set_blob` and `nvs_commit` run on it.
const STACK: usize = 4096;
/// The longest sleep of the thread between notifications, FreeRTOS ticks: a lost notification only
/// delays a write.
const WAIT_TICKS: u32 = 100;

/// Count of the flash writes: the thread records, `info` reads (the scan or `crab-io`). A "ceiling"
/// lock (`board/spin.rs`): the sections are a few instructions.
static WRITES: Spin<WriteWindow> = Spin::new(WriteWindow::new());
/// The namespace is open (`HANDLE` is valid). Opened by the scan when a module reads or writes
/// first; the thread only uses it.
static OPEN: AtomicBool = AtomicBool::new(false);
static HANDLE: AtomicU32 = AtomicU32::new(0);
/// The thread `crab-nvs` runs.
static STARTED: AtomicBool = AtomicBool::new(false);
/// The FreeRTOS task of the thread (for the wake-up).
static TASK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());
/// Writes that found the table full since the thread last told the journal.
static DROPPED: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
struct IntEntry {
    used: bool,
    key: i32,
    value: u32,
    /// The write number of the value: the thread removes the entry only if it is still the one
    /// it wrote.
    gen: u32,
}

#[derive(Clone, Copy)]
struct TextEntry {
    used: bool,
    key: i32,
    /// Bytes in `data`: the text and the marker byte.
    len: u8,
    data: [u8; MAX_TEXT + 1],
    gen: u32,
}

const NO_INT: IntEntry = IntEntry {
    used: false,
    key: 0,
    value: 0,
    gen: 0,
};
const NO_TEXT: TextEntry = TextEntry {
    used: false,
    key: 0,
    len: 0,
    data: [0; MAX_TEXT + 1],
    gen: 0,
};

/// What the thread takes from the table (a copy; the entry stays until it is written).
enum Job {
    Int { key: i32, value: u32, gen: u32 },
    Text(TextEntry),
}

/// Writes waiting for the thread. A "ceiling" lock (`board/spin.rs`): the scan puts and looks up,
/// the thread copies one entry out and later marks it done; the flash is touched outside it.
struct Pending {
    ints: [IntEntry; INTS],
    texts: [TextEntry; TEXTS],
    gen: u32,
}

static PENDING: Spin<Pending> = Spin::new(Pending {
    ints: [NO_INT; INTS],
    texts: [NO_TEXT; TEXTS],
    gen: 0,
});

impl Pending {
    fn next_gen(&mut self) -> u32 {
        self.gen = self.gen.wrapping_add(1);
        self.gen
    }

    /// Queues a word, or replaces the pending value of its key; `false` when the table is full.
    fn put_int(&mut self, key: i32, value: u32) -> bool {
        let gen = self.next_gen();
        let slot = match self.ints.iter().position(|e| e.used && e.key == key) {
            Some(i) => i,
            None => match self.ints.iter().position(|e| !e.used) {
                Some(i) => i,
                None => return false,
            },
        };
        self.ints[slot] = IntEntry {
            used: true,
            key,
            value,
            gen,
        };
        true
    }

    /// Queues a text (with its marker byte), as `put_int`.
    fn put_text(&mut self, key: i32, text: &[u8]) -> bool {
        let gen = self.next_gen();
        let slot = match self.texts.iter().position(|e| e.used && e.key == key) {
            Some(i) => i,
            None => match self.texts.iter().position(|e| !e.used) {
                Some(i) => i,
                None => return false,
            },
        };
        let e = &mut self.texts[slot];
        e.used = true;
        e.key = key;
        e.gen = gen;
        e.data[..text.len()].copy_from_slice(text);
        e.data[text.len()] = 1;
        e.len = (text.len() + 1) as u8;
        true
    }

    fn int(&self, key: i32) -> Option<u32> {
        self.ints
            .iter()
            .find(|e| e.used && e.key == key)
            .map(|e| e.value)
    }

    fn next_job(&self) -> Option<Job> {
        if let Some(e) = self.ints.iter().find(|e| e.used) {
            return Some(Job::Int {
                key: e.key,
                value: e.value,
                gen: e.gen,
            });
        }
        self.texts.iter().find(|e| e.used).map(|e| Job::Text(*e))
    }

    /// The thread wrote the entry of `gen`; a newer value of the key stays for the next round.
    fn done_int(&mut self, key: i32, gen: u32) {
        if let Some(e) = self
            .ints
            .iter_mut()
            .find(|e| e.used && e.key == key && e.gen == gen)
        {
            e.used = false;
        }
    }

    fn done_text(&mut self, key: i32, gen: u32) {
        if let Some(e) = self
            .texts
            .iter_mut()
            .find(|e| e.used && e.key == key && e.gen == gen)
        {
            e.used = false;
        }
    }
}

/// The open namespace; opened at the first use (the flash partition may already be up for Wi-Fi).
/// Only the scan opens it (a module start), so there is no race; the thread starts after it.
fn handle() -> Option<sys::nvs_handle_t> {
    if OPEN.load(Ordering::Acquire) {
        return Some(HANDLE.load(Ordering::Relaxed));
    }
    let mut h: sys::nvs_handle_t = 0;
    // SAFETY: plain calls; a second init of a ready partition is harmless.
    let opened = unsafe {
        let err = sys::nvs_flash_init();
        if err == sys::ESP_ERR_NVS_NO_FREE_PAGES || err == sys::ESP_ERR_NVS_NEW_VERSION_FOUND {
            sys::nvs_flash_erase();
            sys::nvs_flash_init();
        }
        sys::nvs_open(
            c"crab_mod".as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut h,
        ) == 0
    };
    if !opened {
        return None;
    }
    HANDLE.store(h, Ordering::Relaxed);
    OPEN.store(true, Ordering::Release);
    if !STARTED.load(Ordering::Acquire) && super::spawn_background(b"crab-nvs\0", STACK, run) {
        STARTED.store(true, Ordering::Release);
    }
    Some(h)
}

fn count_write() {
    WRITES.lock_ceiling().record(uptime_ms());
}

/// `(writes since boot, writes in the last hour)` of actual NVS writes.
pub fn write_counts() -> (u32, u32) {
    let mut w = WRITES.lock_ceiling();
    let hour = w.last_hour(uptime_ms());
    (w.total(), hour)
}

/// `(used, free)` NVS entries of the default partition, when the statistics are available.
pub fn nvs_entries() -> Option<(usize, usize)> {
    // SAFETY: a null partition name means the default one; the out-parameter lives through the call.
    unsafe {
        let mut st: sys::nvs_stats_t = core::mem::zeroed();
        if sys::nvs_get_stats(core::ptr::null(), &mut st) == 0 {
            Some((st.used_entries as usize, st.free_entries as usize))
        } else {
            None
        }
    }
}

/// The NVS key `k<8 hex digits>` as a C string, on the stack.
fn key(key: i32) -> [u8; 10] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = *b"k00000000\0";
    let v = key as u32;
    for (i, c) in name[1..9].iter_mut().enumerate() {
        *c = HEX[(v >> (28 - 4 * i) & 0xF) as usize];
    }
    name
}

/// Wakes the thread: something is pending.
fn wake() {
    let task = TASK.load(Ordering::Acquire);
    if !task.is_null() {
        // SAFETY: the task of the thread never ends once started.
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
}

/// Writes a word when it differs from the stored one.
fn flush_int(h: sys::nvs_handle_t, k: i32, value: u32) {
    let name = key(k);
    let mut stored = 0u32;
    // SAFETY: a valid handle and key; the out-parameter lives through the call.
    let found = unsafe { sys::nvs_get_u32(h, name.as_ptr().cast(), &mut stored) } == 0;
    if !found || stored != value {
        // SAFETY: as above.
        unsafe {
            sys::nvs_set_u32(h, name.as_ptr().cast(), value);
            count_write();
            sys::nvs_commit(h);
        }
    }
}

/// Writes a text (with its marker byte) when it differs from the stored one.
fn flush_text(h: sys::nvs_handle_t, k: i32, now: &[u8]) {
    let name = key(k);
    let mut stored = [0u8; MAX_TEXT + 1];
    let mut len = stored.len();
    // SAFETY: the buffer and its length live through the call.
    let found = unsafe {
        sys::nvs_get_blob(
            h,
            name.as_ptr().cast(),
            stored.as_mut_ptr().cast(),
            &mut len,
        )
    } == 0
        && len >= 1;
    if found && stored[..len] == *now {
        return;
    }
    // SAFETY: a valid handle and key; the data lives through the call.
    unsafe {
        sys::nvs_set_blob(h, name.as_ptr().cast(), now.as_ptr().cast(), now.len());
        count_write();
        sys::nvs_commit(h);
    }
}

/// The thread `crab-nvs`: writes the pending values one by one, then sleeps until the scan
/// queues one.
fn run() {
    // SAFETY: the handle of the calling task.
    TASK.store(
        unsafe { sys::xTaskGetCurrentTaskHandle() }.cast(),
        Ordering::Release,
    );
    loop {
        let job = PENDING.lock_ceiling().next_job();
        // Opened before the thread starts; only a lost handle would end here.
        let Some(h) = handle() else {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        };
        match job {
            Some(Job::Int { key, value, gen }) => {
                flush_int(h, key, value);
                PENDING.lock_ceiling().done_int(key, gen);
            }
            Some(Job::Text(e)) => {
                flush_text(h, e.key, &e.data[..usize::from(e.len)]);
                PENDING.lock_ceiling().done_text(e.key, e.gen);
            }
            None => {
                let dropped = DROPPED.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    super::notes::push(
                        "retain",
                        format!("запись retain пропущена, очередь заполнена: {dropped}"),
                    );
                }
                // A put after `next_job` found nothing sets the bit, so this returns at once.
                // SAFETY: waits on the notification of the calling task.
                unsafe { sys::ulTaskGenericNotifyTake(0, 1, WAIT_TICKS) };
            }
        }
    }
}

/// `retain(op, key, value)`: op 0 reads (`value` is the default), op 1 writes a changed value.
pub fn retain(op: i32, k: i32, value: i32) -> i32 {
    if op == 0 {
        // A value that is still waiting for the thread is the newest one.
        if let Some(v) = PENDING.lock_scan().int(k) {
            return v as i32;
        }
        let Some(h) = handle() else {
            return value;
        };
        let name = key(k);
        let mut stored = 0u32;
        // SAFETY: a valid handle and key; the out-parameter lives through the call.
        let found = unsafe { sys::nvs_get_u32(h, name.as_ptr().cast(), &mut stored) } == 0;
        return if found { stored as i32 } else { value };
    }
    let Some(h) = handle() else {
        return 0;
    };
    if !STARTED.load(Ordering::Acquire) {
        // No thread (no memory for it): the write happens here, as it used to.
        flush_int(h, k, value as u32);
        return 1;
    }
    if PENDING.lock_scan().put_int(k, value as u32) {
        wake();
        1
    } else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        0
    }
}

/// `retain_text(op, key, ptr, n)`: op 0 reads into the memory and returns the length, or -1 when
/// nothing is stored; op 1 writes a changed text (up to 128 bytes).
pub fn retain_text(op: i32, k: i32, ptr: i32, n: i32, mem: &mut Mem<'_>) -> i32 {
    let Some(h) = handle() else {
        return -1;
    };
    if n < 0 {
        return -1;
    }
    if op == 0 {
        let mut stored = [0u8; MAX_TEXT + 1];
        // A text that is still waiting for the thread is the newest one.
        let pending = {
            let q = PENDING.lock_scan();
            q.texts.iter().find(|e| e.used && e.key == k).map(|e| {
                let len = usize::from(e.len);
                stored[..len].copy_from_slice(&e.data[..len]);
                len
            })
        };
        let len = match pending {
            Some(len) => len,
            None => {
                let name = key(k);
                let mut len = stored.len();
                // SAFETY: the buffer and its length live through the call.
                let found = unsafe {
                    sys::nvs_get_blob(
                        h,
                        name.as_ptr().cast(),
                        stored.as_mut_ptr().cast(),
                        &mut len,
                    )
                } == 0
                    && len >= 1;
                if !found {
                    return -1;
                }
                len
            }
        };
        let take = (len - 1).min(n as usize);
        return if mem.write(ptr, &stored[..take]) {
            take as i32
        } else {
            -1
        };
    }
    let Some(text) = mem.slice(ptr, (n as usize).min(MAX_TEXT)) else {
        return -1;
    };
    if !STARTED.load(Ordering::Acquire) {
        // No thread (no memory for it): the write happens here, as it used to.
        let mut now = [0u8; MAX_TEXT + 1];
        now[..text.len()].copy_from_slice(text);
        now[text.len()] = 1;
        flush_text(h, k, &now[..text.len() + 1]);
        return 1;
    }
    if PENDING.lock_scan().put_text(k, text) {
        wake();
        1
    } else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        0
    }
}
