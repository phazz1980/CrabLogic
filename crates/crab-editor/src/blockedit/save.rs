use super::*;

/// Modal question on closing with unsaved changes; returns true when the window
/// should close (saved or changes dropped).
pub(super) fn confirm_close(
    app: &mut EditorApp,
    ctx: &egui::Context,
    ed: &mut BlockEditor,
) -> bool {
    let def = ed.form.to_def();
    let name = ed.form.name.trim().to_owned();
    let error = app.theme.palette.error;
    let mut answer = None;
    egui::Modal::new(Id::unique("block_editor_close")).show(ctx, |ui| {
        ui.label(RichText::new(crate::i18n::tr("Сохранить изменения?")).strong());
        ui.label(crate::i18n::fmt(
            "В блоке «{0}» есть несохранённые изменения.",
            &[&name],
        ));
        if let Err(e) = &def {
            ui.colored_label(error, crate::i18n::fmt("Сохранить нельзя: {0}", &[&e]));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(def.is_ok(), egui::Button::new(crate::i18n::tr("Сохранить")))
                .clicked()
            {
                answer = Some(1);
            }
            if ui.button(crate::i18n::tr("Выйти без сохранения")).clicked() {
                answer = Some(2);
            }
            if ui.button(crate::i18n::tr("Отмена")).clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                answer = Some(0);
            }
        });
    });
    match answer {
        Some(1) => {
            let saved = save(app, ed, &def);
            ed.confirm_close = saved;
            saved
        }
        Some(2) => true,
        Some(_) => {
            ed.confirm_close = false;
            false
        }
        None => false,
    }
}

pub(super) fn status(ui: &mut Ui, ed: &BlockEditor, def: &Result<BlockDef, String>, pal: &Palette) {
    let (color, text): (Color32, String) = match (def, &ed.generated) {
        (Err(e), _) => (pal.error, crate::i18n::fmt("Описание: {0}", &[&e])),
        (Ok(_), Some(Generated { result: Err(e), .. })) => {
            let first = e.lines().next().unwrap_or_default();
            (pal.error, crate::i18n::fmt("Код: {0}", &[&first]))
        }
        _ => (
            pal.ok,
            crate::i18n::tr("Описание и код в порядке").to_owned(),
        ),
    };
    ui.colored_label(color, text);
}

/// Save and close buttons; returns true when the window should close.
pub(super) fn footer(
    app: &mut EditorApp,
    ui: &mut Ui,
    ed: &mut BlockEditor,
    def: &Result<BlockDef, String>,
) -> bool {
    let mut close = false;
    if ed.read_only {
        ui.horizontal(|ui| {
            ui.colored_label(
                app.theme.palette.warning,
                crate::i18n::tr("Встроенный блок изменять нельзя — создайте на его основе свой."),
            );
            if ui.button(crate::i18n::tr("Создать копию")).clicked() {
                let block = ed.original.as_ref().and_then(|k| app.lib.get(k)).cloned();
                if let Some(block) = block {
                    *ed = BlockEditor::copy_of(app, &block);
                }
            }
            close = ui.button(crate::i18n::tr("Закрыть")).clicked();
        });
        return close;
    }
    ui.horizontal(|ui| {
        // A new block can be saved untouched; an opened one only after changes.
        let savable = ed.dirty() || ed.original.is_none();
        if ui
            .add_enabled(
                def.is_ok() && savable,
                egui::Button::new(crate::i18n::tr("Сохранить")),
            )
            .clicked()
        {
            save(app, ed, def);
        }
        if ui
            .add_enabled(
                ed.form != ed.clean,
                egui::Button::new(crate::i18n::tr("Отменить правки")),
            )
            .clicked()
        {
            ed.form = ed.clean.clone();
        }
        if ui.button(crate::i18n::tr("Закрыть")).clicked() {
            if ed.dirty() {
                ed.confirm_close = true;
            } else {
                close = true;
            }
        }
        if let Some(dir) = target_dir(app, ed) {
            ui.weak(crate::i18n::fmt(
                "Папка: {0}",
                &[&dir.join(&ed.form.kind).display()],
            ));
        }
    });
    close
}

