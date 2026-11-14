//! Putting CrabLogic onto a board: the prebuilt Rust master firmware (`flash`), the WASM modules of
//! the sheets (`mcu_module`, `upload_plan`, `placement`, `wamrc`) and the serial ports they go through.

use std::sync::atomic::{AtomicBool, Ordering};

use serialport::SerialPortType;

pub mod aot_xtensa;
pub mod board_memo;
pub mod debug_link;
pub mod flash;
pub mod mcu_module;
pub mod net_link;
pub mod placement;
pub mod upload_plan;
pub mod wamrc;

pub use debug_link::{choose_link, choose_link_auto, debug_modules, project_link, Chosen};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("отменено")]
    Cancelled,

    #[error(
        "плата не найдена: нет ни одного последовательного порта. \
         Подключите плату по USB или укажите порт через --port"
    )]
    NoPort,

    #[error(
        "найдено несколько портов ({}); укажите нужный через --port \
         или в файле проекта ([board] port = \"COM3\")",
        .0.join(", ")
    )]
    AmbiguousPort(Vec<String>),

    #[error("не удалось получить список портов: {0}")]
    Ports(String),
}

/// Serial port of the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPort {
    pub address: String,
    /// Product name of the USB adapter, or the port label.
    pub description: String,
    /// USB serial adapter (has VID/PID), unlike built-in COM ports.
    pub usb: bool,
}

/// Lists the serial ports of the computer, in natural order (`COM3` before `COM10`).
pub fn serial_ports() -> Result<Vec<SerialPort>, BuildError> {
    let mut ports: Vec<SerialPort> = serialport::available_ports()
        .map_err(|e| BuildError::Ports(e.to_string()))?
        .into_iter()
        .map(|p| {
            let (description, usb) = match p.port_type {
                SerialPortType::UsbPort(info) => {
                    (info.product.unwrap_or_else(|| "USB".to_owned()), true)
                }
                SerialPortType::PciPort => ("PCI".to_owned(), false),
                SerialPortType::BluetoothPort => ("Bluetooth".to_owned(), false),
                SerialPortType::Unknown => (String::new(), false),
            };
            SerialPort {
                address: p.port_name,
                description,
                usb,
            }
        })
        .collect();
    ports.sort_by(|a, b| port_order(&a.address, &b.address));
    Ok(ports)
}

/// Picks the port for automatic upload: the only USB port, otherwise the only port at all.
pub fn auto_port(ports: &[SerialPort]) -> Result<String, BuildError> {
    if ports.is_empty() {
        return Err(BuildError::NoPort);
    }
    let tiers: [&dyn Fn(&SerialPort) -> bool; 2] = [&|p| p.usb, &|_| true];
    for tier in tiers {
        let found: Vec<&SerialPort> = ports.iter().filter(|p| tier(p)).collect();
        match found.len() {
            0 => continue,
            1 => return Ok(found[0].address.clone()),
            _ => {
                return Err(BuildError::AmbiguousPort(
                    found.iter().map(|p| p.address.clone()).collect(),
                ))
            }
        }
    }
    unreachable!("the last tier matches every port")
}

/// Port for upload: explicit, from the project settings, or the only serial port found.
pub fn choose_port(
    explicit: Option<&str>,
    board: &crab_core::BoardConfig,
) -> Result<String, BuildError> {
    if let Some(port) = explicit.or(board.port.as_deref()) {
        return Ok(port.to_owned());
    }
    auto_port(&serial_ports()?)
}

/// Orders port names naturally: `COM3` < `COM10`.
fn port_order(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(s: &str) -> (&str, Option<u64>) {
        let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (head, tail) = s.split_at(s.len() - digits);
        (head, tail.parse().ok())
    }
    split(a).cmp(&split(b)).then_with(|| a.cmp(b))
}

/// Shared control of a running upload: cancellation, set from another thread (the editor UI).
#[derive(Debug, Default)]
pub struct BuildControl {
    cancel: AtomicBool,
}

impl BuildControl {
    /// Asks the job to stop; it ends with [`BuildError::Cancelled`] or an error soon after.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The cancel flag itself, for code that polls a plain `AtomicBool` (the `wamrc` download).
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(address: &str, usb: bool) -> SerialPort {
        SerialPort {
            address: address.into(),
            description: String::new(),
            usb,
        }
    }

    #[test]
    fn auto_port_prefers_the_only_usb_port() {
        let ports = [port("COM1", false), port("COM5", true)];
        assert_eq!(auto_port(&ports).unwrap(), "COM5");
        assert!(matches!(auto_port(&[]), Err(BuildError::NoPort)));
        let two = [port("COM5", true), port("COM6", true)];
        assert!(matches!(auto_port(&two), Err(BuildError::AmbiguousPort(_))));
        assert_eq!(auto_port(&[port("COM1", false)]).unwrap(), "COM1");
    }

    #[test]
    fn ports_sort_naturally() {
        let mut names = ["COM10", "COM3", "COM2"];
        names.sort_by(|a, b| port_order(a, b));
        assert_eq!(names, ["COM2", "COM3", "COM10"]);
    }
}
