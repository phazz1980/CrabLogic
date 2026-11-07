//! "Block editor" window: create a library block or change one — header, inputs,
//! outputs, parameters, the ESP32 code template and docs — with a live picture of
//! the block and a trial generation of its code.

use std::fs;
use std::path::{Path, PathBuf};

use crab_core::ident::ident_from_name;
use crab_core::library::ESP32_CPP_TEMPLATE;
use crab_core::vars::{VAR_GET, VAR_SET};
use crab_core::{
    BlockDef, Choice, LibraryBlock, ParamDef, ParamKind, ParamShow, ParamValue, PinMode, PortDef,
    SignalType,
};
use eframe::egui::{self, Color32, Id, RichText, ScrollArea, TextEdit, Ui};

use crab_core::native_wizard::{LibraryWizard, WizType};

use crate::app::{EditorApp, LogLevel};
use crate::dialogs::{type_name, value_text};
use crate::pictures::Pictures;
use crate::theme::Palette;

mod attach;
mod code;
mod docs;
mod form;
mod native;
mod rows;
mod save;
mod simbuild;
mod snippets;
#[cfg(test)]
mod tests;

pub use self::docs::docs_editor;
pub use self::save::delete_dialog;
use self::{
    attach::*, code::*, docs::*, form::*, native::*, rows::*, save::*, simbuild::*, snippets::*,
};

const PARAM_KINDS: [ParamKind; 6] = [
    ParamKind::Bool,
    ParamKind::Int,
    ParamKind::Float,
    ParamKind::Text,
    ParamKind::Duration,
    ParamKind::Pin,
];

const NEW_TEMPLATE: &str =
    "// @global\n\n// @setup\n\n// @loop\n{{ output.out }} = {{ input.in }};\n";

const SECTIONS_HINT: &str = "Разделы: // @global — переменные блока, // @setup — код в setup(), \
// @loop — код цикла, // @functions — функции. С пометкой once (// @global once) раздел \
попадает в скетч один раз на все блоки. Имена в разделах без once начинайте с {{ id }}.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Ports,
    Params,
    Code,
    Native,
    Docs,
    Files,
    Sketch,
    Trial,
}

pub struct BlockEditor {
    /// Type the form was loaded from; `None` for a block not saved yet.
    original: Option<String>,
    form: Form,
    /// The form as last loaded or saved.
    clean: Form,
    tab: Tab,
    trial: crate::trialrun::TrialRun,
    sim_build: SimBuild,
    /// Test build of the master with the native part, on the "Код C++ для платы" tab.
    native_build: NativeBuild,
    /// Step of the native-part wizard being shown (0-based).
    wizard_step: usize,
    generated: Option<Generated>,
    /// Closing with unsaved changes: asking what to do.
    confirm_close: bool,
    /// The window fills the whole application window.
    maximized: bool,
    /// A built-in block: shown, but cannot be changed.
    read_only: bool,
    /// Sections, packs and folders a new block may be put into.
    folders: Vec<String>,
    /// Names of inputs, outputs and parameters as of the last frame, to follow renames.
    names: Vec<(&'static str, String)>,
}

impl BlockEditor {
    /// A new block in a pack or folder of the user library (`""` — the default pack).
    pub fn new_block(app: &EditorApp, group: &str) -> Self {
        let group = if group.is_empty() {
            crab_core::pack::DEFAULT_PACK
        } else {
            group
        };
        let mut form = Form::new_block(String::new(), group.to_owned());
        form.kind = free_kind(app, &ident_from_name(&form.name));
        Self {
            folders: user_folders(app),
            ..Self::with(None, form)
        }
    }

    /// Opens a library block: a user block for editing, a built-in one read-only.
    pub fn edit(app: &EditorApp, block: &LibraryBlock) -> Self {
        let kind = block.def.info.kind.clone();
        let read_only = !app.lib.is_user(&kind);
        Self {
            read_only,
            ..Self::with(Some(kind), Form::from_block(block))
        }
    }