/// Folder the block is saved into: where it already lies, or for a new block
/// its section and folders under `user_library/`.
pub(super) fn target_dir(app: &EditorApp, ed: &BlockEditor) -> Option<PathBuf> {
    let existing = ed.original.as_ref().and_then(|k| app.lib.get(k));
    if let Some(parent) = existing.and_then(|b| b.dir.parent()) {
        return Some(parent.to_owned());
    }
    let user = app.user_lib_dir()?;
    Some(
        ed.form
            .group
            .split('/')
            .filter(|p| !p.is_empty())
            .fold(user, |d, p| d.join(p)),
    )
}

/// Writes the block into the library and reloads it; returns whether it worked.
pub(super) fn save(
    app: &mut EditorApp,
    ed: &mut BlockEditor,
    def: &Result<BlockDef, String>,
) -> bool {
    let def = match def {
        Ok(def) => def,
        Err(e) => {
            app.info(
                LogLevel::Error,
                crate::i18n::msg("Блок не сохранён: {0}", &[&e]),
            );
            return false;
        }
    };
    let Some(lib_dir) = target_dir(app, ed) else {
        app.info(
            LogLevel::Error,
            crate::i18n::tr("Блок не сохранён: нет папки библиотеки"),
        );
        return false;
    };
    let kind = &def.info.kind;
    if ed.read_only || app.lib.is_builtin(kind) {
        app.info(
            LogLevel::Error,
            crate::i18n::msg(
                "Блок не сохранён: тип «{0}» занят встроенным блоком, его менять нельзя",
                &[&kind],
            ),
        );
        return false;
    }
    let new = ed.original.as_deref() != Some(kind.as_str());
    if new && (app.lib.get(kind).is_some() || lib_dir.join(kind).exists()) {
        app.info(
            LogLevel::Error,
            crate::i18n::msg(
                "Блок не сохранён: тип «{0}» уже есть, выберите другое имя",
                &[&kind],
            ),
        );
        return false;
    }
    // A block always lives in a pack: the first save into a new one creates it.
    if let Some(user) = app.user_lib_dir() {
        if let Some(name) = lib_dir
            .strip_prefix(&user)
            .ok()
            .and_then(|p| p.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
        {
            let pack_dir = user.join(&name);
            if !pack_dir.join(crab_core::pack::PACK_FILE).is_file() {
                if let Err(e) = crab_core::pack::PackInfo::new(&name).write(&pack_dir) {
                    app.info(
                        LogLevel::Error,
                        crate::i18n::msg("Блок не сохранён: {0}", &[&e]),
                    );
                    return false;
                }
            }
        }
    }
    // The wizard with mistakes cannot be saved: its code would not match the fields.
    if let Some(w) = &ed.form.wizard {
        let errors = w.validate();
        if !errors.is_empty() {
            app.info(
                LogLevel::Error,
                crate::i18n::msg(
                    "Блок не сохранён: мастер кода C++ для платы не закончен ({0})",
                    &[&errors.join("; ")],
                ),
            );
            return false;
        }
    }
    // A native part that does not parse never lands in the library.
    if !ed.form.native.trim().is_empty() {
        if let Err(errors) = crab_core::native::parse_native(&ed.form.native, def) {
            app.info(
                LogLevel::Error,
                crate::i18n::msg(
                    "Блок не сохранён: в коде C++ для платы {0}",
                    &[&crab_core::native::join_errors(&errors)],
                ),
            );
            return false;
        }
    }
    match crab_core::save_block(&lib_dir, def, &ed.form.template, &ed.form.docs) {
        Ok(dir) => {
            if let Err(e) = crab_core::save_native(&lib_dir, &def.info.kind, &ed.form.native) {
                app.info(
                    LogLevel::Error,
                    crate::i18n::msg("Код C++ для платы не сохранён: {0}", &[&e]),
                );
            }
            app.info(
                LogLevel::Ok,
                crate::i18n::msg(
                    "Блок «{0}» сохранён в {1}",
                    &[&def.info.name, &dir.display()],
                ),
            );
            if let Err(e) = ed.form.attach.apply(&dir, &ed.form.docs) {
                app.info(
                    LogLevel::Error,
                    crate::i18n::msg("Библиотека или пример не сохранены: {0}", &[&e]),
                );
            }
            ed.sim_build.start(&dir, def, &ed.form.template);
            // A block without a template runs from its module: translate it for the firmware.
            if ed.form.template.trim().is_empty() {
                if let Some(w2c2) = crate::theme::w2c2() {
                    let (dir, kind) = (dir.clone(), kind.clone());
                    std::thread::spawn(move || {
                        let _ = crab_build::wasm_c::translate_firmware(&w2c2, &dir, &kind);
                    });
                }
            }
            ed.original = Some(kind.clone());
            ed.clean = ed.form.clone();
            app.reload_library();
            true
        }
        Err(e) => {
            app.info(
                LogLevel::Error,
                crate::i18n::msg("Блок не сохранён: {0}", &[&e]),
            );
            false
        }
    }
}

/// Asks before deleting a user block (`app.block_delete`) and deletes its folder.
pub fn delete_dialog(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(kind) = app.block_delete.clone() else {
        return;
    };
    let (Some(block), Some(user_dir)) = (app.lib.get(&kind).cloned(), app.user_lib_dir()) else {
        app.block_delete = None;
        return;
    };
    if !app.lib.is_user(&kind) {
        app.block_delete = None;
        app.info(
            LogLevel::Error,
            crate::i18n::tr("Встроенный блок удалить нельзя"),
        );
        return;
    }
    let used = app.doc.as_ref().map_or(0, |d| {
        d.project
            .logic
            .blocks
            .iter()
            .filter(|b| b.kind == kind)
            .count()
    });
    let warning = app.theme.palette.warning;
    let mut answer = None;
    egui::Modal::new(Id::unique("block_delete")).show(ctx, |ui| {
        ui.label(RichText::new(crate::i18n::tr("Удалить блок")).strong());
        ui.label(crate::i18n::fmt(
            "Удалить пользовательский блок «{0}» ({1})? Папка {2} будет удалена.",
            &[&block.def.info.name, &kind, &block.dir.display()],
        ));
        if used > 0 {
            ui.colored_label(
                warning,
                crate::i18n::fmt(
                    "В открытом проекте таких блоков: {0}. Схема перестанет проходить проверку.",
                    &[&used],
                ),
            );
        }
        ui.horizontal(|ui| {
            if ui.button(crate::i18n::tr("Удалить")).clicked() {
                answer = Some(true);
            }
            if ui.button(crate::i18n::tr("Отмена")).clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                answer = Some(false);
            }
        });
    });
    match answer {
        Some(true) => {
            app.block_delete = None;
            // Only a folder inside user_library/ is ever removed.
            if !block.dir.starts_with(&user_dir) || block.dir == user_dir {
                app.info(
                    LogLevel::Error,
                    crate::i18n::tr("Блок не удалён: он не в папке пользовательских блоков"),
                );
                return;
            }
            match fs::remove_dir_all(&block.dir) {
                Ok(()) => {
                    app.info(
                        LogLevel::Ok,
                        crate::i18n::msg("Блок «{0}» удалён", &[&kind]),
                    );
                    if app
                        .block_editor
                        .as_ref()
                        .is_some_and(|e| e.original.as_deref() == Some(kind.as_str()))
                    {
                        app.block_editor = None;
                    }
                    app.reload_library();
                }
                Err(e) => app.info(
                    LogLevel::Error,
                    crate::i18n::msg("Блок не удалён: {0}", &[&e]),
                ),
            }
        }
        Some(false) => app.block_delete = None,
        None => {}
    }
}
