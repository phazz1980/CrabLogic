use super::*;

impl EditorApp {
    /// Looks for serial ports on a worker thread.
    pub fn scan_ports(&mut self, ctx: &egui::Context) {
        if self.port_scan.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = crab_build::serial_ports().map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.port_scan = Some(rx);
    }

    pub fn scanning_ports(&self) -> bool {
        self.port_scan.is_some()
    }

    pub(super) fn poll_ports(&mut self) {
        let Some(rx) = &self.port_scan else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.port_scan = None;
        match result {
            Ok(ports) => self.ports = ports,
            Err(e) => self.info(
                LogLevel::Warning,
                crate::i18n::msg("Порты не найдены: {0}", &[&e]),
            ),
        }
    }

    /// Uploads the modules over the link of the project (USB, else the network); nothing happens
    /// without a project, a link that cannot be chosen is shown as an error.
    pub fn upload_modules_now(&mut self, ctx: &egui::Context) {
        if self.doc.is_none() {
            return;
        }
        match self.link_choice() {
            Ok(port) => self.start_job(JobKind::UploadModules { port: Some(port) }, ctx),
            Err(e) => self.info(LogLevel::Error, e),
        }
    }

    /// Uploads the modules that report their signals to the master firmware, then connects.
    pub fn start_debug(&mut self, ctx: &egui::Context) {
        self.debug_firmware = true;
        self.upload_modules_now(ctx);
    }

    /// Connects to a board that already runs debugging firmware: by the link of the upload job
    /// (`COM10` or `tcp:host`), or by the chosen link when `link` is `None`.
    pub fn connect_debug(&mut self, link: Option<String>, ctx: &egui::Context) {
        // The network accepts one client, USB one reader: the old session lets go first.
        self.stop_debug();
        self.sim = None;
        self.console.release();
        let chosen = match link {
            Some(text) => Ok(match crab_build::net_link::BoardLink::parse(&text) {
                crab_build::net_link::BoardLink::Serial(port) => crab_build::Chosen::Serial(port),
                crab_build::net_link::BoardLink::Tcp(host) => crab_build::Chosen::Net(host),
            }),
            None => self.link_choice().and_then(|_| self.effective_link()),
        };
        let chosen = match chosen {
            Ok(chosen) => chosen,
            Err(e) => {
                self.info(LogLevel::Error, e);
                return;
            }
        };
        let Some(doc) = &self.doc else { return };
        // The same project the upload builds: with the local values (Wi-Fi password and the
        // like), or the module CRCs differ and the debugger sees a foreign module.
        let project = doc.project.with_local(&self.lib, &self.local);
        match DebugState::start(&project, &self.lib, &chosen, ctx) {
            Ok(state) => {
                self.info(
                    LogLevel::Info,
                    crate::i18n::msg("Отладка: подключение к {0}", &[&state.port]),
                );
                self.debug = Some(state);
            }
            Err(e) => self.info(LogLevel::Error, e),
        }
    }

    /// Serial port of the board when the link now goes by USB; `None` for the network.
    pub(super) fn board_port(&self) -> Option<String> {
        match self.effective_link() {
            Ok(crab_build::Chosen::Serial(port)) => Some(port),
            _ => None,
        }
    }

    /// Serial port for what only USB can do (writing the network key): the toolbar port, the
    /// project's one or the detected one, whatever the saved address is.
    pub(super) fn usb_port(&self) -> Option<String> {
        self.port_choice()
            .or_else(|| self.doc.as_ref()?.project.config.board.port.clone())
            .or_else(|| crab_build::auto_port(&self.ports).ok())
    }

