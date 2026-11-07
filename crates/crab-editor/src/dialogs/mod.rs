//! Pop-up windows with apply and cancel buttons: block properties, variable, sheet.
//!
//! Changes are buffered in the dialog and applied as one undo step on ✓.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crab_core::expander::{free_expander_id, pin_param, taken_pins};
use crab_core::pins::{pin_uses, BoardLayout, PinUse};
use crab_core::value::{format_duration_ms, join_min_sec_ms, parse_duration_ms, split_min_sec_ms};
use crab_core::{
    param_input_type, BlockDef, Expander, ExpanderChip, LocalValues, Logic, ParamDef, ParamKind,
    ParamShow, ParamValue, PortDef, SheetInfo, SignalType, Var, VarType,
};
use eframe::egui::{self, Color32, RichText, Ui};

use crate::app::{EditorApp, LogLevel, Selection};

mod block;
mod expander;
mod forms;
mod pins;
#[cfg(test)]
mod tests;

use self::{block::*, expander::*, forms::*, pins::*};

pub enum Dialog {
    Block(BlockDialog),
    Var(VarDialog),
    Sheet(SheetDialog),
    Board(crate::boardcfg::BoardDialog),
    Expander(ExpanderDialog),
}

/// An expander of the project tree: new (`original` is `None`) or being changed.
pub struct ExpanderDialog {
    original: Option<String>,
    form: Expander,
}

pub struct BlockDialog {
    id: String,
    new_id: String,
    kind: String,
    /// Parameter values; `None` means "use the default". Values of `local`
    /// parameters (passwords) come from [`LocalValues`] and go back there.
    values: BTreeMap<String, Option<ParamValue>>,
    /// Parameters that are inputs of the block instead of constants.
    param_in: BTreeSet<String>,
    /// Text being typed for string and duration parameters.
    texts: BTreeMap<String, String>,
    /// Secret parameters shown as plain text (the eye button).
    revealed: BTreeSet<String>,
    /// Page names of the web interface, for the `page` parameter of a widget (empty otherwise).
    pages: Vec<String>,
    /// Where the pin parameter points: a GPIO of the board or a pin of an expander.
    source: PinSource,
    /// The expander being declared while `source` is [`PinSource::New`].
    pending: Expander,
}

/// What the pin parameter of a digital input or output refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinSource {
    Gpio,
    /// An expander of the project, by id.
    Expander(String),
    /// An expander created together with the block.
    New,
}

pub struct VarDialog {
    /// Name before editing; `None` for a new variable.
    original: Option<String>,
    name: String,
    ty: VarType,
    /// Block the dialog was opened from and its write enable input (per block).
    block: Option<(String, bool)>,
    init: String,
    description: String,
    retain: bool,
}

pub struct SheetDialog {
    number: u32,
    name: String,
    comment: String,
    background: bool,
    period: u32,
    disabled: bool,
}

enum Outcome {
    Open,
    Apply,
    Cancel,
    Delete,
}

