//! Ethernet of the master: a W5500 module on SPI2 (`specs/wasm-kernel/plan.md`, 4.2). Modules reach
//! this through the `net` import, ops 6..=8 (the same ABI as the C++ master):
//!
//! - op 6 `(code, value)`: numbers: 1 DHCP (0/1), 2 SCK, 3 MISO, 4 MOSI, 5 CS, 6 last MAC byte,
//!   63 the settings are complete;
//! - op 7 `(code, ptr, len)`: texts: 1 address, 2 gateway, 3 subnet mask;
//! - op 8 `(field)`: 0 ready (0/1), 1 IPv4 address (first octet in the lowest byte, as Wi-Fi op 3
//!   field 4); 0 while not ready.
//!
//! The generator calls ops 6 and 7 once at start (op 7 again when a setting driven by a tag
//! changes), so no step keeps calling: there is no lease as for the radio. Once the settings are
//! complete the interface stays up until the board is reset, as in the C++ master.
//!
//! The thread `crab-eth` starts on the first op 6 code 63. Pins and the MAC are taken once at that
//! start (a later change only puts one note in the journal); DHCP or a fixed address may change
//! at any time. The W5500 is polled over SPI (no INT wire is needed). Pins are checked per chip
//! before any hardware is touched.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crab_kernel::host::Mem;
use crab_master_rs::eth_cfg::{check_pins, parse_fixed, Chip, Fixed, Pins};
use esp_idf_svc::eth::{EspEth, EthDriver, SpiEth, SpiEthChipset, SpiEventSource};
use esp_idf_svc::hal::gpio::{AnyIOPin, AnyInputPin, AnyOutputPin};
use esp_idf_svc::hal::spi::config::DriverConfig;
use esp_idf_svc::hal::spi::{Dma, SpiDriver, SPI2};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::ipv4;
use esp_idf_svc::netif::{EspNetif, NetifConfiguration};
use esp_idf_svc::sys::EspError;

use super::net;

// The C3 is the only RISC-V chip of the master; the ESP32 is Xtensa.
#[cfg(target_arch = "riscv32")]
const CHIP: Chip = Chip::Esp32c3;
#[cfg(not(target_arch = "riscv32"))]
const CHIP: Chip = Chip::Esp32;

/// Stack of `crab-eth`: the SPI and Ethernet drivers are created on it.
const ETH_STACK: usize = 8192;
/// How often the thread looks at the settings and the link.
const TICK: Duration = Duration::from_millis(200);
/// How often the W5500 is read for events (no interrupt wire).
const POLL: Duration = Duration::from_millis(10);
const SPI_HZ: u32 = 20_000_000;
/// Longest text setting a module may send.
const MAX_TEXT: usize = 64;
/// DMA buffer of the SPI bus: more than a frame of the W5500.
const DMA_BYTES: usize = 4096;

type Eth = EspEth<'static, SpiEth<&'static SpiDriver<'static>>>;

/// What modules asked for (the defaults are those of the C++ master).
#[derive(Debug, Clone, PartialEq)]
struct Wanted {
    dhcp: bool,
    pins: Pins,
    /// Last byte of the MAC `78:AC:C0:2C:3E:xx`.
    mac: u8,
    ip: String,
    gateway: String,
    subnet: String,
    /// `net` op 6 code 63 came.
    complete: bool,
}

static WANTED: Mutex<Wanted> = Mutex::new(Wanted {
    dhcp: true,
    pins: Pins {
        sck: 18,
        miso: 19,
        mosi: 23,
        cs: 5,
    },
    mac: 40,
    ip: String::new(),
    gateway: String::new(),
    subnet: String::new(),
    complete: false,
});
static STARTED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
/// First octet in the lowest byte, as Wi-Fi reports it; 0 while not ready.
static IP: AtomicU32 = AtomicU32::new(0);

/// Messages for the master journal (see `net::take_notes`).
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const MAX_NOTES: usize = 16;

