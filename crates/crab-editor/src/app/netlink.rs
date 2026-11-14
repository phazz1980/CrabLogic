use super::*;

/// True when the target is a network address, not a serial port.
fn is_net(target: &str) -> bool {
    crab_build::net_link::looks_like_address(target)
}

impl EditorApp {
    /// The toolbar target is a network address (upload over the network).
    pub fn net_target(&self) -> bool {
        is_net(&self.port)
    }

    /// Opens the window to type the board address.
    pub fn open_net_input(&mut self) {
        self.net_input = Some(if self.net_target() {
            self.port.clone()
        } else {
            String::new()
        });
    }

    /// Keeps `[board] host` of the project in step with the toolbar target.
    pub fn target_changed(&mut self) {
        let host = is_net(&self.port).then(|| self.port.trim().to_owned());
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
                    "Адрес платы: IP или IP:порт (по умолчанию порт 4040)",
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
            let mut address = text.trim().to_owned();
            // A bare name without a dot would be taken for a serial port.
            if !is_net(&address) {
                address = format!("tcp:{address}");
            }
            self.port = address;
            self.target_changed();
            self.net_input = None;
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
        if self.net_target() {
            self.info(
                LogLevel::Error,
                crate::i18n::tr(
                    "Ключ записывается только по USB: выберите COM-порт на панели инструментов",
                ),
            );
            return;
        }
        let Some(port) = self.board_port() else {
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