impl Dialog {
    pub fn for_block(app: &EditorApp, id: &str) -> Option<Self> {
        let doc = app.doc.as_ref()?;
        // A copy of a tag is edited through the tag: the settings are shared.
        let id = doc.project.logic.tag_of(id);
        let block = doc.project.logic.block(id)?;
        if let Some(var) = block.var_name() {
            let en = matches!(
                block.params.get(crab_core::vars::WRITE_ENABLE),
                Some(ParamValue::Bool(true))
            );
            let mut dialog = Self::for_var(app, Some(var))?;
            if let Self::Var(d) = &mut dialog {
                d.block = Some((id.to_owned(), en));
            }
            return Some(dialog);
        }
        let def = &app.lib.resolve(&doc.project.logic, block)?.def;
        let mut block = block.clone();
        app.local.fill_block(def, &mut block);
        let values = def
            .params
            .iter()
            .map(|p| (p.name.clone(), block.params.get(&p.name).cloned()))
            .collect();
        let texts = def
            .params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::Text | ParamKind::Duration))
            .map(|p| {
                let v = block.params.get(&p.name).or(p.default.as_ref());
                let text = match v {
                    Some(ParamValue::Text(s)) => s.clone(),
                    _ => String::new(),
                };
                (p.name.clone(), text)
            })
            .collect();
        Some(Self::Block(BlockDialog {
            id: id.to_owned(),
            new_id: id.to_owned(),
            kind: block.kind.clone(),
            values,
            param_in: block.param_in.clone(),
            texts,
            revealed: BTreeSet::new(),
            pages: if is_widget_kind(&block.kind) {
                crate::netpanel::page_names(&doc.project.logic)
            } else {
                Vec::new()
            },
            source: block
                .expander
                .clone()
                .map_or(PinSource::Gpio, PinSource::Expander),
            pending: new_expander(Some(&doc.project.logic)),
        }))
    }

    /// Window for a new web widget on the given page (0-based).
    pub fn for_var(app: &EditorApp, name: Option<&str>) -> Option<Self> {
        let var = match name {
            Some(n) => app.doc.as_ref()?.project.logic.var(n)?.clone(),
            None => Var {
                name: String::new(),
                ty: VarType::Float,
                init: None,
                description: String::new(),
                retain: false,
                service: false,
            },
        };
        Some(Self::Var(VarDialog {
            original: name.map(str::to_owned),
            init: var.init.as_ref().map(value_text).unwrap_or_default(),
            name: var.name,
            ty: var.ty,
            block: None,
            description: var.description,
            retain: var.retain,
        }))
    }

    /// Window of an expander of the project; `None` declares a new one.
    pub fn for_expander(app: &EditorApp, id: Option<&str>) -> Option<Self> {
        let logic = &app.doc.as_ref()?.project.logic;
        let form = match id {
            Some(id) => logic.expander(id)?.clone(),
            None => new_expander(Some(logic)),
        };
        Some(Self::Expander(ExpanderDialog {
            original: id.map(str::to_owned),
            form,
        }))
    }

    pub fn for_sheet(app: &EditorApp, number: u32) -> Self {
        let info = app
            .doc
            .as_ref()
            .and_then(|d| d.project.layout.sheet(number))
            .cloned()
            .unwrap_or_default();
        Self::Sheet(SheetDialog {
            number,
            name: info.name,
            comment: info.comment,
            background: info.background,
            disabled: info.disabled,
            period: if info.period == 0 {
                crab_core::layout::DEFAULT_BACKGROUND_PERIOD_MS
            } else {
                info.period
            },
        })
    }
}