    /// Keeps the port monitor in step with debugging and uploads.
    pub(super) fn poll_console(&mut self, ctx: &egui::Context) {
        if self.console.hold && !self.ai.busy() {
            self.console.hold = false;
        }
        let busy = self.console.hold
            || self.report.building()
            || self.job.as_ref().is_some_and(|j| j.uses_port);
        let port = self.board_port();
        self.console.debug_reader = self.doc.as_ref().is_some_and(|doc| {
            crab_core::uart::block_uarts(&doc.project.logic, &self.lib)
                .iter()
                .any(|u| {
                    u.number == crab_core::uart::USB && u.access == crab_core::UartAccess::Read
                })
        });
        let live = self.debug.as_ref().map(|d| &d.live);
        self.console.sync(live, port, busy, ctx);
        if let Some(d) = &self.debug {
            for bytes in self.console.take_debug_input() {
                d.send(crab_debug::Command::TextIn(bytes));
            }
        }
        if self.bottom_tab == BottomTab::Console {
            self.console.unread = 0;
        }
    }

    /// Draws the bug report form; saves the report when the user chose a file.
    pub(super) fn poll_report(&mut self, ctx: &egui::Context) {
        // The form is taken out of the editor while it reads facts from the rest of it.
        let mut form = std::mem::take(&mut self.report);
        let event = form.show(ctx, &report_facts(self, form.open));
        match event {
            Some(crate::report::Event::Save(path)) => {
                // The board data needs the port: debugging and the monitor let it go.
                if form.wants_board() {
                    self.stop_debug();
                    self.console.release();
                }
                form.start(
                    path,
                    &report_facts(self, true),
                    &self.lib,
                    crate::update::CURRENT,
                    ctx,
                );
            }
            Some(crate::report::Event::Done { path, problems }) => {
                if path.as_os_str().is_empty() {
                    self.info(LogLevel::Error, crate::i18n::tr("Отчёт не сохранён"));
                } else {
                    self.info(
                        LogLevel::Ok,
                        crate::i18n::fmt("Отчёт сохранён: {0}", &[&path.display()]),
                    );
                    crate::report::reveal(&path);
                }
                for p in problems {
                    self.info(LogLevel::Warning, p);
                }
            }
            None => {}
        }
        self.report = form;
    }

    pub fn stop_debug(&mut self) {
        if let Some(d) = self.debug.take() {
            d.stop();
            self.info(LogLevel::Info, crate::i18n::tr("Отладка остановлена"));
        }
    }

    pub fn send_ai(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        // MCP is on: the request goes to the external client, not to the API.
        if let Some(endpoint) = self.mcp.endpoint() {
            let env = crate::aiconf::claude_code_env(&theme::ai_settings());
            self.ai.send_external(endpoint, env, ctx);
            return;
        }
        self.ai.send(
            &doc.project,
            &self.lib,
            self.lib_dir.clone(),
            self.project_blocks.clone(),
            self.ai_live.clone(),
            ctx,
        );
    }

    pub(super) fn poll_ai(&mut self, ctx: &egui::Context) {
        for request in self.ai.poll() {
            match request {
                crate::ai::Request::Apply(logic, layout, panel) => {
                    let lib = self.lib.clone();
                    if let Some(doc) = &mut self.doc {
                        let schema_same =
                            doc.project.logic == *logic && doc.project.layout == layout;
                        if schema_same {
                            // Only the panel changed: no rebuild of the firmware.
                            if doc.project.panel != *panel {
                                let _ = doc.edit_panel(None, |p| {
                                    *p = *panel;
                                    Ok::<_, String>(())
                                });
                            }
                        } else {
                            // One undo step for the schema and the panel together.
                            let _ = doc.edit(&lib, None, |p| {
                                p.logic = *logic;
                                p.layout = layout;
                                p.panel = *panel;
                                Ok::<_, String>(())
                            });
                        }
                    }
                    // A password set by AI goes to the program, not into the file.
                    self.take_local();
                }
                crate::ai::Request::ReloadLibrary => self.reload_library(),
                crate::ai::Request::UploadModules { port, debug } => {
                    self.debug_firmware = debug;
                    self.start_job(JobKind::UploadModules { port }, ctx);
                }
            }
        }
    }

    /// Starts the simulation of the open schema (debugging, if any, is closed).
    pub fn start_sim(&mut self) {
        let Some(doc) = &self.doc else { return };
        match crate::simview::SimState::start(&doc.project, &self.lib, doc.revision) {
            Ok(state) => {
                self.stop_debug();
                self.sim = Some(state);
            }
            Err(e) => self.info(LogLevel::Error, e),
        }
    }

