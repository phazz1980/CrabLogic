//! Ethernet of the master: a W5500 module on SPI2 (`specs/wasm-kernel/plan.md`, 4.2) or, on the
//! classic ESP32, a LAN8720 PHY on the EMAC (`specs/lan8720`). Modules reach this through the
//! `net` import, ops 6..=8 (the same ABI as the C++ master):
//!
//! - op 6 `(code, value)`: numbers: 1 DHCP (0/1), 2 SCK, 3 MISO, 4 MOSI, 5 CS, 6 last MAC byte,
//!   7 chip (0 W5500, 1 LAN8720), 8 PHY address, 9 MDC, 10 MDIO, 11 PHY power pin (-1 none),
//!   12 RMII clock mode (as `FLPROG_RMII_CLOCK_*`), 63 the settings are complete;
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
//! before any hardware is touched; LAN8720 on the C3 (no EMAC) only puts a note in the journal.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crab_kernel::host::Mem;
use crab_master_rs::eth_cfg::{check_pins, check_rmii, parse_fixed, Chip, Fixed, Pins, Rmii};
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
    /// LAN8720 on the EMAC instead of the W5500 (codes 7..=12).
    rmii: Option<Rmii>,
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
    rmii: None,
    ip: String::new(),
    gateway: String::new(),
    subnet: String::new(),
    complete: false,
});
static STARTED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
/// The W5500 driver took SPI2 (never freed): the SPI of driver blocks cannot use it on the C3.
static SPI2_TAKEN: AtomicBool = AtomicBool::new(false);
/// First octet in the lowest byte, as Wi-Fi reports it; 0 while not ready.
static IP: AtomicU32 = AtomicU32::new(0);

/// Messages for the master journal (see `net::take_notes`).
static NOTES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const MAX_NOTES: usize = 16;

macro_rules! note {
    ($($arg:tt)*) => { push_note(format!($($arg)*)) };
}

pub fn push_note(text: String) {
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

/// Whether the W5500 driver took (or is about to take) SPI2.
#[cfg(target_arch = "riscv32")]
pub fn spi2_taken() -> bool {
    SPI2_TAKEN.load(Ordering::Acquire)
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
            7 => w.rmii = (value == 1).then_some(Rmii::MODULE),
            8..=12 => {
                let r = w.rmii.get_or_insert(Rmii::MODULE);
                match code {
                    8 => r.phy_addr = value,
                    9 => r.mdc = value,
                    10 => r.mdio = value,
                    11 => r.power = value,
                    _ => r.clock = value,
                }
            }
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

fn create_spi(w: &Wanted, fixed: Option<Fixed>) -> Result<Eth, String> {
    let sysloop = net::sysloop().map_err(|e| format!("event loop: {e}"))?;
    let p = w.pins;
    SPI2_TAKEN.store(true, Ordering::Release);
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

/// LAN8720 on the EMAC of the classic ESP32. The data pins are fixed by the chip; MDC, MDIO and
/// the power pin come from the module.
#[cfg(target_arch = "xtensa")]
fn create_rmii(
    r: Rmii,
    fixed: Option<Fixed>,
) -> Result<EspEth<'static, esp_idf_svc::eth::RmiiEth>, String> {
    use esp_idf_svc::eth::{RmiiClockConfig, RmiiEthChipset};
    use esp_idf_svc::hal::gpio::{Gpio0, Gpio16, Gpio17, Gpio19, Gpio21, Gpio22, Gpio25};
    use esp_idf_svc::hal::gpio::{Gpio26, Gpio27, PinDriver};
    use esp_idf_svc::hal::mac::MAC;

    let sysloop = net::sysloop().map_err(|e| format!("event loop: {e}"))?;
    if r.power >= 0 {
        // The PHY (or its 50 MHz oscillator) is switched on only now, after the boot: an
        // oscillator on GPIO0 must not run while the strapping pins are read.
        // SAFETY: the pin was checked against the flash, the RMII pins and the other PHY pins.
        let mut power = PinDriver::output(unsafe { AnyOutputPin::new(r.power) })
            .map_err(|e| format!("power pin: {e}"))?;
        power.set_high().map_err(|e| format!("power pin: {e}"))?;
        // Kept high for the life of the board.
        std::mem::forget(power);
        std::thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: nothing else in the master uses the EMAC, the RMII pins or these GPIOs (checked by
    // `check_rmii`), and the thread creates them once.
    let driver = unsafe {
        let clock = match r.clock {
            1 => RmiiClockConfig::<Gpio0, Gpio16, Gpio17>::OutputGpio0(Gpio0::new()),
            2 => RmiiClockConfig::OutputGpio16(Gpio16::new()),
            3 => RmiiClockConfig::OutputInvertedGpio17(Gpio17::new()),
            _ => RmiiClockConfig::Input(Gpio0::new()),
        };
        EthDriver::new_rmii(
            MAC::new(),
            Gpio25::new(),
            Gpio26::new(),
            Gpio27::new(),
            AnyOutputPin::new(r.mdc),
            Gpio22::new(),
            Gpio21::new(),
            Gpio19::new(),
            AnyIOPin::new(r.mdio),
            clock,
            None::<AnyOutputPin>,
            RmiiEthChipset::LAN87XX,
            Some(r.phy_addr as u32),
            sysloop,
        )
    }
    .map_err(|e| format!("LAN8720 driver: {e}"))?;
    let netif = eth_netif(fixed).map_err(|e| format!("interface: {e}"))?;
    EspEth::wrap_all(driver, netif).map_err(|e| format!("interface glue: {e}"))
}

fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_le_bytes(ip.octets())
}

fn worker() {
    let first = lock().clone();
    let checked = match first.rmii {
        Some(r) => check_rmii(CHIP, r),
        None => check_pins(CHIP, first.pins),
    };
    if let Err(text) = checked {
        note!("{text}");
        // Not started: the next "settings complete" (a module restart) may bring other pins.
        STARTED.store(false, Ordering::Release);
        return;
    }
    let fixed = fixed_of(&first);
    let created = match first.rmii {
        #[cfg(target_arch = "xtensa")]
        Some(r) => create_rmii(r, fixed).map(|eth| run(eth, first, fixed)),
        // `check_rmii` has refused LAN8720 on a chip without the EMAC.
        #[cfg(not(target_arch = "xtensa"))]
        Some(_) => Ok(()),
        None => create_spi(&first, fixed).map(|eth| run(eth, first, fixed)),
    };
    if let Err(text) = created {
        note!("cannot start Ethernet: {text}");
        // The bus stays taken (see `create_spi`): no second try before a reset.
    }
}

/// Starts the interface, then follows the settings and the link for the life of the board.
fn run<T>(mut eth: EspEth<'static, T>, first: Wanted, mut fixed: Option<Fixed>) {
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
            let hardware = (wanted.pins, wanted.mac, wanted.rmii);
            if hardware != (first.pins, first.mac, first.rmii) && !told_pins {
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
