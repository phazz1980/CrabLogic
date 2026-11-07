use super::*;

impl EditorApp {
    /// Looks for serial ports with arduino-cli on a worker thread.
    pub fn scan_ports(&mut self, ctx: &egui::Context) {
        if self.port_scan.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = ArduinoCli::locate()
                .and_then(|cli| cli.serial_ports())
                .map_err(|e| e.to_string());
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

    /// Uploads the modules that report their signals to the master firmware, then connects.
    pub fn start_debug(&mut self, ctx: &egui::Context) {
        self.debug_firmware = true;
        let port = self.port_choice();
        self.start_job(JobKind::UploadModules { port }, ctx);
    }

    /// Connects to a board that already runs debugging firmware.
    pub fn connect_debug(&mut self, port: Option<String>, ctx: &egui::Context) {
        self.stop_debug();
        self.sim = None;
        self.console.release();
        let Some(doc) = &self.doc else { return };
        let Some(port) = port.or_else(|| self.board_port()) else {
            self.info(
                LogLevel::Error,
                crate::i18n::tr("Выберите порт платы на панели инструментов"),
            );
            return;
        };
        match DebugState::start(&doc.project, &self.lib, &port, ctx) {
            Ok(state) => {
                self.info(
                    LogLevel::Info,
                    crate::i18n::msg("Отладка: подключение к {0}", &[&port]),
                );
                self.debug = Some(state);
            }
            Err(e) => self.info(LogLevel::Error, e),
        }
    }

    /// Port of the board: chosen in the toolbar, set in the project, or detected.
    pub(super) fn board_port(&self) -> Option<String> {
        self.port_choice()
            .or_else(|| self.doc.as_ref()?.project.config.board.port.clone())
            .or_else(|| crab_build::auto_port(&self.ports).ok())
    }

    /// Keeps the port monitor in step with debugging and uploads.
    pub(super) fn poll_console(&mut self, ctx: &egui::Context) {
        if self.console.hold && !self.ai.busy() {
            self.console.hold = false;
        }
        let busy = self.console.hold || self.job.as_ref().is_some_and(|j| j.uses_port);
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
            self.ai_live.clone(),
            ctx,
        );
    }

    pub(super) fn poll_ai(&mut self) {
        for request in self.ai.poll() {
            match request {
                crate::ai::Request::Apply(logic, layout) => {
                    let lib = self.lib.clone();
                    if let Some(doc) = &mut self.doc {
                        let _ = doc.edit(&lib, None, |p| {
                            p.logic = *logic;
                            p.layout = layout;
                            Ok::<_, String>(())
                        });
                    }
                    // A password set by AI goes to the program, not into the file.
                    self.take_local();
                }
                crate::ai::Request::StopDebug => {
                    // The AI is about to upload: free the port until it finishes.
                    self.stop_debug();
                    self.console.release();
                    self.console.hold = true;
                }
                crate::ai::Request::ReloadLibrary => self.reload_library(),
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
        let Some(d) = &mut self.debug else {
            *self.ai_live.lock().expect("live values") = None;
            return;
        };
        d.refresh();
        if let Some(doc) = &self.doc {
            d.check_project(&doc.project, doc.revision);
        }
        if d.same_project == Some(false) && !d.hash_asked {
            d.hash_asked = true;
            let port = d.port.clone();
            self.info(
                LogLevel::Warning,
                crate::i18n::tr("На плате другая версия проекта: её можно скачать (Файл → Открыть проект с платы)"),
            );
            self.pull = Some(crate::pullview::PullWindow::new(port));
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
