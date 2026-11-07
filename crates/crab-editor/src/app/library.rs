use super::*;

impl EditorApp {
    /// Folder of user blocks: `user_library/` next to the built-in library.
    pub fn user_lib_dir(&self) -> Option<PathBuf> {
        self.lib_dir.as_deref().map(crab_core::user_library_dir)
    }

    pub fn reload_library(&mut self) {
        let Some(dir) = self.lib_dir.clone() else {
            self.info(
                LogLevel::Error,
                crate::i18n::tr(
                    "Не найдена папка библиотеки блоков library/ (задайте CRAB_LIBRARY)",
                ),
            );
            return;
        };
        match Library::load_with_user(&dir) {
            Ok((lib, errors)) => {
                let user = lib.iter().filter(|b| lib.is_user(&b.def.info.kind)).count();
                let builtin = lib.len() - user;
                self.lib = lib;
                if let Some(help) = &mut self.help {
                    help.blocks_dirty = true;
                }
                for e in errors {
                    self.info(LogLevel::Warning, crate::i18n::msg("Пропущено: {0}", &[&e]));
                }
                self.info(
                    LogLevel::Info,
                    crate::i18n::msg(
                        "Библиотека: {0} встроенных блоков из {1}, {2} пользовательских",
                        &[&builtin, &dir.display(), &user],
                    ),
                );
            }
            Err(e) => self.info(LogLevel::Error, e.to_string()),
        }
        self.lib_stamp = library_stamp(&dir);
        if let Some(doc) = &mut self.doc {
            doc.recheck(&self.lib);
        }
    }

    /// Hot reload: new or changed block folders are picked up without restart.
    pub(super) fn watch_library(&mut self) {
        if self.lib_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.lib_checked = Instant::now();
        let Some(dir) = &self.lib_dir else { return };
        let stamp = library_stamp(dir);
        if stamp != self.lib_stamp {
            self.reload_library();
        }
    }

    pub(super) fn project_stamp(&self) -> Option<SystemTime> {
        let doc = self.doc.as_ref()?;
        fs::metadata(&doc.project.path)
            .and_then(|m| m.modified())
            .ok()
    }

    /// Picks up changes made to the open project file by other programs. Unsaved work in
    /// the editor is kept (only a warning is logged); otherwise the project is reloaded.
    pub(super) fn watch_project(&mut self) {
        if self.disk_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.disk_checked = Instant::now();
        let Some(doc) = &self.doc else { return };
        let stamp = self.project_stamp();
        if stamp.is_none() || stamp == self.disk_stamp {
            return;
        }
        if doc.dirty {
            // The web constructor writes the widgets into the file: take them over, or the
            // next save would wipe them. The rest of the unsaved work stays.
            let path = doc.project.path.clone();
            self.disk_stamp = stamp;
            if let Ok(other) = Doc::open(&path, &self.lib) {
                if let Some(doc) = &mut self.doc {
                    doc.adopt_web(&other.project, &self.lib);
                }
            }
            if !self.disk_warned {
                self.disk_warned = true;
                self.info(
                    LogLevel::Warning,
                    crate::i18n::tr("Файл проекта изменён другой программой, а в редакторе есть несохранённые правки:                      сохранение перезапишет внешние изменения"),
                );
            }
            return;
        }
        let path = doc.project.path.clone();
        match Doc::open(&path, &self.lib) {
            Ok(new) => {
                self.doc = Some(new);
                self.disk_stamp = stamp;
                self.selection = None;
                self.info(
                    LogLevel::Info,
                    crate::i18n::tr("Проект обновлён: файл изменён снаружи"),
                );
            }
            Err(e) => {
                self.disk_stamp = stamp;
                self.info(LogLevel::Error, e.to_string());
            }
        }
    }

    /// User blocks packed into the project file but missing on this computer are put into
    /// `user_library/` before the project is checked.
    pub(super) fn restore_packed_blocks(&mut self, path: &Path) {
        let Some(dir) = self.user_lib_dir() else {
            return;
        };
        if !crab_core::project::is_project_file(path) && !path.is_dir() {
            return;
        }
        // The board goes first: the library has to know it for the blocks of its devices.
        let boards = crab_core::packed::unpack_missing_boards(path, &dir, &self.lib);
        if !boards.is_empty() {
            self.info(
                LogLevel::Info,
                crate::i18n::msg("Из проекта добавлена плата: {0}", &[&boards.join(", ")]),
            );
            self.reload_library();
        }
        let written = crab_core::packed::unpack_missing_blocks(path, &dir, &self.lib);
        if !written.is_empty() {
            self.info(
                LogLevel::Info,
                crate::i18n::msg(
                    "Из проекта добавлены пользовательские блоки: {0}",
                    &[&written.join(", ")],
                ),
            );
            self.reload_library();
        }
    }

