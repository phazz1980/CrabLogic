//! `crab.bus` on the board: GPIO, I²C and UART for driver blocks. The rules (claims, arguments)
//! are in `crab_master_rs::bus_cfg`; here the checked requests meet the hardware.
//!
//! Every module has its own [`Bus`] (inside its `BoardDevices`), so claims are per module and
//! vanish when the module is reloaded or removed. The I²C controller (I2C0) is one per board:
//! its pins are set by the first open and shared by the modules that open it with the same pins.
//! UART1/UART2 go to the first module that opens them. The SPI controller for drivers (SPI3 on the
//! classic ESP32, SPI2 on the C3) is shared like I2C0; chip select is a GPIO driven here. 1-Wire is
//! bit-banged on a claimed pin.

use std::sync::Mutex;

use crab_kernel::abi::bus_op::{ERR_ARGS, ERR_BUS, ERR_NOT_CLAIMED, ERR_TIMEOUT, MAX_LEN};
use crab_kernel::host::Mem;
use crab_master_rs::bus_cfg::{i2c_open_rule, spi_open_rule, Claims, Request};
use crab_master_rs::eth_cfg::Chip;
use esp_idf_svc::sys;

#[cfg(target_arch = "riscv32")]
const CHIP: Chip = Chip::Esp32c3;
#[cfg(not(target_arch = "riscv32"))]
const CHIP: Chip = Chip::Esp32;

const I2C_PORT: sys::i2c_port_t = 0;
/// One transaction waits this long, milliseconds.
const I2C_TIMEOUT_MS: u32 = 10;
const UART_BUF: i32 = 512;

/// The I²C controller of the board: pins of the open one and how many modules use it.
struct I2cShared {
    pins: Option<(i32, i32)>,
    users: u32,
}

static I2C: Mutex<I2cShared> = Mutex::new(I2cShared {
    pins: None,
    users: 0,
});

/// SPI controller of drivers: the classic ESP32 keeps SPI2 for the W5500, the C3 has only SPI2.
#[cfg(target_arch = "riscv32")]
const SPI_HOST: sys::spi_host_device_t = sys::spi_host_device_t_SPI2_HOST;
#[cfg(not(target_arch = "riscv32"))]
const SPI_HOST: sys::spi_host_device_t = sys::spi_host_device_t_SPI3_HOST;

/// An ESP-IDF SPI device handle (a raw pointer, only touched under the `SPI` mutex).
#[derive(Clone, Copy)]
struct SpiDev(sys::spi_device_handle_t);

// SAFETY: the handle is used only while the `SPI` mutex is held.
unsafe impl Send for SpiDev {}

const NO_DEV: SpiDev = SpiDev(core::ptr::null_mut());

/// The SPI controller of the board: pins and clock of the open one, how many modules use it, one
/// device per SPI mode (created on first use, no hardware CS) and the CS pins already set up.
struct SpiShared {
    pins: Option<(i32, i32, i32)>,
    hz: u32,
    users: u32,
    devs: [SpiDev; 4],
    cs_ready: u64,
}

static SPI: Mutex<SpiShared> = Mutex::new(SpiShared {
    pins: None,
    hz: 0,
    users: 0,
    devs: [NO_DEV; 4],
    cs_ready: 0,
});

/// What one module has opened and claimed.
#[derive(Debug, Default)]
pub struct Bus {
    claims: Claims,
    i2c: bool,
    /// Bit per UART port this module installed.
    uarts: u8,
    /// This module uses the SPI controller.
    spi: bool,
    /// Bit per pin set up for 1-Wire by this module.
    ow: u64,
}

fn ticks(ms: u32) -> u32 {
    // The tick is 10 ms by default; never round a wait to nothing.
    (ms * sys::configTICK_RATE_HZ / 1000).max(1)
}

/// Maps an ESP-IDF error of a transfer to a `crab.bus` code.
fn io_error(err: sys::esp_err_t) -> i32 {
    match err {
        sys::ESP_ERR_TIMEOUT => ERR_TIMEOUT,
        sys::ESP_ERR_INVALID_ARG => ERR_ARGS,
        _ => ERR_BUS,
    }
}

