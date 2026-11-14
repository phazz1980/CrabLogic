//! What the online debugging needs from the build side: the map of the module images to the
//! debug table and the choice of the link (USB or network) by the project and the ports present.

use std::path::Path;

use crab_codegen::Program;
use crab_core::{BoardConfig, PortRef};
use crab_debug::{DebugTable, ModuleMap};

/// The link to the board picked for the work now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// A serial port (`COM10`).
    Serial(String),
    /// A network address (`192.168.0.100` or `host:4040`).
    Net(String),
}

/// USB is more important than the network (`specs/ota`, answer 4): a saved port that is present
/// goes by USB; without it the saved address goes by network; a saved port that is gone with no
/// address is still the USB choice (its error is shown, the network is never taken silently);
/// with nothing saved the only present port is used. `ports` are the names of the serial ports
/// present now.
pub fn choose_link(board: &BoardConfig, ports: &[String]) -> Result<Chosen, String> {
    let port = board
        .port
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let host = board
        .host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty());
    if let Some(port) = port {
        if let Some(present) = ports.iter().find(|p| p.eq_ignore_ascii_case(port)) {
            return Ok(Chosen::Serial(present.clone()));
        }
    }
    if let Some(host) = host {
        return Ok(Chosen::Net(host.to_owned()));
    }
    if let Some(port) = port {
        return Ok(Chosen::Serial(port.to_owned()));
    }
    match ports {
        [only] => Ok(Chosen::Serial(only.clone())),
        [] => Err("плата не подключена: нет ни одного COM-порта и не задан адрес платы по сети".into()),
        many => Err(format!(
            "плата не выбрана: найдено несколько портов ({}); выберите порт или задайте адрес платы",
            many.join(", ")
        )),
    }
}

/// [`choose_link`] for the ports of the computer, with the automatic port of the build side when
/// the project names neither a port nor an address (the only USB port, otherwise the only port).
pub fn choose_link_auto(
    board: &BoardConfig,
    ports: &[crate::SerialPort],
) -> Result<Chosen, String> {
    let names: Vec<String> = ports.iter().map(|p| p.address.clone()).collect();
    let unset = |v: &Option<String>| v.as_deref().is_none_or(|t| t.trim().is_empty());
    if unset(&board.port) && unset(&board.host) {
        if let Ok(port) = crate::auto_port(ports) {
            return Ok(Chosen::Serial(port));
        }
    }
    choose_link(board, &names)
}

/// The link of the project by the serial ports present now (the command line, the AI).
pub fn project_link(board: &BoardConfig) -> Result<Chosen, String> {
    let ports = crate::serial_ports().map_err(|e| e.to_string())?;
    choose_link_auto(board, &ports)
}

/// Maps the module images of the program (built as the debugger needs them) to the entries of the
/// debug table: the CRC the board reports, the CRC of its AOT image if the board memo knows it,
/// and the table slots of the outputs each module watches.
pub fn debug_modules(
    program: &Program,
    table: &DebugTable,
    work_dir: &Path,
) -> Result<Vec<ModuleMap>, String> {
    let memo = crate::board_memo::load(work_dir).map(|(m, _)| m);
    let mut modules = Vec::new();
    for sheet in program.sheets() {
        let built = program
            .wasm_module_with(sheet, true)
            .map_err(|e| format!("лист {sheet}: {e}"))?;
        let slots = built
            .watched
            .iter()
            .map(|label| {
                let port = label.parse::<PortRef>().ok()?;
                table.output_index(&port)
            })
            .collect();
        let crc = crate::mcu_module::crc32(&built.wasm);
        // An AOT image is what the board reports a CRC of.
        let image_crc = memo
            .as_ref()
            .and_then(|m| m.image_crc(&built.name, crc))
            .filter(|&c| c != crc);
        modules.push(ModuleMap {
            crc,
            image_crc,
            sheet,
            name: built.name,
            slots,
        });
    }
    Ok(modules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(port: Option<&str>, host: Option<&str>) -> BoardConfig {
        BoardConfig {
            fqbn: "esp32:esp32:esp32".into(),
            model: None,
            port: port.map(str::to_owned),
            host: host.map(str::to_owned),
            options: Default::default(),
        }
    }

    fn ports(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn saved_port_present_wins_over_host() {
        let b = board(Some("COM10"), Some("192.168.0.100"));
        assert_eq!(
            choose_link(&b, &ports(&["COM3", "COM10"])),
            Ok(Chosen::Serial("COM10".into()))
        );
    }

    #[test]
    fn saved_port_gone_goes_by_network() {
        let b = board(Some("COM10"), Some("192.168.0.100"));
        assert_eq!(
            choose_link(&b, &ports(&["COM3"])),
            Ok(Chosen::Net("192.168.0.100".into()))
        );
        assert_eq!(
            choose_link(&b, &[]),
            Ok(Chosen::Net("192.168.0.100".into()))
        );
    }

    #[test]
    fn host_only_goes_by_network_even_with_ports() {
        let b = board(None, Some("board.local"));
        assert_eq!(
            choose_link(&b, &ports(&["COM3"])),
            Ok(Chosen::Net("board.local".into()))
        );
    }

    #[test]
    fn saved_port_gone_without_host_stays_usb() {
        let b = board(Some("COM10"), None);
        assert_eq!(
            choose_link(&b, &ports(&["COM3"])),
            Ok(Chosen::Serial("COM10".into()))
        );
        assert_eq!(choose_link(&b, &[]), Ok(Chosen::Serial("COM10".into())));
    }

    #[test]
    fn nothing_saved_takes_the_only_port() {
        let b = board(None, None);
        assert_eq!(
            choose_link(&b, &ports(&["COM4"])),
            Ok(Chosen::Serial("COM4".into()))
        );
    }

    #[test]
    fn nothing_saved_and_no_single_port_is_an_error() {
        let b = board(None, None);
        let none = choose_link(&b, &[]).unwrap_err();
        assert!(none.starts_with("плата не подключена"), "{none}");
        let many = choose_link(&b, &ports(&["COM3", "COM4"])).unwrap_err();
        assert!(many.contains("COM3, COM4"), "{many}");
    }

    #[test]
    fn blank_values_count_as_unset() {
        let b = board(Some(" "), Some(""));
        assert_eq!(
            choose_link(&b, &ports(&["COM4"])),
            Ok(Chosen::Serial("COM4".into()))
        );
    }

    #[test]
    fn port_name_case_is_ignored() {
        let b = board(Some("com10"), Some("1.2.3.4"));
        assert_eq!(
            choose_link(&b, &ports(&["COM10"])),
            Ok(Chosen::Serial("COM10".into()))
        );
    }
}
