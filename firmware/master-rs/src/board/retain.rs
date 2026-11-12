//! Values that survive a restart (`retain`, `retain_text`), in the NVS namespace `crab_mod` under
//! keys `k<key as 8 hex digits>`, the same as the C++ master: values it stored are read back.

use super::devices::uptime_ms;
use crab_kernel::host::Mem;
use crab_master_rs::write_window::WriteWindow;
use esp_idf_svc::sys;
use std::ffi::CString;
use std::sync::Mutex;

/// Longest retained text, bytes; it is stored with one marker byte, so an empty text is a value too.
const MAX_TEXT: usize = 128;

static WRITES: Mutex<WriteWindow> = Mutex::new(WriteWindow::new());
static NVS: Mutex<Option<sys::nvs_handle_t>> = Mutex::new(None);

/// The open namespace; opened at the first use (the flash partition may already be up for Wi-Fi).
fn handle() -> Option<sys::nvs_handle_t> {
    let mut nvs = NVS.lock().unwrap_or_else(|e| e.into_inner());
    if nvs.is_none() {
        // SAFETY: plain calls; a second init of a ready partition is harmless.
        unsafe {
            let err = sys::nvs_flash_init();
            if err == sys::ESP_ERR_NVS_NO_FREE_PAGES || err == sys::ESP_ERR_NVS_NEW_VERSION_FOUND {
                sys::nvs_flash_erase();
                sys::nvs_flash_init();
            }
            let mut h: sys::nvs_handle_t = 0;
            if sys::nvs_open(
                c"crab_mod".as_ptr(),
                sys::nvs_open_mode_t_NVS_READWRITE,
                &mut h,
            ) == 0
            {
                *nvs = Some(h);
            }
        }
    }
    *nvs
}

fn count_write() {
    WRITES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(uptime_ms());
}

/// `(writes since boot, writes in the last hour)` of actual NVS writes.
pub fn write_counts() -> (u32, u32) {
    let mut w = WRITES.lock().unwrap_or_else(|e| e.into_inner());
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

fn key(key: i32) -> CString {
    CString::new(format!("k{:08x}", key as u32)).expect("no zero bytes")
}

/// `retain(op, key, value)`: op 0 reads (`value` is the default), op 1 writes a changed value.
pub fn retain(op: i32, k: i32, value: i32) -> i32 {
    let Some(h) = handle() else {
        return if op == 0 { value } else { 0 };
    };
    let name = key(k);
    let mut stored = 0u32;
    // SAFETY: a valid handle and key; the out-parameter lives through the call.
    let found = unsafe { sys::nvs_get_u32(h, name.as_ptr(), &mut stored) } == 0;
    if op == 0 {
        return if found { stored as i32 } else { value };
    }
    if !found || stored as i32 != value {
        // SAFETY: as above.
        unsafe {
            sys::nvs_set_u32(h, name.as_ptr(), value as u32);
            count_write();
            sys::nvs_commit(h);
        }
    }
    1
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
    let name = key(k);
    let mut stored = [0u8; MAX_TEXT + 1];
    let mut len = stored.len();
    // SAFETY: the buffer and its length live through the call.
    let found =
        unsafe { sys::nvs_get_blob(h, name.as_ptr(), stored.as_mut_ptr().cast(), &mut len) } == 0
            && len >= 1;
    if op == 0 {
        if !found {
            return -1;
        }
        let take = (len - 1).min(n as usize);
        return if mem.write(ptr, &stored[..take]) {
            take as i32
        } else {
            -1
        };
    }
    let Some(mut now) = mem.read(ptr, n.min(MAX_TEXT as i32), MAX_TEXT) else {
        return -1;
    };
    now.push(1);
    if found && stored[..len] == now[..] {
        return 1;
    }
    // SAFETY: a valid handle and key; the data lives through the call.
    unsafe {
        sys::nvs_set_blob(h, name.as_ptr(), now.as_ptr().cast(), now.len());
        count_write();
        sys::nvs_commit(h);
    }
    1
}