/// Shows the open dialog, if any, and applies or drops it.
pub fn show(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let title = match &dialog {
        Dialog::Block(d) => crate::i18n::fmt("Свойства блока «{0}»", &[&d.id]),
        Dialog::Var(d) => match &d.original {
            Some(n) => crate::i18n::fmt("Переменная «{0}»", &[&n]),
            None => crate::i18n::tr("Новая переменная").to_owned(),
        },
        Dialog::Sheet(d) => crate::i18n::fmt("Плата {0}", &[&d.number]),
        Dialog::Board(_) => crate::i18n::tr("Настройки МК").to_owned(),
        Dialog::Expander(d) => match &d.original {
            Some(id) => crate::i18n::fmt("Расширитель «{0}»", &[&id]),
            None => crate::i18n::tr("Новый расширитель").to_owned(),
        },
    };
    let mut open = true;
    let mut outcome = Outcome::Open;
    // Block properties can fill the whole window. The maximized window has its own id,
    // so the normal one keeps its size and position for the restore.
    let full_id = egui::Id::unique("block_dialog_full");
    let mut full: bool = ctx.data(|d| d.get_temp(full_id)).unwrap_or(false);
    let is_block = matches!(dialog, Dialog::Block(_));
    let mut window = egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(380.0);
    if is_block && full {
        let screen = ctx.content_rect();
        window = window
            .id(egui::Id::unique("block_dialog_maximized"))
            .fixed_pos(screen.min + egui::vec2(4.0, 4.0))
            .fixed_size(screen.size() - egui::vec2(24.0, 56.0));
    }
    window.show(ctx, |ui| {
        if is_block {
            let label = if full {
                "Свернуть окно"
            } else {
                "На весь экран"
            };
            if ui.button(crate::i18n::tr(label)).clicked() {
                full = !full;
                ui.ctx().data_mut(|d| d.insert_temp(full_id, full));
            }
        }
        outcome = buttons(
            ui,
            match &dialog {
                Dialog::Var(d) => d.original.is_some(),
                Dialog::Expander(d) => d.original.is_some(),
                _ => false,
            },
        );
        ui.separator();
        match &mut dialog {
            Dialog::Block(d) => block_body(app, ui, d),
            Dialog::Var(d) => var_body(ui, d),
            Dialog::Sheet(d) => sheet_body(ui, d),
            Dialog::Board(d) => {
                d.body(ui, &app.ports);
                if let Some(request) = d.take_editor_request() {
                    app.board_editor = match request {
                        None => Some(crate::boardedit::BoardEditor::new_board(app)),
                        Some(id) => app
                            .lib
                            .boards()
                            .get(&id)
                            .map(crate::boardedit::BoardEditor::edit),
                    };
                }
            }
            Dialog::Expander(d) => expander_body(app, ui, d),
        }
    });
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Outcome::Cancel;
    }
    if !matches!(outcome, Outcome::Open) {
        ctx.data_mut(|d| d.insert_temp(full_id, false));
    }
    let keep = match outcome {
        Outcome::Open => true,
        Outcome::Cancel => false,
        Outcome::Apply => match apply(app, &dialog) {
            Ok(()) => false,
            Err(e) => {
                app.info(LogLevel::Error, e);
                true
            }
        },
        Outcome::Delete => {
            let lib = app.lib.clone();
            match &dialog {
                Dialog::Var(VarDialog {
                    original: Some(name),
                    ..
                }) => {
                    if let Some(doc) = &mut app.doc {
                        if let Err(e) = doc.edit(&lib, None, |p| p.remove_var(name)) {
                            app.info(LogLevel::Error, e.to_string());
                        }
                    }
                    app.selection = None;
                    false
                }
                Dialog::Expander(ExpanderDialog {
                    original: Some(id), ..
                }) => {
                    // Refused while blocks stand on the chip: the window stays open.
                    let result = match &mut app.doc {
                        Some(doc) => doc.edit(&lib, None, |p| p.remove_expander(id)),
                        None => Ok(()),
                    };
                    match result {
                        Ok(()) => false,
                        Err(e) => {
                            app.info(LogLevel::Error, e.to_string());
                            true
                        }
                    }
                }
                _ => false,
            }
        }
    };
    if keep {
        app.dialog = Some(dialog);
    }
}

/// ✓ / ✗ row at the top of the window.
fn buttons(ui: &mut Ui, deletable: bool) -> Outcome {
    let mut outcome = Outcome::Open;
    ui.horizontal(|ui| {
        let ok = egui::Button::new(
            RichText::new(crate::i18n::tr("✔ Применить")).color(Color32::from_rgb(72, 136, 88)),
        );
        if ui.add(ok).clicked() {
            outcome = Outcome::Apply;
        }
        let cancel = egui::Button::new(
            RichText::new(crate::i18n::tr("✕ Отмена")).color(Color32::from_rgb(186, 74, 66)),
        );
        if ui.add(cancel).clicked() {
            outcome = Outcome::Cancel;
        }
        if deletable && ui.button(crate::i18n::tr("Удалить")).clicked() {
            outcome = Outcome::Delete;
        }
    });
    outcome
}

