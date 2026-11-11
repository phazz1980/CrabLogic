//! The USB console of the board as a byte port: the answers go out through `println!`, the input
//! is read raw, because a module image is binary and the console's line ending conversion would
//! damage it. ESP32-C3: the built-in USB Serial/JTAG; classic ESP32: UART0, which the board's
//! USB-UART bridge (CP2102/CH340) brings to USB.

use esp_idf_svc::sys;

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
    Ok(())
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
    let ticks = wait_ms * sys::configTICK_RATE_HZ / 1000;
    // SAFETY: `buf` is writable for its whole length.
    let n = unsafe { sys::uart_read_bytes(0, buf.as_mut_ptr().cast(), buf.len() as u32, ticks) };
    n.max(0) as usize
}