    /// A new block based on an existing one.
    /// A copy of a user block lands next to it, of a built-in one in the default pack.
    pub fn copy_of(app: &EditorApp, block: &LibraryBlock) -> Self {
        let mut form = Form::from_block(block);
        if !app.lib.is_user(&block.def.info.kind) {
            form.group = crab_core::pack::DEFAULT_PACK.to_owned();
        }
        form.attach = Attach::copied_from(&block.dir);
        form.name = crate::i18n::fmt("{0} (копия)", &[&form.name]);
        form.kind = free_kind(app, &ident_from_name(&form.name));
        Self {
            folders: user_folders(app),
            ..Self::with(None, form)
        }
    }

    fn with(original: Option<String>, form: Form) -> Self {
        Self {
            original,
            clean: form.clone(),
            form,
            tab: Tab::Ports,
            trial: Default::default(),
            sim_build: SimBuild::default(),
            native_build: NativeBuild::default(),
            wizard_step: 0,
            generated: None,
            confirm_close: false,
            maximized: false,
            read_only: false,
            folders: Vec::new(),
            names: Vec::new(),
        }
    }

    /// When a port or parameter is renamed, its references in the template follow,
    /// so the code tab does not fail on the old name while the new one is typed.
    fn follow_renames(&mut self) {
        let now = self.form.row_names();
        if self.names.len() != now.len() {
            self.names = now;
            return;
        }
        for (old, new) in self.names.iter_mut().zip(now) {
            // A field cleared on the way to a new name keeps the old name until it is typed.
            if new.1.is_empty() || old.0 != new.0 || old.1 == new.1 {
                continue;
            }
            self.form.template = rename_in_template(&self.form.template, old.0, &old.1, &new.1);
            old.1 = new.1;
        }
    }

    pub fn dirty(&self) -> bool {
        !self.read_only && self.form != self.clean
    }

    /// Asks "save changes?" on the next frame, as when its window is closed.
    pub fn ask_to_close(&mut self) {
        self.confirm_close = true;
    }

    /// Regenerates the trial sketch when the definition or template changed.
    fn update_generated(&mut self, def: &BlockDef) {
        let fresh = self
            .generated
            .as_ref()
            .is_some_and(|g| g.def == *def && g.template == self.form.template);
        if !fresh {
            let result =
                crab_codegen::preview_block(def, &self.form.template).map_err(|e| e.to_string());
            self.generated = Some(Generated {
                def: def.clone(),
                template: self.form.template.clone(),
                result,
            });
        }
    }
}

/// A type not used by any block, made from `base`.
fn free_kind(app: &EditorApp, base: &str) -> String {
    let taken = |k: &str| app.lib.get(k).is_some() || k == VAR_GET || k == VAR_SET;
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|k| !taken(k))
        .expect("some name is free")
}