impl Bus {
    /// One call of `crab.bus`.
    pub fn call(&mut self, op: i32, a: [i32; 4], mem: &mut Mem<'_>) -> i32 {
        match self.claims.request(CHIP, op, a[0], a[1], a[2], a[3]) {
            Ok(req) => self.run(req, mem),
            Err(code) => code,
        }
    }

    fn run(&mut self, req: Request, mem: &mut Mem<'_>) -> i32 {
        match req {
            Request::Done => 0,
            Request::GpioMode { pin, mode } => gpio_mode(pin, mode),
            // SAFETY: the pin was claimed and checked against the chip.
            Request::GpioRead { pin } => unsafe { sys::gpio_get_level(pin) },
            Request::GpioWrite { pin, high } => {
                // SAFETY: as above.
                unsafe { sys::gpio_set_level(pin, high as u32) };
                0
            }
            Request::I2cOpen { sda, scl, hz } => self.i2c_open(sda, scl, hz),
            Request::I2cWrite { addr, ptr, len } => {
                let Some(data) = self.i2c_ready(mem.read(ptr, len as i32, len)) else {
                    return self.i2c_fail();
                };
                // SAFETY: `data` outlives the call; the address was claimed.
                let err = unsafe {
                    sys::i2c_master_write_to_device(
                        I2C_PORT,
                        addr as u8,
                        data.as_ptr(),
                        data.len(),
                        ticks(I2C_TIMEOUT_MS),
                    )
                };
                if err == 0 {
                    len as i32
                } else {
                    io_error(err)
                }
            }
            Request::I2cRead { addr, ptr, len } => {
                if !self.i2c || mem.read(ptr, len as i32, len).is_none() {
                    return self.i2c_fail();
                }
                let mut buf = [0u8; MAX_LEN as usize];
                // SAFETY: `buf` holds `len` ≤ 256 bytes.
                let err = unsafe {
                    sys::i2c_master_read_from_device(
                        I2C_PORT,
                        addr as u8,
                        buf.as_mut_ptr(),
                        len,
                        ticks(I2C_TIMEOUT_MS),
                    )
                };
                if err != 0 {
                    return io_error(err);
                }
                store(mem, ptr, &buf[..len])
            }
            Request::I2cWriteRead {
                addr,
                wptr,
                wlen,
                rptr,
                rlen,
            } => {
                let Some(data) = self.i2c_ready(mem.read(wptr, wlen as i32, wlen)) else {
                    return self.i2c_fail();
                };
                if mem.read(rptr, rlen as i32, rlen).is_none() {
                    return ERR_ARGS;
                }
                let mut buf = [0u8; MAX_LEN as usize];
                // SAFETY: both buffers outlive the call and have the stated sizes.
                let err = unsafe {
                    sys::i2c_master_write_read_device(
                        I2C_PORT,
                        addr as u8,
                        data.as_ptr(),
                        data.len(),
                        buf.as_mut_ptr(),
                        rlen,
                        ticks(I2C_TIMEOUT_MS),
                    )
                };
                if err != 0 {
                    return io_error(err);
                }
                store(mem, rptr, &buf[..rlen])
            }
            Request::UartOpen { port, tx, rx, baud } => self.uart_open(port, tx, rx, baud),
            Request::UartWrite { port, ptr, len } => {
                let Some(data) = mem.read(ptr, len as i32, len) else {
                    return ERR_ARGS;
                };
                if !self.owns_uart(port) {
                    return ERR_NOT_CLAIMED;
                }
                // SAFETY: the driver of the port is installed; `data` outlives the call.
                let n = unsafe {
                    sys::uart_write_bytes(
                        port as sys::uart_port_t,
                        data.as_ptr().cast(),
                        data.len(),
                    )
                };
                if n < 0 {
                    ERR_BUS
                } else {
                    n
                }
            }
            Request::UartRead { port, ptr, cap } => {
                if mem.read(ptr, cap as i32, cap).is_none() {
                    return ERR_ARGS;
                }
                if !self.owns_uart(port) {
                    return ERR_NOT_CLAIMED;
                }
                let mut buf = [0u8; MAX_LEN as usize];
                // SAFETY: `buf` holds `cap` ≤ 256 bytes; zero ticks: never waits.
                let n = unsafe {
                    sys::uart_read_bytes(
                        port as sys::uart_port_t,
                        buf.as_mut_ptr().cast(),
                        cap as u32,
                        0,
                    )
                };
                if n < 0 {
                    return ERR_BUS;
                }
                store(mem, ptr, &buf[..n as usize])
            }
            Request::UartAvailable { port } => {
                if !self.owns_uart(port) {
                    return ERR_NOT_CLAIMED;
                }
                let mut n = 0usize;
                // SAFETY: the driver is installed; `n` is a valid out value.
                let err =
                    unsafe { sys::uart_get_buffered_data_len(port as sys::uart_port_t, &mut n) };
                if err == 0 {
                    n.min(i32::MAX as usize) as i32
                } else {
                    ERR_BUS
                }
            }
            // SAFETY: no arguments, no state.
            Request::Micros => (unsafe { sys::esp_timer_get_time() }) as i32,
            Request::DelayUs { us } => {
                // SAFETY: a busy wait of at most 100 µs.
                unsafe { sys::esp_rom_delay_us(us) };
                0
            }
            Request::SpiOpen {
                sck,
                mosi,
                miso,
                hz,
            } => self.spi_open(sck, mosi, miso, hz),
            Request::SpiTransfer { cs, ptr, len, mode } => {
                if !self.spi {
                    return ERR_NOT_CLAIMED;
                }
                let Some(data) = mem.read(ptr, len as i32, len) else {
                    return ERR_ARGS;
                };
                match spi_transfer(cs, mode, &data) {
                    Ok(Some(rx)) => store(mem, ptr, &rx),
                    Ok(None) => len as i32,
                    Err(code) => code,
                }
            }
            Request::OwReset { pin } => {
                self.ow_setup(pin);
                ow_reset(pin) as i32
            }
            Request::OwWrite { pin, ptr, len } => {
                let Some(data) = mem.read(ptr, len as i32, len) else {
                    return ERR_ARGS;
                };
                self.ow_setup(pin);
                for byte in &data {
                    for bit in 0..8 {
                        ow_write_bit(pin, byte >> bit & 1 != 0);
                    }
                }
                len as i32
            }
            Request::OwRead { pin, ptr, len } => {
                if mem.read(ptr, len as i32, len).is_none() {
                    return ERR_ARGS;
                }
                self.ow_setup(pin);
                let mut buf = [0u8; MAX_LEN as usize];
                for byte in buf.iter_mut().take(len) {
                    for bit in 0..8 {
                        *byte |= (ow_read_bit(pin) as u8) << bit;
                    }
                }
                store(mem, ptr, &buf[..len])
            }
            Request::OwReadBit { pin } => {
                self.ow_setup(pin);
                ow_read_bit(pin) as i32
            }
            Request::OwWriteBit { pin, bit } => {
                self.ow_setup(pin);
                ow_write_bit(pin, bit);
                0
            }
        }
    }