    /// Keeps the simulation in step with the schema and plays the passed time.
    pub(super) fn poll_sim(&mut self, ctx: &egui::Context) {
        let (Some(sim), Some(doc)) = (&mut self.sim, &self.doc) else {
            return;
        };
        sim.follow(&doc.project, &self.lib, doc.revision);
        sim.tick(ctx);
    }

    pub(super) fn poll_debug(&mut self) {
        // A new connection status (e.g. "the module differs from the schema") goes to the
        // messages once; the status bar only shows the port.
        let note = self.debug.as_mut().and_then(|d| {
            d.refresh();
            if d.live.status == d.logged_status {
                return None;
            }
            d.logged_status = d.live.status.clone();
            let port = d.port.clone();
            (!d.logged_status.is_empty()).then(|| (port, d.logged_status.clone(), d.online()))
        });
        if let Some((port, status, online)) = note {
            let level = if online {
                LogLevel::Info
            } else {
                LogLevel::Warning
            };
            self.info(
                level,
                crate::i18n::fmt("Отладка {0}: {1}", &[&port, &status]),
            );
        }
        // The board restarted after a crash: one hint per connection.
        let crash = self.debug.as_mut().and_then(|d| {
            let reason = crash_reason(d.live.master_reset.as_deref()?)?;
            let first = !d.reset_logged;
            d.reset_logged = true;
            first.then_some(reason)
        });
        if let Some(reason) = crash {
            self.info(
                LogLevel::Warning,
                crate::i18n::fmt(
                    "Плата перезапустилась после сбоя ({0}). Сохраните отчёт об ошибке: Справка → Сообщить об ошибке.",
                    &[&crate::i18n::tr(reason)],
                ),
            );
        }
        let Some(d) = &mut self.debug else {
            *self.ai_live.lock().expect("live values") = None;
            return;
        };
        if let Some(doc) = &self.doc {
            d.check_project(&doc.project, doc.revision);
        }
        if d.same_project == Some(false) && !d.hash_asked {
            d.hash_asked = true;
            self.info(
                LogLevel::Warning,
                crate::i18n::tr("На плате другая версия проекта"),
            );
            return;
        }
        // Publish values for the AI assistant.
        let published = d.online().then(|| {
            (0..d.table.entries.len())
                .map(|i| {
                    let v = d.live.values[i].map_or("—".to_owned(), |v| v.to_string());
                    (d.name(i), v, d.live.forced[i].is_some())
                })
                .collect()
        });
        *self.ai_live.lock().expect("live values") = published;
    }
}

/// Russian name of a restart reason that means a crash; `None` for a normal restart.
fn crash_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "panic" => Some("паника"),
        "int_wdt" | "task_wdt" | "wdt" => Some("сторожевой таймер"),
        "brownout" => Some("просадка питания"),
        _ => None,
    }
}

/// What the report form shows about the editor. Borrows only the fields it reads, so the
/// form (a field of the editor) can be borrowed mutably next to it.
fn report_facts(app: &EditorApp, with_log: bool) -> crate::report::Facts<'_> {
    let doc = app.doc.as_ref();
    crate::report::Facts {
        project: doc.map(|d| &d.project),
        board_name: doc.map(|d| {
            crate::boardcfg::board_name(&d.project.config.board, app.lib.boards()).to_owned()
        }),
        port: doc.and(app.board_port()),
        recent: if with_log {
            let skip = app.log.len().saturating_sub(50);
            app.log[skip..].iter().map(|l| l.text.to_string()).collect()
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::crash_reason;

    #[test]
    fn only_crashes_give_a_reason() {
        assert_eq!(crash_reason("task_wdt"), Some("сторожевой таймер"));
        assert_eq!(crash_reason("brownout"), Some("просадка питания"));
        assert_eq!(crash_reason("panic"), Some("паника"));
        assert_eq!(crash_reason("poweron"), None);
    }
}