pub fn show(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(mut ed) = app.block_editor.take() else {
        return;
    };
    let title = match &ed.original {
        Some(kind) if ed.read_only => {
            crate::i18n::fmt("Встроенный блок — {0} (только просмотр)", &[&kind])
        }
        Some(kind) => crate::i18n::fmt("Редактор блока — {0}", &[&kind]),
        None => crate::i18n::tr("Редактор блока — новый блок").to_owned(),
    };
    let mut open = true;
    let mut close = false;
    let mut window = egui::Window::new(title)
        .id(Id::unique("block_editor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(900.0)
        .default_height(640.0);
    if ed.maximized {
        let area = ctx.content_rect().shrink(4.0);
        window = window
            .fixed_pos(area.min)
            .fixed_size(area.size() - egui::vec2(16.0, 36.0));
    }
    window.show(ctx, |ui| {
        let label = if ed.maximized {
            crate::i18n::tr("🗗 Обычный размер")
        } else {
            crate::i18n::tr("🗖 На всё окно")
        };
        if ui.small_button(label).clicked() {
            ed.maximized = !ed.maximized;
        }
        close = body(app, ui, &mut ed);
    });
    if !open && ed.dirty() {
        ed.confirm_close = true;
    } else if !open {
        close = true;
    }
    if ed.confirm_close && !close {
        close = confirm_close(app, ctx, &mut ed);
    }
    if !close {
        app.block_editor = Some(ed);
    }
}

/// Window contents; returns true when the window should close.
fn body(app: &mut EditorApp, ui: &mut Ui, ed: &mut BlockEditor) -> bool {
    let pal = app.theme.palette;
    // Until the first save the type follows the name.
    if ed.original.is_none() {
        let kind = free_kind(app, &ident_from_name(&ed.form.name));
        ed.clean.kind.clone_from(&kind);
        ed.form.kind = kind;
    }
    if !ed.read_only {
        ed.follow_renames();
    }
    let def = ed.form.to_def();
    if let Ok(def) = &def {
        ed.update_generated(def);
    }

    // Panels inside the window: the footer stays visible and the tab gets the rest.
    let mut close = false;
    let mut help = None;
    egui::Panel::bottom("block_editor_footer").show(ui, |ui| {
        status(ui, ed, &def, &pal);
        close = footer(app, ui, ed, &def);
    });
    egui::Panel::top("block_editor_header").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(crate::i18n::tr("Вид на плате")).strong());
                match &def {
                    Ok(def) => crate::canvas::block_preview(ui, def, &pal),
                    Err(_) => {
                        ui.weak(crate::i18n::tr("(исправьте ошибки описания)"));
                    }
                }
            });
            ui.add_space(16.0);
            let editable = !ed.read_only;
            ui.vertical(|ui| ui.add_enabled_ui(editable, |ui| header(ui, ed)));
        });
        ui.separator();
        ui.horizontal(|ui| {
            let tabs = [
                (Tab::Ports, crate::i18n::tr("Входы и выходы")),
                (Tab::Params, crate::i18n::tr("Параметры")),
                (Tab::Code, crate::i18n::tr("Код ESP32")),
                (Tab::Native, crate::i18n::tr("Код C++ для платы")),
                (Tab::Docs, crate::i18n::tr("Описание")),
                (Tab::Files, crate::i18n::tr("Библиотека и пример")),
                (Tab::Sketch, crate::i18n::tr("Пробная генерация")),
                (Tab::Trial, crate::i18n::tr("Пробный прогон")),
            ];
            for (tab, text) in tabs {
                ui.selectable_value(&mut ed.tab, tab, text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(crate::i18n::tr("? Справка"))
                    .on_hover_text(crate::i18n::tr(
                        "Справка по редактору блока: открывается на текущей вкладке",
                    ))
                    .clicked()
                {
                    let anchor = match ed.tab {
                        Tab::Ports => crate::i18n::tr("входы-и-выходы"),
                        Tab::Params => crate::i18n::tr("параметры"),
                        Tab::Code => crate::i18n::tr("код-esp32"),
                        Tab::Native => crate::i18n::tr("код-c-для-платы"),
                        Tab::Docs => crate::i18n::tr("описание"),
                        Tab::Files => crate::i18n::tr("библиотека-и-пример"),
                        Tab::Sketch => crate::i18n::tr("пробная-генерация"),
                        Tab::Trial => crate::i18n::tr("пробный-прогон"),
                    };
                    help = Some(format!("{}#{anchor}", crate::help::BLOCK_EDITOR));
                }
            });
        });
    });
    let editable = !ed.read_only;
    // The board of the open project (ESP32 DevKit without one) for the test build of the native part.
    let board = app.doc.as_ref().map_or_else(
        || crab_core::BoardConfig {
            fqbn: "esp32:esp32:esp32".into(),
            model: None,
            port: None,
            options: Default::default(),
        },
        |d| d.project.config.board.clone(),
    );
    let trial_dir = ed
        .original
        .as_deref()
        .and_then(|k| app.lib.get(k))
        .map(|b| b.dir.clone());
    egui::CentralPanel::default().show(ui, |ui| {
        ScrollArea::both()
            .id_salt("block_editor_tab")
            .auto_shrink([false, false])
            .show(ui, |ui| match ed.tab {
                Tab::Ports => {
                    ui.add_enabled_ui(editable, |ui| ports_tab(ui, &mut ed.form));
                }
                Tab::Params => {
                    ui.add_enabled_ui(editable, |ui| params_tab(ui, &mut ed.form));
                }
                Tab::Code => code_tab(ui, &mut ed.form, editable),
                Tab::Native => native_tab(ui, ed, &def, &board, &pal),
                Tab::Docs => docs_tab(ui, &mut ed.form, editable),
                Tab::Files => {
                    ui.add_enabled_ui(editable, |ui| files_tab(ui, &mut ed.form));
                }
                Tab::Sketch => sketch_tab(ui, ed, &pal),
                Tab::Trial => trial_tab(ui, ed, trial_dir.as_deref(), &pal),
            });
    });
    if let Some(target) = help {
        app.open_help(&target);
    }
    close
}