macro_rules! note {
    ($($arg:tt)*) => { push_note(format!($($arg)*)) };
}

fn push_note(text: String) {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    if q.len() == MAX_NOTES {
        q.pop_front();
    }
    q.push_back(text);
}

/// Takes the queued Ethernet messages (for the journal).
pub fn take_notes() -> Vec<String> {
    let mut q = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    q.drain(..).collect()
}

fn lock() -> std::sync::MutexGuard<'static, Wanted> {
    WANTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The interface has a link and an address: a service on top (MQTT, NTP) may open sockets.
pub fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------------------------
// The import
// ---------------------------------------------------------------------------------------------

/// `net(op, a, b, c)` for the Ethernet operations (6..=8).
pub fn call(op: i32, a: i32, b: i32, c: i32, mem: &mut Mem<'_>) -> i32 {
    match op {
        6 => set_number(a, b),
        7 => set_text(a, b, c, mem),
        8 => read(a),
        _ => 0,
    }
}

fn set_number(code: i32, value: i32) -> i32 {
    {
        let mut w = lock();
        match code {
            1 => w.dhcp = value != 0,
            2 => w.pins.sck = value,
            3 => w.pins.miso = value,
            4 => w.pins.mosi = value,
            5 => w.pins.cs = value,
            6 => w.mac = value.clamp(0, 255) as u8,
            63 => w.complete = true,
            _ => {}
        }
    }
    if code == 63 {
        start();
    }
    0
}