    /// The data to send if this module opened the bus and the memory range is valid.
    fn i2c_ready(&self, data: Option<Vec<u8>>) -> Option<Vec<u8>> {
        if self.i2c {
            data
        } else {
            None
        }
    }

    /// The code for a transfer that could not start: the bus is not open (-1) or the pointer is bad.
    fn i2c_fail(&self) -> i32 {
        if self.i2c {
            ERR_ARGS
        } else {
            ERR_NOT_CLAIMED
        }
    }

    fn owns_uart(&self, port: i32) -> bool {
        self.uarts >> port & 1 != 0
    }

    fn i2c_open(&mut self, sda: i32, scl: i32, hz: u32) -> i32 {
        let mut shared = I2C.lock().unwrap_or_else(|e| e.into_inner());
        let setup = match i2c_open_rule(shared.pins, sda, scl) {
            Ok(setup) => setup,
            Err(code) => return code,
        };
        if self.i2c {
            return 0;
        }
        if setup {
            // SAFETY: a zeroed config is valid for the C struct; the fields are set below.
            let mut cfg: sys::i2c_config_t = unsafe { core::mem::zeroed() };
            cfg.mode = sys::i2c_mode_t_I2C_MODE_MASTER;
            cfg.sda_io_num = sda;
            cfg.scl_io_num = scl;
            cfg.sda_pullup_en = true;
            cfg.scl_pullup_en = true;
            cfg.__bindgen_anon_1.master.clk_speed = hz;
            // SAFETY: the pins were claimed and checked; the config lives through the call.
            let err = unsafe { sys::i2c_param_config(I2C_PORT, &cfg) };
            if err != 0 {
                return ERR_ARGS;
            }
            // SAFETY: master mode needs no buffers.
            let err = unsafe { sys::i2c_driver_install(I2C_PORT, cfg.mode, 0, 0, 0) };
            if err != 0 {
                return ERR_BUS;
            }
            shared.pins = Some((sda, scl));
        }
        shared.users += 1;
        self.i2c = true;
        0
    }

