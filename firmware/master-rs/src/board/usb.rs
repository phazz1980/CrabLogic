//! The USB console of the board as a byte port: the answers go out through `println!`, the input
//! is read raw, because a module image is binary and the console's line ending conversion would
//! damage it. ESP32-C3: the built-in USB Serial/JTAG; classic ESP32: UART0, which the board's
//! USB-UART bridge (CP2102/CH340) brings to USB.

use esp_idf_svc::sys;
#[cfg(not(target_arch = "riscv32"))]
use std::sync::atomic::{AtomicBool, Ordering};

/// Bytes the driver keeps until the master reads them; a `put` chunk must fit.
const RX_BUFFER: u32 = 4096;
const TX_BUFFER: u32 = 2048;

#[cfg(target_arch = "riscv32")]
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

#[cfg(not(target_arch = "riscv32"))]
pub fn init() -> Result<(), String> {
    const UART: sys::uart_port_t = 0;
    // SAFETY: called once, before anything reads the console; UART0 is already configured by
    // the console of ESP-IDF (115200 baud), only its driver is added.
    let err = unsafe {
        sys::uart_driver_install(
            UART,
            RX_BUFFER as i32,
            TX_BUFFER as i32,
            0,
            core::ptr::null_mut(),
            0,
        )
    };
    if err != sys::ESP_OK {
        return Err(format!("драйвер UART0 не запущен (ошибка {err})"));
    }
    // `println!` now goes through the driver as well and does not lose bytes.
    unsafe { sys::esp_vfs_dev_uart_use_driver(UART as i32) };
    UART_READY.store(true, Ordering::Release);
    Ok(())
}

/// The UART0 driver is installed: lines may go to it directly.
#[cfg(not(target_arch = "riscv32"))]
static UART_READY: AtomicBool = AtomicBool::new(false);

/// One line of the master's answer with CR LF. Classic ESP32: a single write to the UART0
/// driver. Its TX ring buffer stores every write as an item with a header, and the stdout of
/// ESP-IDF writes byte by byte, so a 2 KB buffer held about 70 characters and a `list` answer
/// waited for the 115200 baud line (the scan stood still up to ~35 ms).
#[cfg(not(target_arch = "riscv32"))]
pub fn write_line(text: &str) {
    if !UART_READY.load(Ordering::Acquire) {
        std::println!("{text}");
        return;
    }
    let mut line = Vec::with_capacity(text.len() + 2);
    line.extend_from_slice(text.as_bytes());
    line.extend_from_slice(b"\r\n");
    // SAFETY: the driver is installed; the bytes live through the call (they are copied).
    unsafe { sys::uart_write_bytes(0, line.as_ptr().cast(), line.len()) };
}

/// One line of the master's answer: the USB Serial/JTAG console of the C3 keeps up as it is.
#[cfg(target_arch = "riscv32")]
pub fn write_line(text: &str) {
    std::println!("{text}");
}

/// Reads what has come, waiting up to `wait_ms` for the first byte.
#[cfg(target_arch = "riscv32")]
pub fn read(buf: &mut [u8], wait_ms: u32) -> usize {
    let ticks = wait_ms * sys::configTICK_RATE_HZ / 1000;
    // SAFETY: `buf` is writable for its whole length.
    let n = unsafe {
        sys::usb_serial_jtag_read_bytes(buf.as_mut_ptr().cast(), buf.len() as u32, ticks)
    };
    n.max(0) as usize
}

/// Reads what has come, waiting up to `wait_ms` for the first byte.
#[cfg(not(target_arch = "riscv32"))]
pub fn read(buf: &mut [u8], wait_ms: u32) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let ticks = wait_ms * sys::configTICK_RATE_HZ / 1000;
    // `uart_read_bytes` waits until the whole length has come or the time is out: wait for the
    // first byte only, then take what is already buffered, so a command is handled at once and
    // the next scan is not held back.
    // SAFETY: `buf` is writable for its whole length.
    let first = unsafe { sys::uart_read_bytes(0, buf.as_mut_ptr().cast(), 1, ticks) };
    if first <= 0 {
        return 0;
    }
    let mut ready = 0usize;
    // SAFETY: plain query of the installed driver.
    unsafe { sys::uart_get_buffered_data_len(0, &mut ready) };
    let more = ready.min(buf.len() - 1);
    if more == 0 {
        return 1;
    }
    // SAFETY: the rest of `buf` is writable; the bytes are already in the driver buffer.
    let n = unsafe { sys::uart_read_bytes(0, buf[1..].as_mut_ptr().cast(), more as u32, 0) };
    1 + n.max(0) as usize
}
