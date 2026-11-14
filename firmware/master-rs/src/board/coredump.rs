//! The crash dump of the board (`specs/bug-report`). ESP-IDF writes it to the `coredump`
//! partition when the master panics or a watchdog fires (`CONFIG_ESP_COREDUMP_ENABLE_TO_FLASH`);
//! the scan never touches it. At start the partition is checked once and mapped into the address
//! space, so `core read` copies from memory through the cache and never stops the other core the
//! way a flash read would. Only the thread `crab-io` answers these commands.

use core::ffi::c_void;
use std::sync::Mutex;

use crab_master_rs::protocol::{format_core_chunk, CORE_CHUNK};
use crab_master_rs::storage::crc32;
use esp_idf_svc::sys;

/// Bytes erased by `core erase`: the first sector holds the header and the checksum is no longer
/// valid without it; ESP-IDF erases what it needs before the next dump.
const ERASE: usize = 4096;

struct Dump {
    part: usize,
    /// Mapped start of the image, length, CRC32 of the image.
    base: usize,
    len: usize,
    crc: u32,
}

/// Set once at start, cleared by `core erase`. Taken by `crab-io` only (never in the step).
static DUMP: Mutex<Option<Dump>> = Mutex::new(None);

/// Looks for a valid dump left by the previous run; called once before the scan starts.
pub fn init() {
    let mut addr = 0usize;
    let mut len = 0usize;
    // SAFETY: plain ESP-IDF queries; the image is validated (checksum) by `image_get`.
    let found = unsafe { sys::esp_core_dump_image_get(&mut addr, &mut len) } == sys::ESP_OK;
    if !found || len == 0 {
        return;
    }
    // SAFETY: the returned partition record lives as long as the firmware.
    let part = unsafe {
        sys::esp_partition_find_first(
            sys::esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
            sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_COREDUMP,
            core::ptr::null(),
        )
    };
    if part.is_null() {
        return;
    }
    // SAFETY: `part` is valid.
    let (start, size) = unsafe { ((*part).address as usize, (*part).size as usize) };
    let Some(offset) = addr.checked_sub(start) else {
        return;
    };
    if offset.checked_add(len).is_none_or(|end| end > size) {
        return;
    }
    let mut base: *const c_void = core::ptr::null();
    let mut handle: sys::esp_partition_mmap_handle_t = 0;
    // SAFETY: the range is inside the partition; the mapping is kept for the life of the firmware.
    let err = unsafe {
        sys::esp_partition_mmap(
            part,
            0,
            size,
            sys::esp_partition_mmap_memory_t_ESP_PARTITION_MMAP_DATA,
            &mut base,
            &mut handle,
        )
    };
    if err != sys::ESP_OK || base.is_null() {
        println!("warn coredump not mapped ({err})");
        return;
    }
    let base = base as usize + offset;
    // SAFETY: `base..base + len` is mapped and read only.
    let image = unsafe { core::slice::from_raw_parts(base as *const u8, len) };
    let crc = crc32(image);
    *DUMP.lock().unwrap_or_else(|e| e.into_inner()) = Some(Dump {
        part: part as usize,
        base,
        len,
        crc,
    });
}

/// `core info`: `core len=<n> crc=<hex>` or `core none`.
pub fn answer_info() {
    match &*DUMP.lock().unwrap_or_else(|e| e.into_inner()) {
        Some(d) => println!("core len={} crc={:08x}", d.len, d.crc),
        None => println!("core none"),
    }
    println!("ok");
}

/// `core read <offset>`: one piece of the image with its CRC.
pub fn answer_read(offset: usize) {
    let line = {
        let guard = DUMP.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            Some(d) if offset < d.len => {
                let n = CORE_CHUNK.min(d.len - offset);
                // SAFETY: `offset + n <= len`, inside the mapped image.
                let piece =
                    unsafe { core::slice::from_raw_parts((d.base + offset) as *const u8, n) };
                Ok(format_core_chunk(offset, piece))
            }
            Some(_) => Err("err offset past the end"),
            None => Err("err no dump"),
        }
    };
    match line {
        Ok(line) => {
            println!("{line}");
            println!("ok");
        }
        Err(e) => println!("{e}"),
    }
}

/// `core erase`: the first sector of the partition. A flash erase holds the cache of both cores
/// for a few tens of milliseconds: the editor sends it only after reading the dump.
pub fn answer_erase() {
    let mut guard = DUMP.lock().unwrap_or_else(|e| e.into_inner());
    let Some(d) = guard.take() else {
        println!("ok");
        return;
    };
    // SAFETY: the partition record is valid; the range is its first sector.
    let err =
        unsafe { sys::esp_partition_erase_range(d.part as *const sys::esp_partition_t, 0, ERASE) };
    if err == sys::ESP_OK {
        println!("ok");
    } else {
        *guard = Some(d);
        println!("err erase failed ({err})");
    }
}
