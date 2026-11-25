use std::path::Path;
use std::process::ExitCode;

use crab_core::Project;

use super::common::{EXIT_BUILD_FAILED, EXIT_LOAD_FAILED};
use super::link::open_master_port;

/// `find`: prints the CrabLogic boards that answer the search in the local network.
pub(crate) fn run_find(timeout_ms: u64) -> ExitCode {
    let found = crab_debug::find::find_boards(std::time::Duration::from_millis(timeout_ms));
    if found.is_empty() {
        eprintln!("платы не найдены.");
        eprintln!("  Проверьте, что плата подключена к той же сети (Wi-Fi или Ethernet).");
        eprintln!(
            "  Брандмауэр Windows может блокировать ответы: разрешите crablogic в частной сети."
        );
        eprintln!("  Широковещание не проходит через другую подсеть и VLAN: тогда введите адрес вручную (--host).");
        return ExitCode::FAILURE;
    }
    for f in &found {
        println!("{}", find_line(f));
    }
    ExitCode::SUCCESS
}

/// One line of `find`: address, id, chip, master version, hash, key state and name.
fn find_line(f: &crab_debug::find::Found) -> String {
    let b = &f.board;
    let hash = b.hash.as_deref().unwrap_or("-");
    let key = if b.has_key {
        "ключ есть"
    } else {
        "ключа нет"
    };
    let mut line = format!(
        "{}  {}  {}  мастер {}  хеш {}  {}",
        f.ip, b.id, b.chip, b.master, hash, key
    );
    if !b.name.is_empty() {
        line.push_str("  ");
        line.push_str(&b.name);
    }
    line
}

/// `netkey`: writes the network key of the project password to the board over USB, or removes it.
pub(crate) fn run_netkey(project_dir: &Path, port: Option<String>, remove: bool) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let password = if remove {
        None
    } else {
        match Project::load(project_dir) {
            Ok(project) => Some(project.config.project.password.clone()),
            Err(e) => {
                eprintln!("ошибка: {e}");
                return ExitCode::from(EXIT_LOAD_FAILED);
            }
        }
    };
    let (mut serial, _) = match open_master_port(port) {
        Ok(opened) => opened,
        Err(message) => return failed(message),
    };
    if let Err(e) =
        crab_build::mcu_module::wait_for_master(&mut serial, std::time::Duration::from_secs(8))
    {
        return failed(e.to_string());
    }
    if let Err(e) = crab_build::net_link::set_net_key(&mut serial, password.as_deref()) {
        return failed(e.to_string());
    }
    if remove {
        println!("ключ для заливки по сети удалён с платы");
    } else {
        println!(
            "ключ для заливки по сети записан на плату (порт 4040 откроется, когда модуль поднимет Wi-Fi или Ethernet)"
        );
    }
    ExitCode::SUCCESS
}
