//! The USB console of the board (USB Serial/JTAG) as a byte port: the answers go out through
//! `println!`, the input is read raw, because a module image is binary and the console's line
//! ending conversion would damage it.

use esp_idf_svc::sys;

/// Bytes the driver keeps until the master reads them; a `put` chunk must fit.
const RX_BUFFER: u32 = 4096;
const TX_BUFFER: u32 = 2048;

pub fn init() -> Result<(), String> {
    let mut config = sys::usb_serial_jtag_driver_config_t {
        tx_buffer_size: TX_BUFFER,
        rx_buffer_size: RX_BUFFER,
    };
    // SAFETY: called once, before anything reads the console.
    let err = unsafe { sys::usb_serial_jtag_driver_install(&mut config) };
    if err != sys::ESP_OK {
        return Err(format!("драйвер USB не запущен (ошибка {err})"));
    }
    // `println!` now goes through the driver as well and does not lose bytes.
    unsafe { sys::esp_vfs_usb_serial_jtag_use_driver() };
    Ok(())
}

/// Reads what has come, waiting up to `wait_ms` for the first byte.
pub fn read(buf: &mut [u8], wait_ms: u32) -> usize {
    let ticks = wait_ms * sys::configTICK_RATE_HZ / 1000;
    // SAFETY: `buf` is writable for its whole length.
    let n = unsafe {
        sys::usb_serial_jtag_read_bytes(buf.as_mut_ptr().cast(), buf.len() as u32, ticks)
    };
    n.max(0) as usize
}
