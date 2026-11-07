use super::*;

impl EditorApp {
    /// Asks GitHub for the latest release on a worker thread. A manual check
    /// reports every outcome in the log, an automatic one only news and failures.
    pub(super) fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update_rx.is_some() {
            return;
        }
        self.update_checked = Instant::now();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let repo = theme::update_repo();
        let beta = self.update_beta;
        thread::spawn(move || {
            let _ = tx.send((manual, update::check(&repo, beta)));
            ctx.request_repaint();
        });
        self.update_rx = Some(rx);
    }

    /// Offer to install arduino-cli (download) when it is not found; shows the progress.
    pub(super) fn cli_window(&mut self, ctx: &egui::Context) {
        match &self.cli_prompt {
            CliPrompt::Hidden => {}
            CliPrompt::Ask | CliPrompt::AskCore => {
                use crab_build::cores;
                let core_only = matches!(self.cli_prompt, CliPrompt::AskCore);
                let mut install = false;
                let mut later = false;
                let title = if core_only {
                    crate::i18n::tr("Ядро ESP32 не найдено")
                } else {
                    crate::i18n::tr("arduino-cli не найден")
                };
                let accent = ctx.global_style().visuals.selection.stroke.color;
                let frame = egui::Frame::window(&ctx.global_style())
                    .stroke(egui::Stroke::new(2.5, accent))
                    .inner_margin(egui::Margin::same(18))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 28,
                        spread: 6,
                        color: egui::Color32::from_black_alpha(160),
                    });
                // Chips other than ESP32 are unpacked on top of the base pack.
                let base_missing = !cores::PACKS.first().is_some_and(cores::is_installed);
                let others_chosen = self
                    .cli_choice
                    .iter()
                    .enumerate()
                    .skip(1)
                    .any(|(i, c)| *c && !cores::PACKS.get(i).is_some_and(cores::is_installed));
                let forced_base = base_missing && others_chosen;
                let mut picked: Vec<usize> = Vec::new();
                for (i, pack) in cores::PACKS.iter().enumerate() {
                    let on = self.cli_choice.get(i).copied().unwrap_or(false);
                    if !cores::is_installed(pack) && (on || (i == 0 && forced_base)) {
                        picked.push(i);
                    }
                }
                egui::Window::new(egui::RichText::new(title).strong().size(18.0))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(frame)
                    .show(ctx, |ui| {
                        ui.set_max_width(460.0);
                        if core_only {
                            ui.label(crate::i18n::tr("Для сборки и заливки прошивки нужно ядро ESP32 для arduino-cli."));
                        } else {
                            ui.label(crate::i18n::tr("Для сборки и заливки прошивки нужна программа arduino-cli."));
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(crate::i18n::tr("Выберите, какие ядра скачать:"))
                                .strong(),
                        );
                        ui.add_space(4.0);
                        for (i, pack) in cores::PACKS.iter().enumerate() {
                            let have = cores::is_installed(pack);
                            ui.horizontal(|ui| {
                                if have {
                                    let mut yes = true;
                                    ui.add_enabled(false, egui::Checkbox::new(&mut yes, pack.title));
                                    ui.weak(crate::i18n::tr("уже установлено"));
                                } else {
                                    let forced = i == 0 && forced_base;
                                    let mut on = forced
                                        || self.cli_choice.get(i).copied().unwrap_or(false);
                                    let label = crate::i18n::fmt(
                                        "{0} — {1} МБ",
                                        &[&pack.title, &pack.size_mb],
                                    );
                                    let r = ui.add_enabled(!forced, egui::Checkbox::new(&mut on, label));
                                    if r.changed() {
                                        if let Some(c) = self.cli_choice.get_mut(i) {
                                            *c = on;
                                        }
                                    }
                                    if forced {
                                        ui.weak(crate::i18n::tr("нужно для выбранных чипов"));
                                    }
                                }
                            });
                        }
                        ui.add_space(4.0);
                        let total_mb: u32 = picked
                            .iter()
                            .filter_map(|&i| cores::PACKS.get(i))
                            .map(|p| p.size_mb)
                            .sum();
                        ui.label(
                            egui::RichText::new(crate::i18n::fmt(
                                "Будет скачано: {0} МБ",
                                &[&total_mb],
                            ))
                            .strong(),
                        );
                        ui.add_space(4.0);
                        ui.weak(crate::i18n::tr(
                            "Ядра загружаются в папку программы. Остальные можно скачать позже: Файл → Ядра ESP32.",
                        ));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let can = !picked.is_empty() || !core_only;
                            let go = egui::Button::new(
                                egui::RichText::new(crate::i18n::tr("Установить"))
                                    .strong()
                                    .size(15.0),
                            )
                            .fill(ui.visuals().selection.bg_fill)
                            .min_size(egui::vec2(120.0, 30.0));
                            install = ui.add_enabled(can, go).clicked();
                            later = ui
                                .add(egui::Button::new(crate::i18n::tr("Позже")).min_size(egui::vec2(80.0, 30.0)))
                                .clicked();
                        });
                    });
                if install {
                    let chips: Vec<&'static cores::CorePack> =
                        picked.iter().filter_map(|&i| cores::PACKS.get(i)).collect();
                    let (tx, rx) = mpsc::channel();
                    let ctx = ctx.clone();
                    let state = CliInstall {
                        rx,
                        done: Arc::new(AtomicU64::new(0)),
                        total: Arc::new(AtomicU64::new(0)),
                        cancel: Arc::new(AtomicBool::new(false)),
                        last: Arc::new(Mutex::new(String::new())),
                    };
                    let (d, t, c, last) = (
                        state.done.clone(),
                        state.total.clone(),
                        state.cancel.clone(),
                        state.last.clone(),
                    );
                    thread::spawn(move || {
                        let mut lines = Vec::new();
                        let repaint = ctx.clone();
                        let result = crab_build::install_toolchain(
                            &chips,
                            &c,
                            &mut |l| {
                                lines.push(l.to_owned());
                                *last.lock().unwrap() = l.to_owned();
                                repaint.request_repaint();
                            },
                            &mut |got, all| {
                                d.store(got, Ordering::Relaxed);
                                t.store(all, Ordering::Relaxed);
                                ctx.request_repaint();
                            },
                        );
                        let text = lines.join("\n");
                        let _ = tx.send(match result {
                            Ok(()) => Ok(text),
                            Err(e) => Err(format!("{e}\n{text}")),
                        });
                    });
                    self.info(
                        LogLevel::Info,
                        crate::i18n::tr("Скачиваю arduino-cli и ядро ESP32..."),
                    );
                    self.cli_prompt = CliPrompt::Installing(state);
                } else if later {
                    self.cli_prompt = CliPrompt::Hidden;
                }
            }
            CliPrompt::Installing(job) => {
                let done = job.rx.try_recv().ok();
                egui::Window::new(crate::i18n::tr("Установка arduino-cli"))
                    .collapsible(false)
                    .resizable(false)
                    .default_width(420.0)
                    .show(ctx, |ui| {
                        let got = job.done.load(Ordering::Relaxed);
                        let all = job.total.load(Ordering::Relaxed);
                        if all > 0 {
                            ui.add(
                                egui::ProgressBar::new(got as f32 / all as f32).text(format!(
                                    "{} / {} МБ",
                                    got >> 20,
                                    all >> 20
                                )),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                panels::busy(ui);
                                ui.label(crate::i18n::tr("Идёт установка..."));
                            });
                        }
                        ui.weak(job.last.lock().unwrap().as_str());
                        if ui.button(crate::i18n::tr("Отмена")).clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                ctx.request_repaint_after(Duration::from_millis(300));
                if let Some(result) = done {
                    self.cli_prompt = CliPrompt::Hidden;
                    match result {
                        Ok(_) => {
                            self.info(LogLevel::Info, crate::i18n::tr("arduino-cli установлен"));
                            self.scan_ports(ctx);
                        }
                        Err(e) => self.info(
                            LogLevel::Error,
                            crate::i18n::msg("Не удалось установить arduino-cli: {0}", &[&e]),
                        ),
                    }
                }
            }
        }
    }

    pub(super) fn start_self_update(&mut self, url: String, auto: bool) {
        let (done, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (tx, rx) = mpsc::channel();
        let (d, t) = (done.clone(), total.clone());
        thread::spawn(move || {
            let _ = tx.send(selfupdate::download_and_stage(&url, &d, &t));
        });
        self.self_update = SelfUpdate::Downloading {
            done,
            total,
            rx,
            auto,
        };
    }

    /// Starts the background download of the release found earlier, if it can be installed.
    pub(super) fn start_known_release_download(&mut self) {
        let Some(release) = &self.new_release else {
            return;
        };
        if selfupdate::install_dir().is_some() && matches!(self.self_update, SelfUpdate::Idle(_)) {
            let url = release.url.clone();
            self.start_self_update(url, true);
        }
    }

    pub(super) fn poll_self_update(&mut self, ctx: &egui::Context) {
        let SelfUpdate::Downloading { rx, auto, .. } = &self.self_update else {
            return;
        };
        let auto = *auto;
        ctx.request_repaint_after(Duration::from_millis(200));
        let Ok(result) = rx.try_recv() else { return };
        self.self_update = SelfUpdate::Idle(None);
        let staged = match result {
            Ok(staged) => staged,
            Err(e) => {
                if auto {
                    self.info(
                        LogLevel::Warning,
                        crate::i18n::msg("Автоскачивание обновления: {0}", &[&e]),
                    );
                }
                self.self_update = SelfUpdate::Idle(Some(e));
                return;
            }
        };
        if auto {
            let version = self.new_release.as_ref().map_or("", |r| r.version.as_str());
            let text = crate::i18n::fmt(
                "Обновление {0} скачано: Справка → «Перезапустить и обновить»",
                &[&version],
            );
            self.info(LogLevel::Info, text);
            self.self_update = SelfUpdate::Ready(staged);
            return;
        }
        self.apply_update(ctx, &staged);
    }

    /// Hands the unpacked release to the helper and closes the editor.
    pub(super) fn apply_update(&mut self, ctx: &egui::Context, staged: &Path) {
        // The new version opens the project again from its file: save it first.
        if self.doc.as_ref().is_some_and(|d| d.dirty) && !self.save() {
            self.self_update = SelfUpdate::Idle(Some(
                crate::i18n::tr("проект не сохранён, обновление отменено").into(),
            ));
            return;
        }
        let project = self.doc.as_ref().map(|d| d.project.path.clone());
        match selfupdate::launch_helper(staged, project.as_deref()) {
            Ok(()) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.self_update = SelfUpdate::Idle(Some(e)),
        }
    }

    pub(super) fn poll_update(&mut self) {
        let Some(rx) = &self.update_rx else { return };
        let Ok((manual, result)) = rx.try_recv() else {
            return;
        };
        self.update_rx = None;
        match result {
            // A periodic check that finds the version already announced stays silent.
            Ok(Check::Newer(release))
                if !manual
                    && self.new_release.as_ref().map(|r| &r.version) == Some(&release.version) => {}
            Ok(Check::Newer(release)) => {
                // A still newer release than the one downloaded earlier: fetch that one instead.
                if matches!(self.self_update, SelfUpdate::Ready(_)) {
                    self.self_update = SelfUpdate::Idle(None);
                }
                self.info(
                    LogLevel::Info,
                    crate::i18n::msg(
                        "Доступна версия {0} (установлена {1}): {2}",
                        &[&release.version, &update::CURRENT, &release.url],
                    ),
                );
                self.new_release = Some(release);
                // The window opens by itself only when the user asked for the check.
                self.update_window = manual;
                if self.auto_update {
                    self.start_known_release_download();
                }
            }
            Ok(Check::UpToDate) if manual => self.info(
                LogLevel::Info,
                crate::i18n::msg("Установлена последняя версия ({0})", &[&update::CURRENT]),
            ),
            Ok(Check::NoReleases) if manual => self.info(
                LogLevel::Info,
                crate::i18n::tr(
                    "Выпущенных версий не найдено (или у токена нет доступа к репозиторию)",
                ),
            ),
            Ok(_) => {}
            Err(e) => self.info(
                LogLevel::Warning,
                crate::i18n::msg("Проверка обновлений: {0}", &[&e]),
            ),
        }
    }
}