/// Folders a new block can go into: the existing ones and the default pack.
fn user_folders(app: &EditorApp) -> Vec<String> {
    let mut folders: Vec<String> = app.lib.user_folders().map(str::to_owned).collect();
    if !folders.iter().any(|f| f == crab_core::pack::DEFAULT_PACK) {
        folders.push(crab_core::pack::DEFAULT_PACK.to_owned());
        folders.sort();
    }
    folders
}

fn header(ui: &mut Ui, ed: &mut BlockEditor) {
    let read_only = ed.read_only;
    let f = &mut ed.form;
    egui::Grid::new("block_header")
        .num_columns(4)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Название"));
            ui.add_sized([220.0, 20.0], TextEdit::singleline(&mut f.name))
                .on_hover_text(crate::i18n::fmt(
                    "Тип блока в схеме: {0}. У нового блока он берётся из названия, \
                     после сохранения не меняется.",
                    &[&f.kind],
                ));
            ui.label(crate::i18n::tr("Обозначение"));
            ui.add_sized(
                [160.0, 20.0],
                TextEdit::singleline(&mut f.symbol).hint_text(crate::i18n::tr("как название")),
            )
            .on_hover_text(crate::i18n::tr("Короткая надпись на блоке: TON, a + b, &"));
            ui.end_row();

            ui.label(crate::i18n::tr("Где лежит"));
            let place = if read_only {
                crate::i18n::fmt("Встроенные / {0}", &[&f.group])
            } else {
                crate::i18n::fmt("Блоки пользователей / {0}", &[&f.group.replace('/', " / ")])
            };
            if ed.original.is_none() && !read_only {
                // A new block: choose the pack (or a folder in it) it goes into.
                egui::ComboBox::from_id_salt("block_place")
                    .selected_text(place)
                    .show_ui(ui, |ui| {
                        for folder in &ed.folders {
                            ui.selectable_value(
                                &mut f.group,
                                folder.clone(),
                                folder.replace('/', " / "),
                            );
                        }
                    })
                    .response
                    .on_hover_text(crate::i18n::tr(
                        "Набор, раздел или папка, куда сохранить новый блок",
                    ));
            } else {
                ui.label(place).on_hover_text(crate::i18n::tr(
                    "Раздел и папка в библиотеке. Перенести блок — правая кнопка на нём \
                     во вкладке «Пользовательские».",
                ));
            }
            ui.end_row();

            ui.label(crate::i18n::tr("Описание"));
            ui.add_sized([160.0, 20.0], TextEdit::singleline(&mut f.description));
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.memory, crate::i18n::tr("Хранит состояние"))
                    .on_hover_text(crate::i18n::tr(
                        "Блок помнит значения между циклами (таймеры, триггеры, счётчики); \
                         через такой блок допустимы обратные связи",
                    ));
                ui.checkbox(&mut f.single, crate::i18n::tr("Один на проект"))
                    .on_hover_text(crate::i18n::tr(
                        "В проекте может быть только один такой блок (Wi-Fi, NTP)",
                    ));
            });
            ui.end_row();

            ui.label(crate::i18n::tr("Библиотеки"));
            ui.add_sized(
                [220.0, 20.0],
                TextEdit::singleline(&mut f.libraries).hint_text("DHT sensor library"),
            )
            .on_hover_text(crate::i18n::tr(
                "Библиотеки arduino-cli через запятую, можно с версией: OneWire@2.3.8. \
                 Сборка ставит недостающие сама.",
            ));
            ui.end_row();
        });
}
