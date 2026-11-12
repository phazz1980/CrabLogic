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
        self.update_board_check_now();
        let board_lines = self
            .board_check
            .as_ref()
            .map(|c| c.lines.clone())
            .unwrap_or_default();
        for text in board_lines {
            self.info(LogLevel::Warning, text);
        }
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
            if job.background && self.queued.is_none() {
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
        if doc.report.has_errors() {
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

    /// Lines of the board check for the current schema (empty when everything goes accelerated
    /// or nothing is known about the board).
    fn compute_board_check(&self) -> BoardCheck {
        let doc = self.doc.as_ref().expect("the caller checked the document");
        let work_dir = doc.project.work_dir();
        let memo = crab_build::board_memo::load(&work_dir);
        let mut check = BoardCheck {
            revision: doc.revision,
            project: doc.project.path.clone(),
            memo_time: memo.as_ref().map(|(_, t)| *t),
            lines: Vec::new(),
        };
        let Some((memo, time)) = memo else {
            return check;
        };
        if doc.report.errors().next().is_some() {
            return check;
        }
        let project = doc.project.with_local(&self.lib, &self.local);
        // A build error is not repeated here: the schema check reports it.
        let Ok((project, modules, numbers)) = crate::job::build_sheets(&project, &self.lib, false)
        else {
            return check;
        };
        let sheets = crate::job::sheet_modules(&project, &modules, &numbers);
        let cache = work_dir.join("aot");
        match crab_build::upload_plan::preview(&memo, &sheets, &cache) {
            Ok(demoted) => {
                for d in demoted
                    .iter()
                    .filter(|d| d.reason == crab_build::placement::DemoteReason::NoSpace)
                {
                    let mut line = crate::i18n::fmt(
                        "Лист {0} («{1}») не поместится ускоренным: шаг ≈ {2} мкс вместо ≈ {3} мкс",
                        &[&d.sheet, &d.name, &d.step_us, &d.fast_step_us],
                    );
                    if d.exceeds_period {
                        line.push_str(crate::i18n::tr(
                            " — длиннее периода, таймеры и импульсы будут неточными",
                        ));
                    }
                    check.lines.push(line);
                }
            }
            Err(e) => check.lines.push(e.to_string()),
        }
        if !check.lines.is_empty() {
            let date = chrono::DateTime::<chrono::Local>::from(time).format("%d.%m.%Y %H:%M");
            check.lines.push(crate::i18n::fmt(
                "По данным платы с последней заливки модулей ({0}). Как исправить: удалите ненужные модули с платы, разделите лист или поставьте ему скорость «Фоновая».",
                &[&date],
            ));
        }
        check
    }

    /// Keeps `board_check` fresh: after a second of quiet since the last edit, and when the
    /// memo file changed (a module upload finished).
    pub(super) fn update_board_check(&mut self, ctx: &egui::Context) {
        const PAUSE: Duration = Duration::from_secs(1);
        let Some(doc) = &self.doc else {
            self.board_check = None;
            self.board_seen = None;
            return;
        };
        let now = Instant::now();
        let key = (doc.revision, doc.project.path.clone());
        if self.board_seen.as_ref() != Some(&key) {
            self.board_seen = Some(key);
            self.board_changed = now;
        }
        let stale = self
            .board_check
            .as_ref()
            .is_none_or(|c| c.revision != doc.revision || c.project != doc.project.path);
        if stale {
            if now - self.board_changed < PAUSE {
                ctx.request_repaint_after(PAUSE);
                return;
            }
        } else if now - self.board_polled >= PAUSE {
            self.board_polled = now;
            let file = doc.project.work_dir().join(crab_build::board_memo::FILE);
            let time = std::fs::metadata(file).and_then(|m| m.modified()).ok();
            if self
                .board_check
                .as_ref()
                .is_none_or(|c| c.memo_time == time)
            {
                return;
            }
        } else {
            return;
        }
        self.board_polled = now;
        self.board_check = Some(self.compute_board_check());
    }

    /// Recomputes `board_check` right away when it is out of date.
    fn update_board_check_now(&mut self) {
        let Some(doc) = &self.doc else { return };
        let stale = self
            .board_check
            .as_ref()
            .is_none_or(|c| c.revision != doc.revision || c.project != doc.project.path);
        if stale {
            self.board_check = Some(self.compute_board_check());
        }
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
                JobEvent::Ask(question) => self.upload_question = Some((question, false)),
            }
        }
        if done {
            self.job = None;
            self.upload_question = None;
            if let Some(kind) = self.queued.take() {
                self.start_job(kind, ctx);
            }
        }
        if let Some(port) = connect {
            self.connect_debug(Some(port), ctx);
        }
    }

    /// One question for the upload when some sheets go without acceleration
    /// (`specs/wasm-kernel`, decision 16).
    pub(super) fn upload_question_dialog(&mut self, ctx: &egui::Context) {
        let Some((question, never_ask)) = &mut self.upload_question else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("upload_question")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            let stop = question.kind == QuestionKind::Stop;
            ui.heading(crate::i18n::tr(if stop {
                "Листы остановятся на время заливки"
            } else {
                "Не все листы будут ускорены"
            }));
            ui.add_space(4.0);
            ui.label(&question.message);
            ui.add_space(8.0);
            if !stop {
                ui.checkbox(
                    never_ask,
                    crate::i18n::tr("Больше не спрашивать для этих листов (скорость «Фоновая»)"),
                );
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                let go = if stop {
                    "Залить"
                } else {
                    "Залить так"
                };
                if ui.button(crate::i18n::tr(go)).clicked() {
                    answer = Some(true);
                }
                if ui.button(crate::i18n::tr("Отмена")).clicked() {
                    answer = Some(false);
                }
            });
        });
        let Some(upload) = answer else { return };
        let Some((question, never_ask)) = self.upload_question.take() else {
            return;
        };
        if upload && never_ask && question.kind == QuestionKind::Demote {
            self.make_sheets_background(&question.sheets);
        }
        if let Some(job) = &self.job {
            job.answer(upload);
        }
    }

    /// «Do not ask again»: the sheets get the background speed, so the next upload puts them
    /// without acceleration silently.
    fn make_sheets_background(&mut self, sheets: &[u32]) {
        let Some(doc) = self.doc.as_mut() else { return };
        let result = doc.edit(&self.lib, None, |p| {
            for &number in sheets {
                match p.logic.sheets.iter_mut().find(|s| s.number == number) {
                    Some(sheet) => sheet.speed = crab_core::SheetSpeed::Background,
                    None => p.logic.sheets.push(crab_core::SheetExecution {
                        number,
                        speed: crab_core::SheetSpeed::Background,
                    }),
                }
            }
            Ok::<_, String>(())
        });
        if let Err(e) = result {
            self.info(LogLevel::Error, e);
        }
    }
}
