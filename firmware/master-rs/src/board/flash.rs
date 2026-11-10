//! The `crabmod` partition of the board: read, write and erase through ESP-IDF, and the whole
//! partition mapped into the address space so that AOT images run in place.

use core::ffi::c_void;

use crab_master_rs::storage::Flash;
use esp_idf_svc::sys;

/// The custom partition type of `partitions.csv`.
const PARTITION_TYPE: sys::esp_partition_type_t = 0x40;

pub struct PartitionFlash {
    part: *const sys::esp_partition_t,
    /// Where the partition is mapped (data bus).
    base: *const u8,
    size: usize,
}

impl PartitionFlash {
    pub fn open() -> Result<Self, String> {
        // SAFETY: the name is NUL-terminated; the returned record lives as long as the firmware.
        let part = unsafe {
            sys::esp_partition_find_first(
                PARTITION_TYPE,
                sys::esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_ANY,
                c"crabmod".as_ptr(),
            )
        };
        if part.is_null() {
            return Err(
                "в таблице разделов нет раздела crabmod: прошейте плату вместе с partitions.csv"
                    .to_owned(),
            );
        }
        let size = unsafe { (*part).size } as usize;
        let mut base: *const c_void = core::ptr::null();
        let mut handle: sys::esp_partition_mmap_handle_t = 0;
        // SAFETY: `part` is valid; the mapping is never released.
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
            return Err(format!(
                "раздел crabmod не отображён в память (ошибка {err})"
            ));
        }
        Ok(Self {
            part,
            base: base.cast(),
            size,
        })
    }

    /// The bytes of the partition as the CPU sees them.
    ///
    /// # Safety
    /// The slice is only valid until the range is erased or written: whoever holds it (a
    /// running AOT module) must be gone before its area is reused.
    pub unsafe fn mapped(&self, at: usize, len: usize) -> Option<&'static [u8]> {
        (at.checked_add(len)? <= self.size)
            .then(|| core::slice::from_raw_parts(self.base.add(at), len))
    }

    fn inside(&self, at: usize, len: usize) -> bool {
        at.checked_add(len).is_some_and(|end| end <= self.size)
    }
}

impl Flash for PartitionFlash {
    fn size(&self) -> usize {
        self.size
    }

    fn read(&self, at: usize, buf: &mut [u8]) -> bool {
        self.inside(at, buf.len())
            // SAFETY: the range is inside the partition and `buf` is writable.
            && unsafe { sys::esp_partition_read(self.part, at, buf.as_mut_ptr().cast(), buf.len()) }
                == sys::ESP_OK
    }

    fn write(&mut self, at: usize, data: &[u8]) -> bool {
        self.inside(at, data.len())
            // SAFETY: the range is inside the partition.
            && unsafe { sys::esp_partition_write(self.part, at, data.as_ptr().cast(), data.len()) }
                == sys::ESP_OK
    }

    fn erase(&mut self, at: usize, len: usize) -> bool {
        self.inside(at, len)
            // SAFETY: the range is inside the partition.
            && unsafe { sys::esp_partition_erase_range(self.part, at, len) } == sys::ESP_OK
    }
}