    fn spi_open(&mut self, sck: i32, mosi: i32, miso: i32, hz: u32) -> i32 {
        let mut shared = SPI.lock().unwrap_or_else(|e| e.into_inner());
        let setup = match spi_open_rule(shared.pins, sck, mosi, miso) {
            Ok(setup) => setup,
            Err(code) => return code,
        };
        if self.spi {
            return 0;
        }
        if setup {
            #[cfg(target_arch = "riscv32")]
            if super::eth::spi2_taken() {
                super::eth::push_note(
                    "SPI for drivers: SPI2 is taken by the W5500 Ethernet (the C3 has no other SPI)"
                        .into(),
                );
                return ERR_BUS;
            }
            // SAFETY: a zeroed config is valid for the C struct; the fields are set below.
            let mut cfg: sys::spi_bus_config_t = unsafe { core::mem::zeroed() };
            cfg.__bindgen_anon_1.mosi_io_num = mosi;
            cfg.__bindgen_anon_2.miso_io_num = miso;
            cfg.sclk_io_num = sck;
            cfg.__bindgen_anon_3.quadwp_io_num = -1;
            cfg.__bindgen_anon_4.quadhd_io_num = -1;
            cfg.max_transfer_sz = MAX_LEN;
            // SAFETY: the pins were claimed and checked; the config lives through the call.
            let err = unsafe {
                sys::spi_bus_initialize(SPI_HOST, &cfg, sys::spi_common_dma_t_SPI_DMA_CH_AUTO)
            };
            if err != 0 {
                if err == sys::ESP_ERR_INVALID_STATE {
                    super::eth::push_note(
                        "SPI for drivers: the SPI controller is already in use".into(),
                    );
                }
                return io_error(err);
            }
            shared.pins = Some((sck, mosi, miso));
            shared.hz = hz;
            shared.devs = [NO_DEV; 4];
            shared.cs_ready = 0;
        }
        shared.users += 1;
        self.spi = true;
        0
    }

