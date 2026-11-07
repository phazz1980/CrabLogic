use super::*;

impl EditorApp {
    /// Runs the schema check and prints the result to the log.
    pub fn run_check(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.recheck(&self.lib);
        let mut lines = Vec::new();
        for d in &doc.report.diagnostics {
            let level = match d.severity {
                Severity::Error => LogLevel::Error,
                Severity::Warning => LogLevel::Warning,
            };
            let block = d.block.clone().or_else(|| {
                d.link
                    .and_then(|i| doc.project.logic.links.get(i))
                    .map(|l| l.to.block.clone())
            });
            lines.push(LogLine {
                level,
                text: crate::i18n::LogText::Plain(d.to_string()),
                block,
                help: Some(format!(
                    "{}#{}",
                    crate::help::ERRORS,
                    crate::help::error_anchor(d.kind)
                )),
            });
        }
        let errors = doc.report.errors().count();
        self.log.extend(lines);
        if errors == 0 {
            self.info(LogLevel::Ok, crate::i18n::tr("Проверка: ошибок нет"));
        } else {
            self.info(
                LogLevel::Error,
                crate::i18n::msg("Проверка: ошибок — {0}", &[&errors]),
            );
        }
    }

    pub fn start_job(&mut self, kind: JobKind, ctx: &egui::Context) {
        if let Some(job) = &mut self.job {
            // Upload joins the background master build: it stops being background and goes
            // on as a normal master upload.
            if let JobKind::Upload { port } = &kind {
                if job.promote(port.clone()) {
                    let title = job.title;
                    self.stop_debug();
                    self.console.release();
                    self.info(LogLevel::Info, format!("{title}..."));
                    return;
                }
            }
            // The user's build waits for precompilation, which shares the core and libraries.
            if job.background && !matches!(kind, JobKind::Precompile) && self.queued.is_none() {
                // It is raised to normal priority: the user waits for it now.
                if !job.is_cancelled() {
                    job.raise_priority();
                    self.info(
                        LogLevel::Info,
                        crate::i18n::tr("Ждём окончания фоновой сборки..."),
                    );
                }
                self.queued = Some(kind);
            }
            return;
        }
        // The upload needs the serial port that debugging or the monitor holds.
        if matches!(kind, JobKind::Upload { .. } | JobKind::UploadModules { .. }) {
            self.stop_debug();
            self.console.release();
        }
        let Some(doc) = &self.doc else { return };
        if doc.report.has_errors() && !matches!(kind, JobKind::Precompile) {
            self.run_check();
            self.info(
                LogLevel::Error,
                crate::i18n::tr("Сначала исправьте ошибки схемы"),
            );
            return;
        }
        let ctx = ctx.clone();
        let job = Job::start(
            kind,
            self.debug_firmware,
            doc.project.with_local(&self.lib, &self.local),
            doc.project.clone(),
            self.lib.clone(),
            move || ctx.request_repaint(),
        );
        if !job.background {
            self.info(LogLevel::Info, format!("{}...", job.title));
        }
        self.job = Some(job);
    }

    /// Cancels the running job together with the queued one.
    pub fn cancel_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
            self.queued = None;
        }
    }

    /// A foreground job runs; precompilation does not block build buttons.
    pub fn job_running(&self) -> bool {
        self.job.as_ref().is_some_and(|j| !j.background) || self.queued.is_some()
    }

    /// Key of the master firmware build the current project and board need.
    pub(super) fn master_key(&self) -> Option<MasterKey> {
        let doc = self.doc.as_ref()?;
        Some((
            doc.project.path.clone(),
            board_hash(&doc.project.config.board),
            crab_build::master_sketch_id(&crate::job::master_features(&doc.project, &self.lib)),
        ))
    }

    /// Builds the master firmware in the background when the project, the board (or its
    /// settings) or the embedded master sketch changed. Schema edits do not start it:
    /// the schema goes to the board as modules.
    pub(super) fn precompile(&mut self, ctx: &egui::Context) {
        const PAUSE: Duration = Duration::from_secs(15);
        // No background build while the core of the board is missing or being downloaded.
        let core_missing = self
            .doc
            .as_ref()
            .is_some_and(|d| !crab_build::cores::fqbn_available(&d.project.config.board.fqbn));
        if core_missing
            || matches!(self.cli_prompt, CliPrompt::Installing(_))
            || self.cores_conf.downloading()
        {
            return;
        }
        let Some(key) = self.master_key() else { return };
        let now = Instant::now();
        match &self.seen_rev {
            Some((seen, _)) if *seen == key => {}
            _ => self.seen_rev = Some((key.clone(), now)),
        }
        let quiet = self
            .seen_rev
            .as_ref()
            .is_some_and(|(_, t)| now - *t >= PAUSE);
        if self.job.is_some() || !quiet || self.precompiled.as_ref() == Some(&key) {
            return;
        }
        // Mark it tried even if the build fails, so a broken build is not repeated in a loop.
        self.precompiled = Some(key);
        self.start_job(JobKind::Precompile, ctx);
    }

    pub(super) fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let background = job.background;
        let cancelled = job.is_cancelled();
        let title = job.title;
        let mut done = false;
        let mut connect = None;
        for event in job.poll() {
            match event {
                JobEvent::Line(..) if background || cancelled => {}
                JobEvent::Note(..) if cancelled => {}
                JobEvent::Line(level, text) | JobEvent::Note(level, text) => self.info(level, text),
                // Precompilation of an outdated schema is cancelled quietly.
                JobEvent::Done(Err(_)) if cancelled && background => done = true,
                JobEvent::Done(Err(_)) if cancelled => {
                    self.info(
                        LogLevel::Warning,
                        crate::i18n::msg("{0}: отменено", &[&title]),
                    );
                    done = true;
                }
                JobEvent::Done(Ok(_)) if background => done = true,
                JobEvent::Done(Err(_)) if background => {
                    self.info(
                        LogLevel::Info,
                        crate::i18n::tr("Фоновая сборка не удалась; подробности покажет «Собрать»"),
                    );
                    done = true;
                }
                JobEvent::Done(Ok(result)) => {
                    self.info(LogLevel::Ok, result.message);
                    if let (true, Some(port)) = (result.debug, result.port) {
                        connect = Some(port);
                    }
                    done = true;
                }
                JobEvent::Done(Err(text)) => {
                    self.info(LogLevel::Error, crate::i18n::msg("ошибка: {0}", &[&text]));
                    done = true;
                }
            }
        }
        if done {
            self.job = None;
            if let Some(kind) = self.queued.take() {
                self.start_job(kind, ctx);
            }
        }
        if let Some(port) = connect {
            self.connect_debug(Some(port), ctx);
        }
    }
}