fn set_text(code: i32, ptr: i32, len: i32, mem: &Mem<'_>) -> i32 {
    let Some(bytes) = mem.read(ptr, len, MAX_TEXT) else {
        return 0;
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut w = lock();
    let field = match code {
        1 => &mut w.ip,
        2 => &mut w.gateway,
        3 => &mut w.subnet,
        _ => return 0,
    };
    if *field != text {
        *field = text;
    }
    0
}

fn read(field: i32) -> i32 {
    match field {
        0 => ready() as i32,
        1 => IP.load(Ordering::Acquire) as i32,
        _ => 0,
    }
}

fn start() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("crab-eth".into())
        .stack_size(ETH_STACK)
        .spawn(worker);
    if let Err(e) = spawned {
        note!("cannot start the Ethernet thread: {e}");
        STARTED.store(false, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------------------------
// The thread
// ---------------------------------------------------------------------------------------------

fn eth_netif(fixed: Option<Fixed>) -> Result<EspNetif, EspError> {
    let mut conf = NetifConfiguration::eth_default_client();
    if let Some((ip, gateway, prefix)) = fixed {
        conf.ip_configuration = Some(ipv4::Configuration::Client(
            ipv4::ClientConfiguration::Fixed(ipv4::ClientSettings {
                ip,
                subnet: ipv4::Subnet {
                    gateway,
                    mask: ipv4::Mask(prefix),
                },
                // The router usually answers DNS too, which SNTP needs for the server name.
                dns: Some(gateway),
                secondary_dns: None,
            }),
        ));
    }
    EspNetif::new_with_conf(&conf)
}

fn fixed_of(w: &Wanted) -> Option<Fixed> {
    match parse_fixed(w.dhcp, &w.ip, &w.gateway, &w.subnet) {
        Ok(f) => f,
        Err(text) => {
            note!("{text}");
            None
        }
    }
}

fn create(w: &Wanted, fixed: Option<Fixed>) -> Result<Eth, String> {
    let sysloop = net::sysloop().map_err(|e| format!("event loop: {e}"))?;
    let p = w.pins;
    // SAFETY: nothing else in the master uses SPI2 or these pins (the pins were checked against
    // the flash and the USB console of the chip), and the thread creates them once.
    let (spi, sck, mosi, miso, cs) = unsafe {
        (
            SPI2::new(),
            AnyOutputPin::new(p.sck),
            AnyOutputPin::new(p.mosi),
            AnyInputPin::new(p.miso),
            AnyIOPin::new(p.cs),
        )
    };
    let bus = SpiDriver::new(
        spi,
        sck,
        mosi,
        Some(miso),
        &DriverConfig::new().dma(Dma::Auto(DMA_BYTES)),
    )
    .map_err(|e| format!("SPI bus: {e}"))?;
    // The bus is never freed: when the W5500 does not answer, esp-idf-svc drops the driver without
    // removing its SPI device, and freeing the bus then fails an assert in ESP-IDF (reboot). There is
    // no second try before a reset anyway.
    let bus: &'static SpiDriver<'static> = Box::leak(Box::new(bus));
    let mac = [0x78, 0xAC, 0xC0, 0x2C, 0x3E, w.mac];
    let source = SpiEventSource::polling(POLL).map_err(|e| format!("poll period: {e}"))?;
    let driver = EthDriver::new_spi_with_event_source(
        bus,
        source,
        Some(cs),
        None::<AnyOutputPin>,
        SpiEthChipset::W5500,
        Hertz(SPI_HZ),
        Some(&mac),
        None,
        sysloop,
    )
    .map_err(|e| format!("W5500 driver: {e}"))?;
    let netif = eth_netif(fixed).map_err(|e| format!("interface: {e}"))?;
    EspEth::wrap_all(driver, netif).map_err(|e| format!("interface glue: {e}"))
}

fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_le_bytes(ip.octets())
}

fn worker() {
    let first = lock().clone();
    if let Err(text) = check_pins(CHIP, first.pins) {
        note!("{text}");
        // Not started: the next "settings complete" (a module restart) may bring other pins.
        STARTED.store(false, Ordering::Release);
        return;
    }
    let mut fixed = fixed_of(&first);
    let mut eth = match create(&first, fixed) {
        Ok(e) => e,
        Err(text) => {
            note!("cannot start Ethernet: {text}");
            // The bus stays taken (see `create`): no second try before a reset.
            return;
        }
    };
    if let Err(e) = eth.start() {
        note!("start failed: {e}");
        return;
    }
    note!("driver ready");
    let mut seen = first.clone();
    let mut told_pins = false;
    let mut link = false;
    let mut address = Ipv4Addr::UNSPECIFIED;
    loop {
        let wanted = lock().clone();
        if wanted != seen {
            if (wanted.pins, wanted.mac) != (first.pins, first.mac) && !told_pins {
                told_pins = true;
                note!("pins/MAC change needs a reset");
            }
            let new_fixed = fixed_of(&wanted);
            if new_fixed != fixed {
                let result = eth_netif(new_fixed).and_then(|n| eth.swap_netif(n).map(drop));
                match result {
                    Ok(()) => {
                        fixed = new_fixed;
                        address = Ipv4Addr::UNSPECIFIED;
                        note!("address mode changed ({})", mode_text(fixed));
                    }
                    Err(e) => note!("cannot change the interface: {e}"),
                }
            }
            seen = wanted;
        }
        let up = eth.is_connected().unwrap_or(false);
        if up != link {
            link = up;
            note!("link {}", if up { "up" } else { "down" });
        }
        let ip = if up && eth.netif().is_up().unwrap_or(false) {
            eth.netif()
                .get_ip_info()
                .map(|i| i.ip)
                .unwrap_or(Ipv4Addr::UNSPECIFIED)
        } else {
            Ipv4Addr::UNSPECIFIED
        };
        if ip != address {
            address = ip;
            if !ip.is_unspecified() {
                note!("address {ip}");
            }
        }
        IP.store(ip_to_u32(ip), Ordering::Release);
        READY.store(!ip.is_unspecified(), Ordering::Release);
        std::thread::sleep(TICK);
    }
}

fn mode_text(fixed: Option<Fixed>) -> &'static str {
    if fixed.is_some() {
        "fixed"
    } else {
        "DHCP"
    }
}