    /// Offers to sync user blocks whose contents differ between the project and the
    /// program. Each block gets one of two choices: keep the project's copy (it replaces
    /// the program's) or replace it with the program's library copy (saved into the
    /// project). Blocks changed independently on both sides have no preselected choice.
    pub(super) fn block_diffs_window(&mut self, ctx: &egui::Context) {
        use crab_core::packed::Newer;
        if self.block_diffs.is_empty() {
            return;
        }
        let undecided = self.block_pick.values().any(|p| *p == BlockPick::Undecided);
        let any_replace = self.block_pick.values().any(|p| *p == BlockPick::Replace);
        let mut apply = false;
        let mut close = false;
        egui::Window::new(crate::i18n::tr("Блоки проекта и программы"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(crate::i18n::tr(
                    "Пользовательские блоки в проекте и в программе отличаются содержимым.",
                ));
                ui.weak(crate::i18n::tr(
                    "Выберите для каждого блока одно: оставить его таким, как в проекте, или заменить в проекте копией из библиотеки программы.",
                ));
                ui.add_space(6.0);
                egui::Grid::new("block_diffs")
                    .num_columns(3)
                    .spacing([24.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong(crate::i18n::tr("Название"));
                        ui.strong(crate::i18n::tr("Оставить"))
                            .on_hover_text(crate::i18n::tr(
                                "Блок остаётся таким, как в проекте: копия в программе заменится им",
                            ));
                        ui.strong(crate::i18n::tr("Заменить из библиотеки"))
                            .on_hover_text(crate::i18n::tr(
                                "Копия в проекте заменится блоком из библиотеки программы (проект будет сохранён)",
                            ));
                        ui.end_row();
                        for d in &self.block_diffs {
                            ui.horizontal(|ui| {
                                ui.label(&d.kind);
                                let (text, color) = match d.newer {
                                    Newer::Project => (
                                        "новее в проекте",
                                        egui::Color32::from_rgb(90, 160, 235),
                                    ),
                                    Newer::Program => (
                                        "новее в библиотеке",
                                        egui::Color32::from_rgb(90, 190, 120),
                                    ),
                                    Newer::Parallel => (
                                        "параллельные версии: изменены независимо",
                                        egui::Color32::from_rgb(235, 160, 60),
                                    ),
                                    Newer::Unknown => (
                                        "какой новее — неизвестно",
                                        ui.visuals().weak_text_color(),
                                    ),
                                };
                                ui.label(
                                    egui::RichText::new(format!("({})", crate::i18n::tr(text)))
                                        .color(color),
                                );
                            });
                            // Two checkboxes of a row exclude each other.
                            let pick = self
                                .block_pick
                                .get(&d.kind)
                                .copied()
                                .unwrap_or(BlockPick::Undecided);
                            let mut keep = pick == BlockPick::Keep;
                            if ui.checkbox(&mut keep, "").clicked() {
                                self.block_pick.insert(d.kind.clone(), BlockPick::Keep);
                            }
                            let mut from_library = pick == BlockPick::Replace;
                            if ui.checkbox(&mut from_library, "").clicked() {
                                self.block_pick.insert(d.kind.clone(), BlockPick::Replace);
                            }
                            ui.end_row();
                        }
                    });
                if any_replace {
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut self.block_keep_copy,
                        crate::i18n::tr(
                            "При замене сохранить копию из проекта рядом как новый блок «<тип>_project»",
                        ),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Чтобы сравнить две версии и перенести нужное вручную; копия появится в библиотеке",
                    ));
                }
                if undecided {
                    ui.add_space(6.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(235, 160, 60),
                        crate::i18n::tr(
                            "Для блоков с независимыми правками выберите вариант: без этого применить нельзя.",
                        ),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let button = |text: &str, fill: egui::Color32| {
                        egui::Button::new(
                            egui::RichText::new(crate::i18n::tr(text))
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(90)))
                        .min_size(egui::vec2(140.0, 30.0))
                    };
                    apply = ui
                        .add_enabled(
                            !undecided,
                            button("Применить", egui::Color32::from_rgb(38, 105, 185)),
                        )
                        .clicked();
                    close = ui
                        .add(button("Решить позже", egui::Color32::from_rgb(88, 92, 100)))
                        .on_hover_text(crate::i18n::tr(
                            "Ничего не менять; вопрос появится при следующем открытии проекта",
                        ))
                        .clicked();
                });
            });
        if apply {
            let of = |wanted: BlockPick| -> Vec<String> {
                self.block_diffs
                    .iter()
                    .filter(|d| self.block_pick.get(&d.kind) == Some(&wanted))
                    .map(|d| d.kind.clone())
                    .collect()
            };
            let take = of(BlockPick::Keep);
            let replace = of(BlockPick::Replace);
            if self.block_keep_copy {
                self.save_copies_aside(&replace);
            }
            if !take.is_empty() {
                if let (Some(dir), Some(doc)) = (self.user_lib_dir(), &self.doc) {
                    let written =
                        crab_core::packed::adopt_project_blocks(&doc.project.path, &dir, &take);
                    self.info(
                        LogLevel::Ok,
                        crate::i18n::msg(
                            "Блоки программы обновлены из проекта: {0}",
                            &[&written.join(", ")],
                        ),
                    );
                    self.reload_library();
                }
            }
            if !replace.is_empty() {
                // Saving packs the program's copies of the blocks the schema uses.
                if self.save() {
                    self.info(
                        LogLevel::Ok,
                        crate::i18n::tr("Блоки проекта обновлены из программы"),
                    );
                }
            }
            self.block_diffs.clear();
        }
        if close {
            self.block_diffs.clear();
        }
    }

    /// Saves the project's copies of the blocks `kinds` into the library as `<type>_project`
    /// (a free name is picked), before the project's copies are replaced.
    pub(super) fn save_copies_aside(&mut self, kinds: &[String]) {
        let (Some(dir), Some(path)) = (
            self.user_lib_dir(),
            self.doc.as_ref().map(|d| d.project.path.clone()),
        ) else {
            return;
        };
        let mut saved = Vec::new();
        for kind in kinds {
            let name = (0..)
                .map(|n| match n {
                    0 => format!("{kind}_project"),
                    n => format!("{kind}_project{n}"),
                })
                .find(|name| self.lib.get(name).is_none() && !saved.contains(name))
                .expect("an endless range has a free name");
            if crab_core::packed::save_project_copy_aside(&path, &dir, kind, &name) {
                saved.push(name);
            }
        }
        if !saved.is_empty() {
            self.info(
                LogLevel::Ok,
                crate::i18n::msg(
                    "Копии из проекта сохранены как новые блоки: {0}",
                    &[&saved.join(", ")],
                ),
            );
            self.reload_library();
        }
    }
}

