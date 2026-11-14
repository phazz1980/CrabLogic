use super::*;

/// The text of a link for the upload job and the debugger: `COM10` or `tcp:192.168.0.100`.
pub(super) fn link_text(link: &crab_build::Chosen) -> String {
    match link {
        crab_build::Chosen::Serial(port) => port.clone(),
        crab_build::Chosen::Net(host) => format!("tcp:{host}"),
    }
}

impl EditorApp {
    /// Network address of the board kept in the project (`[board] host`).
    pub fn board_host(&self) -> Option<String> {
        let host = self.doc.as_ref()?.project.config.board.host.as_deref()?;
        let host = host.trim();
        (!host.is_empty()).then(|| host.to_owned())
    }

    /// The link the upload and the debugging use now (USB is more important than the network):
    /// the toolbar port or the project's one if it is among the ports present, else the project's
    /// address, see [`crab_build::choose_link`]. Uses the last scan of the ports.
    pub fn effective_link(&self) -> Result<crab_build::Chosen, String> {
        let doc = self
            .doc
            .as_ref()
            .ok_or_else(|| crate::i18n::tr("проект не открыт").to_owned())?;
        let mut board = doc.project.config.board.clone();
        if let Some(port) = self.port_choice() {
            board.port = Some(port);
        }
        crab_build::choose_link_auto(&board, &self.ports)
    }

    /// [`Self::effective_link`] as text for a job, after a fresh look at the serial ports (a cable
    /// may have been plugged or pulled since the last scan).
    pub(super) fn link_choice(&mut self) -> Result<String, String> {
        if let Ok(ports) = crab_build::serial_ports() {
            self.ports = ports;
        }
        self.effective_link().map(|l| link_text(&l))
    }

    /// Opens the window to type the board address.
    pub fn open_net_input(&mut self) {
        self.net_input = Some(self.board_host().unwrap_or_default());
    }

    /// Writes the toolbar COM port to `[board] port`; the saved address stays.
    pub fn target_changed(&mut self) {
        let port = self.port_choice();
        let Some(doc) = &mut self.doc else { return };
        if doc.project.config.board.port == port {
            return;
        }
        let lib = self.lib.clone();
        let _ = doc.edit(&lib, Some("board_port".into()), |p| {
            p.config.board.port = port;
            Ok::<(), String>(())
        });
    }

    /// Sets (or with `None` removes) `[board] host`; the saved port stays.
    pub fn set_board_host(&mut self, host: Option<String>) {
        let Some(doc) = &mut self.doc else { return };
        if doc.project.config.board.host == host {
            return;
        }
        let lib = self.lib.clone();
        let _ = doc.edit(&lib, Some("board_host".into()), |p| {
            p.config.board.host = host;
            Ok::<(), String>(())
        });
    }

    /// Small window to type the board address.
    pub(super) fn net_window(&mut self, ctx: &egui::Context) {
        let Some(text) = &mut self.net_input else {
            return;
        };
        let mut apply = false;
        let mut close = false;
        egui::Window::new(crate::i18n::tr("Плата по сети"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(crate::i18n::tr(
                    "Адрес платы: IP или IP:порт (по умолчанию порт 4040). Нужен, когда кабель не подключён: по USB работа идёт в первую очередь",
                ));
                let edit = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text("192.168.1.50")
                        .desired_width(220.0),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.weak(crate::i18n::tr(
                    "Нужен пароль проекта и ключ на плате (Проект → Записать ключ для заливки по сети)",
                ));
                ui.horizontal(|ui| {
                    let ok = !text.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("OK")).clicked() || (enter && ok) {
                        apply = true;
                    }
                    if ui.button(crate::i18n::tr("Отмена")).clicked() {
                        close = true;
                    }
                });
            });
        if apply {
            let address = text.trim();
            let address = address.strip_prefix("tcp:").unwrap_or(address).trim();
            let address = address.to_owned();
            self.net_input = None;
            self.set_board_host(Some(address));
        } else if close {
            self.net_input = None;
        }
    }

    /// Writes the network key of the project password to the board over USB.
    pub fn write_net_key(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        let password = doc.project.config.project.password.clone();
        if password.is_empty() {
            self.info(
                LogLevel::Error,
                crate::i18n::tr(
                    "У проекта нет пароля: заливка по сети требует пароль. Задайте его в меню Проект → Проект в памяти платы → Пароль",
                ),
            );
            return;
        }
        if self.net_key_rx.is_some() {
            return;
        }
        let Some(port) = self.usb_port() else {
            self.info(
                LogLevel::Error,
                crate::i18n::tr("Выберите COM-порт платы на панели инструментов"),
            );
            return;
        };
        // The port is free for the key: debugging and the monitor release it.
        self.stop_debug();
        self.console.release();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let name = port.clone();
        thread::spawn(move || {
            let result = (|| {
                let mut serial = serialport::new(&port, 115_200)
                    .timeout(Duration::from_millis(200))
                    .open()
                    .map_err(|e| crate::i18n::fmt("порт {0} не открыт: {1}", &[&port, &e]))?;
                // Do not reset the board by opening the port.
                let _ = serial.write_data_terminal_ready(false);
                let _ = serial.write_request_to_send(false);
                crab_build::net_link::set_net_key(&mut serial, Some(&password))
                    .map_err(|e| e.to_string())
            })();
            let _ = tx.send(result.map(|()| port));
            ctx.request_repaint();
        });
        self.net_key_rx = Some(rx);
        self.info(
            LogLevel::Info,
            crate::i18n::fmt("Запись ключа для заливки по сети на {0}...", &[&name]),
        );
    }

    pub(super) fn poll_net_key(&mut self) {
        let Some(rx) = &self.net_key_rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.net_key_rx = None;
        match result {
            Ok(port) => self.info(
                LogLevel::Ok,
                crate::i18n::fmt(
                    "Ключ записан на плату {0}: заливка по сети с паролем проекта включена",
                    &[&port],
                ),
            ),
            Err(e) => self.info(
                LogLevel::Error,
                crate::i18n::fmt("Ключ не записан: {0}", &[&e]),
            ),
        }
    }
}