    /// Sets a pin up for 1-Wire once per module: open-drain with the internal pull-up, released.
    fn ow_setup(&mut self, pin: i32) {
        if self.ow >> pin & 1 != 0 {
            return;
        }
        // SAFETY: the pin was claimed and checked against the chip.
        unsafe {
            sys::gpio_reset_pin(pin);
            super::devices::forget_output(pin);
            sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD);
            sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY);
            sys::gpio_set_level(pin, 1);
        }
        self.ow |= 1 << pin;
    }

    fn uart_open(&mut self, port: i32, tx: i32, rx: i32, baud: u32) -> i32 {
        if self.owns_uart(port) {
            return 0;
        }
        let cfg = sys::uart_config_t {
            baud_rate: baud as i32,
            data_bits: sys::uart_word_length_t_UART_DATA_8_BITS,
            parity: sys::uart_parity_t_UART_PARITY_DISABLE,
            stop_bits: sys::uart_stop_bits_t_UART_STOP_BITS_1,
            flow_ctrl: sys::uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE,
            rx_flow_ctrl_thresh: 0,
            ..Default::default()
        };
        // SAFETY: the port and pins were checked; the driver is installed first, so a port used
        // by another module (install fails) is left alone.
        unsafe {
            if sys::uart_driver_install(
                port as sys::uart_port_t,
                UART_BUF,
                UART_BUF,
                0,
                core::ptr::null_mut(),
                0,
            ) != 0
            {
                return ERR_BUS;
            }
            let err = sys::uart_param_config(port as sys::uart_port_t, &cfg);
            let err = if err == 0 {
                sys::uart_set_pin(port as sys::uart_port_t, tx, rx, -1, -1)
            } else {
                err
            };
            if err != 0 {
                sys::uart_driver_delete(port as sys::uart_port_t);
                return ERR_ARGS;
            }
        }
        self.uarts |= 1 << port;
        0
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        for port in 0..8 {
            if self.owns_uart(port) {
                // SAFETY: this module installed the driver.
                unsafe { sys::uart_driver_delete(port as sys::uart_port_t) };
            }
        }
        for pin in 0..64 {
            if self.ow >> pin & 1 != 0 {
                // SAFETY: this module set the pin up.
                {
                    unsafe { sys::gpio_reset_pin(pin) };
                    super::devices::forget_output(pin);
                }
            }
        }
        if self.spi {
            let mut shared = SPI.lock().unwrap_or_else(|e| e.into_inner());
            shared.users = shared.users.saturating_sub(1);
            if shared.users == 0 {
                for dev in shared.devs {
                    if !dev.0.is_null() {
                        // SAFETY: the device was added by this controller and is not in use.
                        unsafe { sys::spi_bus_remove_device(dev.0) };
                    }
                }
                // SAFETY: the last user of the controller is gone, all devices are removed.
                unsafe { sys::spi_bus_free(SPI_HOST) };
                *shared = SpiShared {
                    pins: None,
                    hz: 0,
                    users: 0,
                    devs: [NO_DEV; 4],
                    cs_ready: 0,
                };
            }
        }
        if self.i2c {
            let mut shared = I2C.lock().unwrap_or_else(|e| e.into_inner());
            shared.users = shared.users.saturating_sub(1);
            if shared.users == 0 {
                // SAFETY: the last user of the controller is gone.
                unsafe { sys::i2c_driver_delete(I2C_PORT) };
                shared.pins = None;
            }
        }
    }
}

