use super::*;

impl EditorApp {
    /// Menu part: build simulation modules of user blocks with zig.
    pub(super) fn sim_settings_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(
                &mut self.template_sim,
                crate::i18n::tr("Собирать симуляцию пользовательских блоков"),
            )
            .on_hover_text(crate::i18n::tr(
                "При сохранении блока его код превращается в модуль симуляции; нужен компилятор zig",
            ))
            .changed()
        {
            theme::save_template_sim(self.template_sim);
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("Путь к zig:"));
            if ui.text_edit_singleline(&mut self.zig_path).changed() {
                theme::save_zig_path(&self.zig_path);
                self.zig_status.clear();
            }
            if ui.button(crate::i18n::tr("Обзор...")).clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.zig_path = path.to_string_lossy().into_owned();
                    theme::save_zig_path(&self.zig_path);
                    self.zig_status.clear();
                }
            }
        });
        if ui.button(crate::i18n::tr("Проверить")).clicked() {
            self.zig_status =
                match crab_build::wasm::zig_version(std::path::Path::new(self.zig_path.trim())) {
                    Ok(v) => crate::i18n::fmt("zig {0} найден", &[&v]),
                    Err(e) => e,
                };
        }
        if !self.zig_status.is_empty() {
            ui.label(&self.zig_status);
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("Путь к w2c2:"))
                .on_hover_text(crate::i18n::tr(
                "Нужен блокам на Rust/WASM без шаблона C++: переводит block.wasm в C для прошивки",
            ));
            if ui.text_edit_singleline(&mut self.w2c2_path).changed() {
                theme::save_w2c2_path(&self.w2c2_path);
                self.w2c2_status.clear();
            }
            if ui.button(crate::i18n::tr("Обзор...")).clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.w2c2_path = path.to_string_lossy().into_owned();
                    theme::save_w2c2_path(&self.w2c2_path);
                    self.w2c2_status.clear();
                }
            }
        });
        if ui.button(crate::i18n::tr("Проверить w2c2")).clicked() {
            self.w2c2_status =
                match crab_build::wasm_c::w2c2_check(std::path::Path::new(self.w2c2_path.trim())) {
                    Ok(()) => crate::i18n::tr("w2c2 найден").to_owned(),
                    Err(e) => e,
                };
        }
        if !self.w2c2_status.is_empty() {
            ui.label(&self.w2c2_status);
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("Путь к wamrc (AOT-компилятор модулей):"))
                .on_hover_text(crate::i18n::tr(
                    "Пустое поле: wamrc скачается автоматически при первом использовании",
                ));
            if ui.text_edit_singleline(&mut self.wamrc_path).changed() {
                theme::save_wamrc_path(&self.wamrc_path);
            }
            if ui.button(crate::i18n::tr("Обзор...")).clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.wamrc_path = path.to_string_lossy().into_owned();
                    theme::save_wamrc_path(&self.wamrc_path);
                }
            }
        });
    }

    /// Opens the help window on `target` (`page` or `page#anchor`).
    pub fn open_help(&mut self, target: &str) {
        match &mut self.help {
            Some(help) => help.goto(target),
            None => self.help = Some(crate::help::HelpWindow::new(&self.lib, target)),
        }
    }

    /// Help page of the selected block, or the start page without one.
    pub(super) fn help_target(&self) -> String {
        let kind = match (&self.selection, &self.doc) {
            (Some(Selection::Block(id)), Some(doc)) => doc
                .project
                .logic
                .blocks
                .iter()
                .find(|b| b.id == *id)
                .map(|b| b.kind.as_str()),
            _ => None,
        };
        match kind.and_then(|k| self.lib.get(k)) {
            Some(block) if !block.def.info.hidden => {
                format!("{}/{}", crate::help::BLOCKS, block.def.info.kind)
            }
            _ => crate::help::START.to_owned(),
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let shortcut = |m, k| KeyboardShortcut::new(m, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        // The shortcut with Shift first: `COMMAND` alone would also match it.
        if pressed(shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)) {
            self.save_as();
        }
        if pressed(shortcut(Modifiers::COMMAND, Key::S)) {
            self.save();
        }
        if pressed(shortcut(Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }
        if pressed(shortcut(Modifiers::COMMAND, Key::Y))
            || pressed(shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
        {
            self.redo();
        }
        if pressed(shortcut(Modifiers::NONE, Key::F1)) {
            let target = self.help_target();
            self.open_help(&target);
        }
        if pressed(shortcut(Modifiers::NONE, Key::F11)) {
            self.toggle_fullscreen(ctx);
        }
        if pressed(shortcut(Modifiers::NONE, Key::F7)) {
            self.run_check();
        }
        if pressed(shortcut(Modifiers::NONE, Key::F5)) {
            self.start_job(JobKind::Build, ctx);
        }
        // Delete only when no text field or window has focus.
        if self.dialog.is_none()
            && !ctx.egui_wants_keyboard_input()
            && pressed(shortcut(Modifiers::NONE, Key::Delete))
        {
            self.delete_selection();
        }
        if self.dialog.is_none() && !ctx.egui_wants_keyboard_input() {
            self.nudge_selection(ctx);
        }
        if self.dialog.is_none() && !ctx.egui_wants_keyboard_input() {
            let (copy, cut, paste) = ctx.input(|i| {
                let mut copy = false;
                let mut cut = false;
                let mut paste = None;
                for e in &i.events {
                    match e {
                        egui::Event::Copy => copy = true,
                        egui::Event::Cut => cut = true,
                        egui::Event::Paste(t) => paste = Some(t.clone()),
                        _ => {}
                    }
                }
                (copy, cut, paste)
            });
            if cut {
                self.cut_selection(ctx);
            } else if copy {
                self.copy_selection(ctx);
            }
            if let Some(text) = paste {
                let at = self.canvas.paste_pos();
                self.paste(&text, at);
            }
        }
    }

    /// Custom window title bar: icon, burger with the editor menus, draggable title and
    /// window buttons (the OS decorations are off).
    pub(super) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (maximized, fullscreen, inner, monitor) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.maximized.unwrap_or(false),
                v.fullscreen.unwrap_or(false),
                v.inner_rect.map(|r| r.size()),
                v.monitor_size,
            )
        });
        self.fix_fullscreen(&ctx, maximized, fullscreen, inner, monitor);
        // Loaded outside `data_mut`: `load_texture` takes the context lock too.
        let icon_id = egui::Id::unique("app_icon");
        let icon = match ctx.data(|d| d.get_temp::<egui::TextureHandle>(icon_id)) {
            Some(icon) => icon,
            None => {
                let icon = ctx.load_texture(
                    "app_icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [64, 64],
                        include_bytes!("../../assets/icon64.rgba"),
                    ),
                    egui::TextureOptions::LINEAR,
                );
                ctx.data_mut(|d| d.insert_temp(icon_id, icon.clone()));
                icon
            }
        };
        ui.add_space(6.0);
        // The row is as tall as the port selector from the start, so every item is
        // centred on the same line (a taller item would only lower those after it).
        let row_height = ui.spacing().interact_size.y;
        ui.spacing_mut().interact_size.y = 34.0;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::vec2(24.0, 24.0)));
            // No `MenuBar` here: it takes the full row width and pushes the rest off screen.
            // The burger is drawn with lines, so it does not depend on font glyphs.
            let burger = ui
                .allocate_response(egui::vec2(36.0, 30.0), egui::Sense::click())
                .on_hover_text(crate::i18n::tr("Меню"));
            let color = if burger.hovered() || burger.is_pointer_button_down_on() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().text_color()
            };
            if burger.hovered() {
                ui.painter().rect_filled(
                    burger.rect,
                    3.0,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            for dy in [-6.0, 0.0, 6.0] {
                let y = burger.rect.center().y + dy;
                ui.painter().line_segment(
                    [
                        egui::pos2(burger.rect.center().x - 9.0, y),
                        egui::pos2(burger.rect.center().x + 9.0, y),
                    ],
                    egui::Stroke::new(2.0, color),
                );
            }
            egui::Popup::menu(&burger).show(|ui| self.menus(ui));
            panels::toolbar(self, ui);
            // The rest of the row drags the window; double click maximizes.
            // The program name sits at its right end, next to the window buttons.
            let width = (ui.available_width() - 3.0 * 44.0 - 14.0).max(0.0);
            let (rect, drag) = ui.allocate_exact_size(
                egui::vec2(width, ui.spacing().interact_size.y.max(30.0)),
                egui::Sense::click_and_drag(),
            );
            let font = egui::TextStyle::Body.resolve(ui.style());
            let galley = ui.painter().layout_no_wrap(
                "CrabLogic".to_owned(),
                font,
                ui.visuals().weak_text_color(),
            );
            if galley.size().x + 12.0 <= rect.width() {
                ui.painter().galley(
                    egui::pos2(
                        rect.right() - 8.0 - galley.size().x,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    ui.visuals().weak_text_color(),
                );
            }
            if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() && !fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if !fullscreen {
                if panels::window_button(ui, panels::WinIcon::Minimize, crate::i18n::tr("Свернуть"))
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                let (icon, tip) = if maximized {
                    (panels::WinIcon::Restore, crate::i18n::tr("Восстановить"))
                } else {
                    (panels::WinIcon::Maximize, crate::i18n::tr("Развернуть"))
                };
                if panels::window_button(ui, icon, tip) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            } else if panels::window_button(
                ui,
                panels::WinIcon::ExitFullscreen,
                crate::i18n::tr("Выйти из полного экрана (F11)"),
            ) {
                self.toggle_fullscreen(&ctx);
            }
            if panels::window_button(ui, panels::WinIcon::Close, crate::i18n::tr("Закрыть"))
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.spacing_mut().interact_size.y = row_height;
    }

    pub(super) fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        let (on, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (v.fullscreen.unwrap_or(false), v.maximized.unwrap_or(false))
        });
        if on {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        } else if maximized {
            // A maximized undecorated window keeps the work area (the taskbar stays
            // visible) when made full screen: leave the maximized state first.
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            self.fs_pending = true;
            ctx.request_repaint();
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
    }

    /// Finishes entering full screen after the window left the maximized state; if the
    /// window still does not cover the monitor, resizes it to the monitor.
    pub(super) fn fix_fullscreen(
        &mut self,
        ctx: &egui::Context,
        maximized: bool,
        fullscreen: bool,
        inner: Option<egui::Vec2>,
        monitor: Option<egui::Vec2>,
    ) {
        if self.fs_pending && !maximized {
            self.fs_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        if !fullscreen {
            self.fs_fixes = 0;
            return;
        }
        if let (Some(inner), Some(monitor)) = (inner, monitor) {
            let gap = (inner.x - monitor.x).abs() > 2.0 || (inner.y - monitor.y).abs() > 2.0;
            if gap && self.fs_fixes < 2 {
                self.fs_fixes += 1;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(monitor));
            }
        }
    }

    /// "Project in the board": keep a copy in the firmware and protect it with a password.
    pub(super) fn board_copy_menu(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let info = doc.project.config.project.clone();
        let lib = self.lib.clone();
        let (mut embed, mut password) = (info.embed, info.password.clone());
        ui.menu_button(
            crate::i18n::tr("Проект в памяти платы"),
            |ui| {
                ui.checkbox(&mut embed, crate::i18n::tr("Хранить копию в прошивке"))
                    .on_hover_text(crate::i18n::tr(
                        "Тогда проект можно прочитать с платы (Файл → Открыть проект с платы)",
                    ));
                ui.add_enabled_ui(embed, |ui| {
                    ui.label(crate::i18n::tr("Пароль (пусто — не шифровать):"));
                    ui.add(egui::TextEdit::singleline(&mut password).password(true));
                });
            },
        );
        if embed != info.embed || password != info.password {
            if let Some(doc) = &mut self.doc {
                let _ = doc.edit(&lib, Some("project_copy".into()), |p| {
                    p.config.project.embed = embed;
                    p.config.project.password = password;
                    Ok::<(), String>(())
                });
            }
        }
    }

    /// "Flash saving": default timing of retained variables (`[retain]` of the project).
    pub(super) fn retain_menu(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let cur = doc.project.config.retain;
        let lib = self.lib.clone();
        let mut new = cur;
        ui.menu_button(crate::i18n::tr("Сохранение во флеш"), |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Сохранение во флеш: покой, с"))
                    .on_hover_text(crate::i18n::tr(
                        "Значение пишется во флеш, только когда не менялось столько секунд: частые правки дают одну запись",
                    ));
                ui.add(egui::DragValue::new(&mut new.settle_s).range(0..=86_400));
            });
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("не чаще раза в, с"))
                    .on_hover_text(crate::i18n::tr(
                        "Минимум секунд между записями одной переменной; 0 — без ограничения. Флеш выдерживает ограниченное число записей",
                    ));
                ui.add(egui::DragValue::new(&mut new.period_s).range(0..=86_400));
            });
            ui.weak(crate::i18n::tr("0 в периоде — без ограничения"));
        });
        if new != cur {
            if let Some(doc) = &mut self.doc {
                let _ = doc.edit(&lib, Some("retain_timing".into()), |p| {
                    p.config.retain = new;
                    Ok::<(), String>(())
                });
            }
        }
    }

    /// The editor menus; shown inside the burger button of the title bar.
    pub(super) fn menus(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(crate::i18n::tr("Файл"), |ui| {
            if ui.button(crate::i18n::tr("Новый проект...")).clicked() {
                self.new_project();
            }
            if ui.button(crate::i18n::tr("Открыть...")).clicked() {
                self.pick_project();
            }
            if ui
                .button(crate::i18n::tr("Открыть проект с платы..."))
                .on_hover_text(crate::i18n::tr("Прочитать проект из памяти платы (нужна прошивка с отладкой)"))
                .clicked()
            {
                self.pull = Some(crate::pullview::PullWindow::new(self.board_port().unwrap_or_default()));
            }
            let has_doc = self.doc.is_some();
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Сохранить")).shortcut_text("Ctrl+S"),
                )
                .clicked()
            {
                self.save();
            }
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Сохранить как...")).shortcut_text("Ctrl+Shift+S"),
                )
                .clicked()
            {
                self.save_as();
            }
            if ui
                .add_enabled(has_doc, egui::Button::new(crate::i18n::tr("Сохранить PDF...")))
                .on_hover_text(crate::i18n::tr("Схема всех плат и таблицы параметров блоков"))
                .clicked()
            {
                self.export_pdf();
            }
            if ui
                .button(crate::i18n::tr("Открывать .crab двойным щелчком"))
                .on_hover_text(crate::i18n::tr("Связать файлы проектов, наборов блоков (.crabblocks) и плат (.crabboard) с этим редактором в Проводнике"))
                .clicked()
            {
                match crate::assoc::register() {
                    Ok(()) => self.info(
                        LogLevel::Ok,
                        crate::i18n::tr("Файлы .crab теперь открываются этим редактором"),
                    ),
                    Err(e) => {
                        self.info(LogLevel::Error, crate::i18n::msg("Не удалось связать .crab: {0}", &[&e]))
                    }
                }
            }
            ui.menu_button(crate::i18n::tr("Автосохранение"), |ui| {
                let mut minutes = self.autosave_minutes;
                ui.radio_value(&mut minutes, 0, crate::i18n::tr("Выключено"));
                for m in [1, 2, 5, 10, 15, 30] {
                    ui.radio_value(&mut minutes, m, crate::i18n::fmt("Каждые {0} мин", &[&m]));
                }
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("Период:"));
                    ui.add(
                        egui::DragValue::new(&mut minutes)
                            .range(0..=240)
                            .suffix(crate::i18n::tr(" мин")),
                    );
                });
                if minutes != self.autosave_minutes {
                    self.set_autosave(minutes);
                }
            });
            ui.menu_button(crate::i18n::tr("Симуляция блоков"), |ui| self.sim_settings_menu(ui));
            if ui.button(crate::i18n::tr("Ядра ESP32...")).clicked() {
                self.cores_conf.show();
                ui.close();
            }
            ui.separator();
            if ui.button(crate::i18n::tr("Выход")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button(crate::i18n::tr("Примеры"), |ui| {
            let examples = self.example_projects();
            if examples.is_empty() {
                ui.label(crate::i18n::tr("Папка examples не найдена"));
            }
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for (name, path) in examples {
                        if ui.button(&name).clicked() {
                            self.open_example(&name, &path);
                            ui.close();
                        }
                    }
                });
        });
        ui.menu_button(crate::i18n::tr("Вид"), |ui| {
            let on = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            if ui
                .add(
                    egui::Button::new(crate::i18n::tr("На весь экран"))
                        .selected(on)
                        .shortcut_text("F11"),
                )
                .clicked()
            {
                self.toggle_fullscreen(ui.ctx());
                ui.close();
            }
            ui.separator();
            panels::theme_menu(self, ui);
            ui.separator();
            ui.menu_button("Language / Язык", |ui| {
                for lang in crate::i18n::Lang::ALL {
                    let on = crate::i18n::current() == lang;
                    if ui.radio(on, lang.native_name()).clicked() {
                        crate::i18n::set(lang);
                        theme::save_language(lang.code());
                        ui.close();
                    }
                }
            });
        });
        ui.menu_button(crate::i18n::tr("Правка"), |ui| {
            let (can_undo, can_redo) = self
                .doc
                .as_ref()
                .map_or((false, false), |d| (d.can_undo(), d.can_redo()));
            if ui
                .add_enabled(
                    can_undo,
                    egui::Button::new(crate::i18n::tr("Отменить")).shortcut_text("Ctrl+Z"),
                )
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(
                    can_redo,
                    egui::Button::new(crate::i18n::tr("Повторить")).shortcut_text("Ctrl+Y"),
                )
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.selection.is_some(),
                    egui::Button::new(crate::i18n::tr("Удалить")).shortcut_text("Del"),
                )
                .clicked()
            {
                self.delete_selection();
            }
            let has_blocks = self
                .selection
                .as_ref()
                .is_some_and(|s| !s.blocks().is_empty());
            if ui
                .add_enabled(
                    has_blocks,
                    egui::Button::new(crate::i18n::tr("Вырезать")).shortcut_text("Ctrl+X"),
                )
                .clicked()
            {
                self.cut_selection(ui.ctx());
            }
            if ui
                .add_enabled(
                    has_blocks,
                    egui::Button::new(crate::i18n::tr("Копировать")).shortcut_text("Ctrl+C"),
                )
                .clicked()
            {
                self.copy_selection(ui.ctx());
            }
            if ui
                .add_enabled(
                    self.doc.is_some(),
                    egui::Button::new(crate::i18n::tr("Вставить")).shortcut_text("Ctrl+V"),
                )
                .clicked()
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
        });
        ui.menu_button(crate::i18n::tr("Проект"), |ui| {
            let has_doc = self.doc.is_some();
            let idle = !self.job_running();
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Настройки МК...")),
                )
                .clicked()
            {
                self.open_board_settings(ui.ctx());
            }
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Конструктор веб-интерфейса...")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Страница управления на ESP32 (главная страница): откроется в браузере",
                ))
                .clicked()
            {
                self.open_web_designer();
            }
            ui.add_enabled_ui(has_doc, |ui| self.board_copy_menu(ui));
            ui.add_enabled_ui(has_doc, |ui| self.retain_menu(ui));
            ui.separator();
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Проверить")).shortcut_text("F7"),
                )
                .clicked()
            {
                self.run_check();
            }
            if ui
                .add_enabled(has_doc, egui::Button::new(crate::i18n::tr("Код C++")))
                .clicked()
            {
                self.code_view = Some(crate::codeview::CodeView::new());
            }
            if ui
                .add_enabled(
                    has_doc && idle,
                    egui::Button::new(crate::i18n::tr("Собрать")).shortcut_text("F5"),
                )
                .clicked()
            {
                self.start_job(JobKind::Build, ui.ctx());
            }
            if ui
                .add_enabled(
                    has_doc && idle,
                    egui::Button::new(crate::i18n::tr("Пересобрать полностью")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Собрать заново, даже если схема не менялась: \
                     после обновления библиотек или ядра ESP32",
                ))
                .clicked()
            {
                self.start_job(JobKind::Rebuild, ui.ctx());
            }
            if ui
                .add_enabled(
                    has_doc && idle,
                    egui::Button::new(crate::i18n::tr("Залить мастер-прошивку")),
                )
                .clicked()
            {
                let port = self.port_choice();
                self.start_job(JobKind::Upload { port }, ui.ctx());
            }
            if ui
                .add_enabled(
                    has_doc,
                    egui::Button::new(crate::i18n::tr("Симуляция схемы на ПК")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Запустить схему на компьютере без платы: входы задаются руками",
                ))
                .clicked()
            {
                self.start_sim();
            }
            if ui
                .add_enabled(
                    has_doc && idle,
                    egui::Button::new(crate::i18n::tr("Отладка: залить и подключиться")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Залить прошивку с модулем отладки и подключиться к плате",
                ))
                .clicked()
            {
                self.start_debug(ui.ctx());
            }
        });
        ui.menu_button(crate::i18n::tr("Теги"), |ui| {
            if ui
                .add_enabled(
                    self.doc.is_some(),
                    egui::Button::new(crate::i18n::tr("Перенести из проекта...")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Взять регистры Modbus, описанные блоками другого проекта",
                ))
                .clicked()
            {
                self.tag_import = Some(Default::default());
                ui.close();
            }
        });
        ui.menu_button(crate::i18n::tr("Библиотека"), |ui| {
            if ui.button(crate::i18n::tr("Новый блок...")).clicked() {
                self.block_editor = Some(crate::blockedit::BlockEditor::new_block(self, ""));
            }
            ui.weak(crate::i18n::tr(
                "Изменить блок — правой кнопкой в библиотеке",
            ));
            ui.separator();
            if ui.button(crate::i18n::tr("Новая плата...")).clicked() {
                self.board_editor = Some(crate::boardedit::BoardEditor::new_board(self));
            }
            let boards: Vec<_> = self.lib.boards().iter().cloned().collect();
            ui.add_enabled_ui(!boards.is_empty(), |ui| {
                ui.menu_button(crate::i18n::tr("Изменить плату"), |ui| {
                    for b in &boards {
                        if ui.button(crate::i18n::tr(&b.def.info.name)).clicked() {
                            self.board_editor = Some(crate::boardedit::BoardEditor::edit(b));
                            ui.close();
                        }
                    }
                });
                ui.menu_button(crate::i18n::tr("Экспорт платы"), |ui| {
                    for b in &boards {
                        if ui.button(crate::i18n::tr(&b.def.info.name)).clicked() {
                            crate::boardio::export(self, &b.def.info.id);
                            ui.close();
                        }
                    }
                });
            });
            if ui
                .button(crate::i18n::tr("Импорт платы из файла..."))
                .clicked()
            {
                crate::boardio::pick_files(self);
                ui.close();
            }
            ui.separator();
            if ui.button(crate::i18n::tr("Перечитать блоки")).clicked() {
                self.reload_library();
            }
        });
        ui.menu_button(crate::i18n::tr("Справка"), |ui| {
            if ui
                .add(egui::Button::new(crate::i18n::tr("Содержание")).shortcut_text("F1"))
                .clicked()
            {
                self.open_help(crate::help::START);
                ui.close();
            }
            if ui.button(crate::i18n::tr("Справочник блоков")).clicked() {
                self.open_help(crate::help::BLOCKS);
                ui.close();
            }
            if ui.button(crate::i18n::tr("Частые ошибки")).clicked() {
                self.open_help(crate::help::ERRORS);
                ui.close();
            }
            ui.separator();
            ui.weak(format!("CrabLogic {}", update::version_label()));
            ui.separator();
            if ui
                .add_enabled(
                    self.update_rx.is_none(),
                    egui::Button::new(crate::i18n::tr("Проверить обновления")),
                )
                .clicked()
            {
                self.start_update_check(ui.ctx(), true);
                ui.close();
            }
            if ui
                .checkbox(
                    &mut self.update_check,
                    crate::i18n::tr("Проверять при запуске"),
                )
                .changed()
            {
                theme::save_update_check(self.update_check);
            }
            if ui
                .checkbox(
                    &mut self.update_beta,
                    crate::i18n::tr("Получать бета-версии"),
                )
                .on_hover_text(crate::i18n::tr(
                    "Предлагать и пробные версии (пререлизы на GitHub); они могут быть нестабильны",
                ))
                .changed()
            {
                theme::save_update_beta(self.update_beta);
            }
            if ui
                .checkbox(
                    &mut self.auto_update,
                    crate::i18n::tr("Скачивать обновления автоматически"),
                )
                .on_hover_text(crate::i18n::tr(
                    "Найденная версия скачивается в фоне; устанавливается по вашей команде",
                ))
                .changed()
            {
                theme::save_auto_update(self.auto_update);
                if self.auto_update {
                    self.start_known_release_download();
                }
            }
            if let SelfUpdate::Ready(staged) = &self.self_update {
                let staged = staged.clone();
                if ui
                    .button(crate::i18n::tr("Перезапустить и обновить"))
                    .clicked()
                {
                    self.apply_update(ui.ctx(), &staged);
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .button(crate::i18n::tr("Сообщить об ошибке"))
                .on_hover_text(crate::i18n::tr("Форма: описание уйдёт на GitHub"))
                .clicked()
            {
                self.report.open = true;
                ui.close();
            }
            if ui
                .button(crate::i18n::tr("Что нового в этой версии"))
                .clicked()
            {
                self.changes_window = true;
                ui.close();
            }
            if let Some(release) = &self.new_release {
                ui.separator();
                if ui
                    .button(crate::i18n::fmt("Что нового в {0}", &[&release.version]))
                    .clicked()
                {
                    self.update_window = true;
                    ui.close();
                }
                ui.hyperlink_to(
                    crate::i18n::fmt("Скачать версию {0}", &[&release.version]),
                    &release.url,
                );
            }
        });
    }

    pub fn open_board_settings(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        let users = crab_core::uart::block_uarts(&doc.project.logic, &self.lib)
            .into_iter()
            .map(|u| (u.number, u.block))
            .collect();
        let from_blocks = crab_core::uart::block_configs(&doc.project.logic, &self.lib)
            .into_iter()
            .map(|(id, u)| (u.number, id))
            .collect();
        let dialog = crate::boardcfg::BoardDialog::new(
            &doc.project.config.board,
            &doc.project.config.uart,
            users,
            from_blocks,
            self.lib.boards(),
            ctx,
        );
        self.dialog = Some(Dialog::Board(dialog));
        self.scan_ports(ctx);
    }

    /// Port typed in the toolbar, or `None` to use the project settings / autodetect.
    pub fn port_choice(&self) -> Option<String> {
        let port = self.port.trim();
        (!port.is_empty()).then(|| port.to_owned())
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let mut cancel = false;
        let mut install = None;
        ui.horizontal(|ui| {
            self.status_items(ui, &mut cancel);
            if let Some(release) = &self.new_release {
                ui.separator();
                let link = ui
                    .link(crate::i18n::fmt("Доступна версия {0}", &[&release.version]))
                    .on_hover_text(crate::i18n::tr("Показать, что изменилось"));
                if link.clicked() {
                    self.update_window = true;
                }
                // Downloaded: installing waits for the user's yes.
                if let SelfUpdate::Ready(staged) = &self.self_update {
                    if ui
                        .small_button(crate::i18n::tr("Установить и перезапустить"))
                        .on_hover_text(crate::i18n::tr(
                            "Проект сохранится, редактор перезапустится уже новым",
                        ))
                        .clicked()
                    {
                        install = Some(staged.clone());
                    }
                }
                if let SelfUpdate::Downloading { done, total, .. } = &self.self_update {
                    let (done, total) =
                        (done.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
                    let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0).replace('.', ",");
                    let (fraction, text) = if total > 0 {
                        (
                            done as f32 / total as f32,
                            crate::i18n::fmt("{0} из {1} Мб", &[&mb(done), &mb(total)]),
                        )
                    } else {
                        (0.0, crate::i18n::fmt("{0} Мб", &[&mb(done)]))
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(110.0)
                            .desired_height(12.0)
                            .text(egui::RichText::new(text).small()),
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("v{}", update::version_label()));
            });
        });
        if let Some(staged) = install {
            self.apply_update(&ui.ctx().clone(), &staged);
        }
        if cancel {
            self.cancel_job();
        }
    }

    pub(super) fn status_items(&self, ui: &mut egui::Ui, cancel: &mut bool) {
        match &self.doc {
            Some(doc) => {
                let p = &doc.project;
                ui.label(crate::i18n::fmt("Проект: {0}", &[&p.config.project.name]));
                ui.separator();
                let board = &p.config.board;
                ui.label(crate::i18n::fmt(
                    "Плата: {0}",
                    &[&crate::boardcfg::board_name(board, self.lib.boards())],
                ))
                .on_hover_text(&board.fqbn);
                ui.separator();
                ui.label(crate::i18n::fmt("Блоков: {0}", &[&p.logic.blocks.len()]));
                ui.separator();
                let errors = doc.report.errors().count();
                if errors == 0 {
                    ui.label(
                        RichText::new(crate::i18n::tr("Схема без ошибок"))
                            .color(self.theme.palette.ok),
                    );
                } else {
                    ui.label(
                        RichText::new(crate::i18n::fmt("Ошибок в схеме: {0}", &[&errors]))
                            .color(self.theme.palette.error),
                    );
                }
                if let Some(check) = self.board_check.as_ref().filter(|c| !c.lines.is_empty()) {
                    ui.separator();
                    ui.label(
                        RichText::new(crate::i18n::tr("Плата: не всё ускорено"))
                            .color(self.theme.palette.warning),
                    )
                    .on_hover_text(check.lines.join(
                        "
",
                    ));
                }
                if doc.dirty {
                    ui.separator();
                    ui.label(crate::i18n::tr("Есть несохранённые изменения"));
                }
                if let Some(job) = &self.job {
                    ui.separator();
                    panels::busy(ui);
                    ui.label(job.title);
                    if ui.small_button(crate::i18n::tr("Отменить")).clicked() {
                        *cancel = true;
                    }
                }
                if let Some(d) = &self.debug {
                    ui.separator();
                    let color = if d.online() {
                        self.theme.palette.ok
                    } else {
                        self.theme.palette.warning
                    };
                    ui.label(
                        RichText::new(crate::i18n::fmt(
                            "Отладка {0}: {1}",
                            &[&d.port, &d.live.status],
                        ))
                        .color(color),
                    );
                    match d.same_project {
                        Some(true) => {
                            ui.label(crate::i18n::tr("Проект совпадает с платой"));
                        }
                        Some(false) => {
                            ui.label(
                                RichText::new(crate::i18n::tr("Проект отличается от платы"))
                                    .color(self.theme.palette.warning),
                            );
                        }
                        None => {}
                    }
                    if let Some(bad) = d.live.master_safe {
                        ui.label(
                            RichText::new(crate::job::safe_mode_message(bad))
                                .color(self.theme.palette.warning),
                        );
                    }
                    if let Some(nvs) = &d.live.nvs {
                        let (text, warn) = crate::job::nvs_message(nvs);
                        let mut label = RichText::new(text);
                        if warn {
                            label = label.color(self.theme.palette.warning);
                        }
                        ui.label(label);
                    }
                    // The master on the board against the image of the program (`specs/rust-only`).
                    if let Some(id) = &d.live.master_id {
                        let image = crab_build::flash::image_version(&p.config.board.fqbn);
                        match crab_build::flash::master_verdict(Some(id), image.as_deref()) {
                            crab_build::flash::MasterVerdict::Current => {
                                ui.label(crate::i18n::tr("Мастер на плате актуален"));
                            }
                            crab_build::flash::MasterVerdict::Older(have, want) => {
                                ui.label(
                                    RichText::new(crate::i18n::fmt(
                                        "Мастер на плате версии {0}, в программе {1}: залейте мастер кнопкой ⬆",
                                        &[&have, &want],
                                    ))
                                    .color(self.theme.palette.warning),
                                );
                            }
                            crab_build::flash::MasterVerdict::NotRust => {
                                ui.label(
                                    RichText::new(crate::i18n::tr(
                                        "На плате не мастер на Rust: залейте мастер кнопкой ⬆",
                                    ))
                                    .color(self.theme.palette.warning),
                                );
                            }
                        }
                    }
                    if d.live.needs_password {
                        let id = egui::Id::unique("debug_password_input");
                        let mut text: String = ui.data_mut(|m| m.get_temp(id).unwrap_or_default());
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .password(true)
                                .desired_width(110.0)
                                .hint_text(crate::i18n::tr("Пароль отладки")),
                        );
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let go = ui.small_button(crate::i18n::tr("Войти")).clicked();
                        if (enter || go) && !text.is_empty() {
                            d.send(crab_debug::Command::Password(text.clone()));
                        }
                        ui.data_mut(|m| m.insert_temp(id, text));
                    }
                    if let Some(s) = d.live.stats.as_ref().filter(|_| d.online()) {
                        ui.separator();
                        ui.label(crate::i18n::fmt(
                            "Цикл {0} мкс, макс {1} мкс",
                            &[&s.last_us, &s.max_us],
                        ));
                        if let Some(bg) = &s.background {
                            ui.label(crate::i18n::fmt(
                                "Ядро 0: {0} мкс, макс {1} мкс",
                                &[&bg.last_us, &bg.max_us],
                            ))
                            .on_hover_text(crate::i18n::tr(
                                "Проход фоновой задачи на ядре 0: сеть и фоновые платы",
                            ));
                        }
                    }
                    if let Some(ip) = d.device_address(&p.logic.blocks) {
                        ui.separator();
                        ui.hyperlink_to(&ip, format!("http://{ip}")).on_hover_text(
                            crate::i18n::tr("Открыть страницу устройства в браузере"),
                        );
                    }
                }
            }
            None => {
                ui.label(crate::i18n::tr("Проект не открыт"));
            }
        }
    }

    pub(super) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let mut decision = None;
        egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
            ui.heading(crate::i18n::tr("Несохранённые изменения"));
            ui.label(crate::i18n::tr("Сохранить изменения в проекте?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("Сохранить")).clicked() {
                    decision = Some(Some(true));
                }
                if ui.button(crate::i18n::tr("Не сохранять")).clicked() {
                    decision = Some(Some(false));
                }
                if ui.button(crate::i18n::tr("Отмена")).clicked() {
                    decision = Some(None);
                }
            });
        });
        let Some(decision) = decision else { return };
        self.pending = None;
        let Some(save) = decision else { return };
        if save {
            self.save();
            if self.doc.as_ref().is_some_and(|d| d.dirty) {
                return; // saving failed, keep the project open
            }
        }
        match pending {
            Pending::Open(path) => self.open(&path),
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(super) fn handle_close(&mut self, ctx: &egui::Context) {
        let close = ctx.input(|i| i.viewport().close_requested());
        if !close || self.allow_close {
            return;
        }
        // An open block editor with unsaved changes asks first; the project is
        // checked on the next close.
        if let Some(ed) = self.block_editor.as_mut().filter(|e| e.dirty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ed.ask_to_close();
        } else if let Some(ed) = self.board_editor.as_mut().filter(|e| e.dirty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ed.ask_to_close();
        } else if self.doc.as_ref().is_some_and(|d| d.dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        }
    }

    pub(super) fn window_title(&self) -> String {
        match &self.doc {
            Some(doc) => format!(
                "{}{} — CrabLogic",
                doc.project.config.project.name,
                if doc.dirty { " *" } else { "" }
            ),
            None => "CrabLogic".to_owned(),
        }
    }

    /// Comments of the generated sketch follow the interface: Russian for the Russian
    /// interface, English for all others.
    pub(super) fn sync_comment_lang(&mut self) {
        let want = if crate::i18n::current() == crate::i18n::Lang::Ru {
            ""
        } else {
            "en"
        };
        if let Some(doc) = self.doc.as_mut() {
            if doc.project.config.project.comment_lang != want {
                doc.project.config.project.comment_lang = want.to_owned();
                doc.revision += 1;
            }
        }
    }

    pub(super) fn update_title(&self, ctx: &egui::Context) {
        let title = self.window_title();
        // A viewport command always forces a repaint, so send it only on change;
        // otherwise the window redraws non-stop (~30 fps) and loads the GPU at idle.
        let id = egui::Id::unique("window_title");
        if ctx.data(|d| d.get_temp::<String>(id)).as_deref() != Some(title.as_str()) {
            ctx.data_mut(|d| d.insert_temp(id, title.clone()));
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}