fn apply(app: &mut EditorApp, dialog: &Dialog) -> Result<(), String> {
    let lib = app.lib.clone();
    let doc = app
        .doc
        .as_mut()
        .ok_or(crate::i18n::tr("проект не открыт"))?;
    match dialog {
        Dialog::Board(d) => {
            let board = d.config()?;
            let uarts = d.uarts();
            if doc.project.config.board != board || doc.project.config.uart != uarts {
                app.port = board.port.clone().unwrap_or_default();
                doc.project.config.board = board;
                doc.project.config.uart = uarts;
                doc.dirty = true;
                doc.revision += 1;
                // Pins and UARTs are checked against the board.
                doc.recheck(&lib);
            }
        }
        Dialog::Block(d) => {
            let new_id = d.new_id.trim().to_owned();
            let block = doc
                .project
                .logic
                .block(&d.id)
                .ok_or(crate::i18n::tr("блок удалён"))?;
            let def = lib
                .resolve(&doc.project.logic, block)
                .ok_or(crate::i18n::tr("неизвестный тип блока"))?
                .def
                .clone();
            let is_local = |name: &str| def.param(name).is_some_and(|p| p.local.is_some());
            // The pin parameter on an expander is a pin of the chip: set with the source.
            let chip_pin = (def.info.expander && d.source != PinSource::Gpio)
                .then(|| pin_param(&def).map(|q| q.name.clone()))
                .flatten();
            let before = block.param_in.clone();
            doc.edit(&lib, None, |p| {
                for name in d.param_in.symmetric_difference(&before) {
                    p.set_param_input(&lib, &d.id, name, d.param_in.contains(name))?;
                }
                // Back to a GPIO: drops the chip pin number the parameter still holds.
                if d.source == PinSource::Gpio
                    && p.logic.block(&d.id).is_some_and(|b| b.expander.is_some())
                {
                    p.set_block_expander(&lib, &d.id, None)?;
                }
                for (name, value) in &d.values {
                    if is_local(name) || chip_pin.as_ref() == Some(name) {
                        continue;
                    }
                    let now = p.logic.block(&d.id).and_then(|b| b.params.get(name));
                    if now != value.as_ref() {
                        p.set_param(&lib, &d.id, name, value.clone())?;
                    }
                }
                if let Some(name) = &chip_pin {
                    let Some(Some(ParamValue::Int(pin))) = d.values.get(name) else {
                        return Err(crab_core::EditError::BadExpander(
                            crate::i18n::tr("выберите вывод расширителя").to_owned(),
                        ));
                    };
                    let target = match &d.source {
                        PinSource::Expander(id) => id.clone(),
                        _ => {
                            p.add_expander(d.pending.clone())?;
                            d.pending.id.clone()
                        }
                    };
                    p.set_block_expander(&lib, &d.id, Some((&target, *pin)))?;
                }
                p.rename_block(&d.id, &new_id)
            })
            .map_err(|e| e.to_string())?;
            // Passwords stay in the program, under the SSID set above.
            let mut block = doc.project.logic.block(&new_id).cloned();
            if let Some(block) = &mut block {
                for q in def.params.iter().filter(|q| q.local.is_some()) {
                    let value = d.values.get(&q.name).cloned().flatten();
                    let value = value.or_else(|| q.default.clone());
                    if let Some(value) = value {
                        block.params.insert(q.name.clone(), value);
                    }
                }
                app.local.take_block(&def, block);
                app.save_local();
            }
            app.select(Some(Selection::Block(new_id)));
        }
        Dialog::Var(d) => {
            let var = Var {
                name: d.name.trim().to_owned(),
                ty: d.ty,
                init: parse_init(d.ty, &d.init)?,
                description: d.description.trim().to_owned(),
                retain: d.retain,
                service: false,
            };
            match &d.original {
                Some(name) => doc.edit(&lib, None, |p| p.update_var(name, var)),
                None => doc.edit(&lib, None, |p| p.add_var(var)),
            }
            .map_err(|e| e.to_string())?;
            if let Some((id, en)) = &d.block {
                doc.edit(&lib, None, |p| p.set_var_write_enable(&lib, id, *en))
                    .map_err(|e| e.to_string())?;
            }
        }
        Dialog::Expander(d) => {
            let mut e = d.form.clone();
            e.id = e.id.trim().to_owned();
            match &d.original {
                Some(id) => doc.edit(&lib, None, |p| p.update_expander(id, e)),
                None => doc.edit(&lib, None, |p| p.add_expander(e)),
            }
            .map_err(|e| e.to_string())?;
        }
        Dialog::Sheet(d) => {
            let info = SheetInfo {
                number: d.number,
                name: d.name.trim().to_owned(),
                comment: d.comment.trim().to_owned(),
                background: d.background,
                disabled: d.disabled,
                period: if d.period == crab_core::layout::DEFAULT_BACKGROUND_PERIOD_MS {
                    0
                } else {
                    d.period
                },
            };
            doc.edit(&lib, None, |p| {
                p.set_sheet_info(info);
                Ok::<_, String>(())
            })?;
        }
    }
    Ok(())
}

pub fn type_name(ty: SignalType) -> &'static str {
    ty.name()
}

pub fn value_text(v: &ParamValue) -> String {
    match v {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => format!("{f}"),
        ParamValue::Text(s) => s.clone(),
    }
}