/// One full-duplex SPI transfer with CS held low by GPIO. `Ok(None)`: nothing to read back (no MISO).
fn spi_transfer(cs: i32, mode: u8, tx: &[u8]) -> Result<Option<Vec<u8>>, i32> {
    let mut shared = SPI.lock().unwrap_or_else(|e| e.into_inner());
    let Some((_, _, miso)) = shared.pins else {
        return Err(ERR_NOT_CLAIMED);
    };
    let slot = mode as usize;
    if shared.devs[slot].0.is_null() {
        // SAFETY: a zeroed config is valid for the C struct; the fields are set below.
        let mut cfg: sys::spi_device_interface_config_t = unsafe { core::mem::zeroed() };
        cfg.mode = mode;
        cfg.clock_speed_hz = shared.hz as i32;
        cfg.spics_io_num = -1;
        cfg.queue_size = 1;
        let mut handle: sys::spi_device_handle_t = core::ptr::null_mut();
        // SAFETY: the bus is initialized (pins are set); the config and handle live through the call.
        let err = unsafe { sys::spi_bus_add_device(SPI_HOST, &cfg, &mut handle) };
        if err != 0 {
            return Err(io_error(err));
        }
        shared.devs[slot] = SpiDev(handle);
    }
    if cs >= 0 && shared.cs_ready >> cs & 1 == 0 {
        // SAFETY: the pin was claimed and checked against the chip.
        unsafe {
            sys::gpio_reset_pin(cs);
            super::devices::forget_output(cs);
            sys::gpio_set_level(cs, 1);
            sys::gpio_set_direction(cs, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        }
        shared.cs_ready |= 1 << cs;
    }
    let mut rx = vec![0u8; if miso >= 0 { tx.len() } else { 0 }];
    // SAFETY: a zeroed transaction is valid for the C struct; the buffers outlive the call.
    let mut t: sys::spi_transaction_t = unsafe { core::mem::zeroed() };
    t.length = tx.len() * 8;
    t.__bindgen_anon_1.tx_buffer = tx.as_ptr().cast();
    if miso >= 0 {
        t.__bindgen_anon_2.rx_buffer = rx.as_mut_ptr().cast();
    }
    if cs >= 0 {
        // SAFETY: as above.
        unsafe { sys::gpio_set_level(cs, 0) };
    }
    // SAFETY: the device handle is valid while the `SPI` lock is held; `t` and the buffers are live.
    let err = unsafe { sys::spi_device_polling_transmit(shared.devs[slot].0, &mut t) };
    if cs >= 0 {
        // SAFETY: as above.
        unsafe { sys::gpio_set_level(cs, 1) };
    }
    if err != 0 {
        return Err(io_error(err));
    }
    Ok((miso >= 0).then_some(rx))
}

fn delay_us(us: u32) {
    // SAFETY: a busy wait of a few microseconds.
    unsafe { sys::esp_rom_delay_us(us) };
}

/// Runs the timing-critical part of a 1-Wire slot with interrupts off (a few tens of µs at most).
fn critical<R>(f: impl FnOnce() -> R) -> R {
    esp_idf_svc::hal::interrupt::free(f)
}

fn ow_low(pin: i32) {
    // SAFETY: the pin is claimed and set up as open-drain.
    unsafe { sys::gpio_set_level(pin, 0) };
}

fn ow_release(pin: i32) {
    // SAFETY: as above.
    unsafe { sys::gpio_set_level(pin, 1) };
}

fn ow_level(pin: i32) -> bool {
    // SAFETY: as above.
    unsafe { sys::gpio_get_level(pin) != 0 }
}

/// 1-Wire reset: low 480 µs (outside the critical section, longer is harmless), release, sample the
/// presence pulse 70 µs later with interrupts off, wait out the rest of the slot.
fn ow_reset(pin: i32) -> bool {
    ow_low(pin);
    delay_us(480);
    let present = critical(|| {
        ow_release(pin);
        delay_us(70);
        !ow_level(pin)
    });
    delay_us(410);
    present
}

fn ow_write_bit(pin: i32, bit: bool) {
    if bit {
        critical(|| {
            ow_low(pin);
            delay_us(6);
            ow_release(pin);
        });
        delay_us(64);
    } else {
        critical(|| {
            ow_low(pin);
            delay_us(60);
            ow_release(pin);
        });
        delay_us(10);
    }
}

fn ow_read_bit(pin: i32) -> bool {
    let bit = critical(|| {
        ow_low(pin);
        delay_us(6);
        ow_release(pin);
        delay_us(9);
        ow_level(pin)
    });
    delay_us(55);
    bit
}

/// Copies `data` into the module; the byte count, or `ERR_ARGS` if the range is outside its memory.
fn store(mem: &mut Mem<'_>, ptr: i32, data: &[u8]) -> i32 {
    if mem.write(ptr, data) {
        data.len() as i32
    } else {
        ERR_ARGS
    }
}

/// Sets the direction, driver and pulls of a pin: 0 input, 1 output, 2 pull-up, 3 pull-down,
/// 4 open-drain output. Outputs can be read back.
fn gpio_mode(pin: i32, mode: i32) -> i32 {
    let (dir, pull) = match mode {
        0 => (
            sys::gpio_mode_t_GPIO_MODE_INPUT,
            sys::gpio_pull_mode_t_GPIO_FLOATING,
        ),
        1 => (
            sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT,
            sys::gpio_pull_mode_t_GPIO_FLOATING,
        ),
        2 => (
            sys::gpio_mode_t_GPIO_MODE_INPUT,
            sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY,
        ),
        3 => (
            sys::gpio_mode_t_GPIO_MODE_INPUT,
            sys::gpio_pull_mode_t_GPIO_PULLDOWN_ONLY,
        ),
        _ => (
            sys::gpio_mode_t_GPIO_MODE_INPUT_OUTPUT_OD,
            sys::gpio_pull_mode_t_GPIO_FLOATING,
        ),
    };
    // SAFETY: the pin was claimed and checked against the chip.
    let err = unsafe {
        // Some ESP32 pins (12–15 are JTAG) boot with another function: take them as GPIO.
        sys::gpio_reset_pin(pin);
        super::devices::forget_output(pin);
        let e = sys::gpio_set_direction(pin, dir);
        if e == 0 {
            sys::gpio_set_pull_mode(pin, pull)
        } else {
            e
        }
    };
    if err == 0 {
        0
    } else {
        ERR_ARGS
    }
}
