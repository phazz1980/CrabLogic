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
