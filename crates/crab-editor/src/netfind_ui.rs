//! «Найти платы» in the network window («По сети…»): the search runs in a background thread and
//! the chosen board fills the address field.

use std::net::Ipv4Addr;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crab_debug::find::{find_boards, Found};
use eframe::egui;

use crate::i18n::tr;

/// Command port of the board. An address on this port is written without the port.
const COMMAND_PORT: u16 = 4040;
/// How long the search listens for replies.
const SEARCH_TIME: Duration = Duration::from_millis(1500);

/// State of the search shown in the network window.
#[derive(Default)]
pub struct NetFind {
    /// Result channel of the running search.
    running: Option<Receiver<Vec<Found>>>,
    /// Boards of the last finished search.
    found: Option<Vec<Found>>,
}

impl NetFind {
    /// Starts a search on a worker thread; does nothing while one runs.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(find_boards(SEARCH_TIME));
            ctx.request_repaint();
        });
        self.running = Some(rx);
        self.found = None;
    }

    /// Takes the result of a finished search. Call once per frame.
    pub fn poll(&mut self) {
        let Some(rx) = &self.running else { return };
        match rx.try_recv() {
            Ok(list) => {
                self.running = None;
                self.found = Some(list);
            }
            // The worker ended without an answer: report it as an empty result.
            Err(TryRecvError::Disconnected) => {
                self.running = None;
                self.found = Some(Vec::new());
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn is_running(&self) -> bool {
        self.running.is_some()
    }
}

/// The address text of a board as the network window keeps it: `192.168.1.50`, or
/// `192.168.1.50:5000` when the board does not use the command port 4040.
pub fn host_text(ip: Ipv4Addr, port: u16) -> String {
    if port == COMMAND_PORT {
        ip.to_string()
    } else {
        format!("{ip}:{port}")
    }
}

/// One row of the list without the key state: `ip · id · chip · мастер версия · name`.
pub fn row_text(found: &Found) -> String {
    let b = &found.board;
    let mut parts = vec![
        found.ip.to_string(),
        b.id.clone(),
        b.chip.clone(),
        format!("мастер {}", b.master),
    ];
    if !b.name.is_empty() {
        parts.push(b.name.clone());
    }
    parts.join(" · ")
}

/// The search part of the network window: the button, the state and the list of boards. A click
/// on a board writes its address into `host`.
pub fn section(ui: &mut egui::Ui, state: &mut NetFind, host: &mut String) {
    state.poll();
    let ctx = ui.ctx().clone();
    let running = state.is_running();
    ui.horizontal(|ui| {
        let button = ui
            .add_enabled(!running, egui::Button::new(tr("Найти платы")))
            .on_hover_text(tr(
                "Ищет платы CrabLogic в той же сети: ответ приходит с адреса платы",
            ));
        if button.clicked() {
            state.start(&ctx);
        }
        if running {
            ui.spinner();
            ui.weak(tr("Поиск…"));
        }
    });

    let Some(found) = &state.found else { return };
    if found.is_empty() {
        ui.weak(tr("Платы не найдены."));
        ui.weak(tr(
            "Проверьте, что плата подключена к той же сети (Wi-Fi или Ethernet).",
        ));
        ui.weak(tr(
            "Брандмауэр Windows может блокировать ответы: разрешите crablogic в частной сети.",
        ));
        ui.weak(tr(
            "Широковещание не проходит через другую подсеть и VLAN: тогда введите адрес вручную.",
        ));
        return;
    }
    for f in found {
        ui.horizontal(|ui| {
            if ui.selectable_label(false, row_text(f)).clicked() {
                *host = host_text(f.ip, f.board.port);
            }
            if !f.board.has_key {
                ui.colored_label(ui.visuals().warn_fg_color, tr("ключа нет"))
                    .on_hover_text(tr(
                        "На плате нет ключа для сети: её видно, но войти по сети нельзя. Запишите ключ по USB (Проект → Записать ключ для заливки по сети)",
                    ));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_netfind::Board;

    fn found(name: &str, port: u16) -> Found {
        Found {
            ip: Ipv4Addr::new(192, 168, 1, 50),
            board: Board {
                id: "a1b2c3d4e5f6".into(),
                chip: "esp32".into(),
                master: "2.0.1".into(),
                hash: None,
                has_key: true,
                port,
                name: name.into(),
            },
        }
    }

    #[test]
    fn host_text_omits_default_port() {
        assert_eq!(
            host_text(Ipv4Addr::new(192, 168, 1, 50), 4040),
            "192.168.1.50"
        );
        assert_eq!(
            host_text(Ipv4Addr::new(192, 168, 1, 50), 5000),
            "192.168.1.50:5000"
        );
    }

    #[test]
    fn row_text_with_and_without_name() {
        assert_eq!(
            row_text(&found("Насосная", 4040)),
            "192.168.1.50 · a1b2c3d4e5f6 · esp32 · мастер 2.0.1 · Насосная"
        );
        assert_eq!(
            row_text(&found("", 4040)),
            "192.168.1.50 · a1b2c3d4e5f6 · esp32 · мастер 2.0.1"
        );
    }
}