/// `library/` next to the project (or its parents), next to the executable, or `CRAB_LIBRARY`.
pub(super) fn find_library(project: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CRAB_LIBRARY") {
        return Some(PathBuf::from(dir));
    }
    let starts = [
        project.and_then(|p| std::path::absolute(p).ok()),
        std::env::current_dir().ok(),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
    ];
    starts
        .into_iter()
        .flatten()
        .flat_map(|s| s.ancestors().map(|a| a.join("library")).collect::<Vec<_>>())
        .find(|d| d.is_dir())
}

/// Change stamp of the built-in and user block folders.
pub(super) fn library_stamp(dir: &Path) -> Option<SystemTime> {
    newest_mtime(dir).max(newest_mtime_deep(&crab_core::user_library_dir(dir)))
}

/// Latest modification time in a whole tree: user blocks lie in sections and folders.
pub(super) fn newest_mtime_deep(dir: &Path) -> Option<SystemTime> {
    let mut newest = fs::metadata(dir).and_then(|m| m.modified()).ok();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let t = if path.is_dir() {
            newest_mtime_deep(&path)
        } else {
            fs::metadata(&path).and_then(|m| m.modified()).ok()
        };
        newest = newest.max(t);
    }
    newest
}

/// Latest modification time of any file in the library (two levels deep).
pub(super) fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest = fs::metadata(dir).and_then(|m| m.modified()).ok();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let mut consider = |path: &Path| {
            if let Ok(t) = fs::metadata(path).and_then(|m| m.modified()) {
                newest = newest.max(Some(t));
            }
        };
        consider(&entry.path());
        if let Ok(files) = fs::read_dir(entry.path()) {
            for f in files.flatten() {
                consider(&f.path());
            }
        }
    }
    newest
}

pub(super) fn is_block_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        e.eq_ignore_ascii_case(crab_core::pack::EXTENSION)
            || e.eq_ignore_ascii_case(crab_core::pack::LEGACY_EXTENSION)
    })
}
