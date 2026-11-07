use super::*;

/// Asks before deleting a sheet (`app.sheet_delete`) and deletes it with its blocks.
pub fn sheet_delete_dialog(app: &mut EditorApp, ctx: &egui::Context) {
    let Some(sheet) = app.sheet_delete else {
        return;
    };
    let Some(doc) = &app.doc else {
        app.sheet_delete = None;
        return;
    };
    let name = doc
        .project
        .layout
        .sheet(sheet)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let blocks = doc
        .project
        .layout
        .blocks
        .values()
        .filter(|p| p.sheet == sheet)
        .count();
    let title = if name.is_empty() {
        crate::i18n::fmt("Плата {0}", &[&sheet])
    } else {
        crate::i18n::fmt("Плата {0} «{1}»", &[&sheet, &name])
    };
    let mut answer = None;
    egui::Modal::new(Id::unique("sheet_delete")).show(ctx, |ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Удалить плату")).strong());
        ui.label(crate::i18n::fmt(
            "Удалить «{0}» вместе с блоками на ней ({1})? Это можно отменить (Ctrl+Z).",
            &[&title, &blocks],
        ));
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
    let Some(delete) = answer else {
        return;
    };
    app.sheet_delete = None;
    if !delete {
        return;
    }
    let lib = app.lib.clone();
    let result = app.doc.as_mut().map(|doc| {
        doc.edit(&lib, None, |p| {
            p.delete_sheet(sheet)?;
            p.release_unused_services(&lib)
        })
    });
    match result {
        Some(Err(e)) => app.info(LogLevel::Error, e.to_string()),
        _ => {
            app.extra_sheets.remove(&sheet);
            app.canvas.collapsed.remove(&sheet);
            app.selection = None;
            app.sheet = app.sheets().first().copied().unwrap_or(1);
        }
    }
}

/// Cut / Copy / Paste entries of the board context menus (they act on the selection).
pub(super) fn clipboard_items(app: &mut EditorApp, ui: &mut Ui) {
    let has_blocks = app
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
        app.cut_selection(ui.ctx());
        ui.close();
    }
    if ui
        .add_enabled(
            has_blocks,
            egui::Button::new(crate::i18n::tr("Копировать")).shortcut_text("Ctrl+C"),
        )
        .clicked()
    {
        app.copy_selection(ui.ctx());
        ui.close();
    }
    if ui
        .add(egui::Button::new(crate::i18n::tr("Вставить")).shortcut_text("Ctrl+V"))
        .clicked()
    {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        ui.close();
    }
}

pub(super) fn inverted_output(logic: &Logic, port: &PortRef) -> bool {
    logic
        .block(&port.block)
        .is_some_and(|b| b.inverted.contains(&port.port))
}

/// Live value as a consumer sees it: logical values flip when inverted.
pub(super) fn flip(v: crab_debug::Value, invert: bool) -> crab_debug::Value {
    match v {
        crab_debug::Value::Bool(b) if invert => crab_debug::Value::Bool(!b),
        v => v,
    }
}

/// Constant typed in the menu of an input of type `ty`; `None` if it does not fit.
pub(super) fn parse_const(ty: SignalType, text: &str) -> Option<ParamValue> {
    let t = text.trim();
    match ty {
        SignalType::Bool => match t {
            "1" | "true" => Some(ParamValue::Bool(true)),
            "0" | "false" => Some(ParamValue::Bool(false)),
            _ => None,
        },
        SignalType::Int => t.parse().ok().map(ParamValue::Int),
        SignalType::Float => t.replace(',', ".").parse().ok().map(ParamValue::Float),
        SignalType::String => Some(ParamValue::Text(text.to_owned())),
        SignalType::Any | SignalType::Same => Some(SignalType::guess_value(text)),
        SignalType::Number => {
            Some(SignalType::guess_value(text)).filter(|v| SignalType::of_value(v).is_numeric())
        }
    }
}

/// Right click on a port: a constant on an input, inversion of a logical port
/// and, online, forcing an output or putting it on the chart.
pub(super) fn port_context_menu(
    app: &mut EditorApp,
    port: &PortRef,
    output: bool,
    ty: SignalType,
    resp: &egui::Response,
) {
    let Some(block) = app
        .doc
        .as_ref()
        .and_then(|d| d.project.logic.block(&port.block))
    else {
        return;
    };
    let inverted = if output {
        &block.inverted
    } else {
        &block.inverted_inputs
    }
    .contains(&port.port);
    let constant = block.consts.get(&port.port).filter(|_| !output).cloned();
    if resp.secondary_clicked() {
        app.canvas.const_text = constant.as_ref().map(value_text).unwrap_or_default();
    }
    // An input has no storage of its own: it is forced through its source, which
    // changes the value for every consumer of that source.
    let debug_index = app.debug.as_ref().and_then(|d| {
        if output {
            return d.table.output_index(port);
        }
        let logic = &app.doc.as_ref()?.project.logic;
        let link = logic.links.iter().find(|l| &l.to == port)?;
        match logic.block(&link.from.block).and_then(|b| b.var_name()) {
            Some(var) => d.table.var_index(var),
            None => d.table.output_index(&link.from),
        }
    });
    let mut debug = app.debug.take();
    let mut sim = app.sim.take();
    let mut set_const = None;
    let mut invert = None;
    let mut bad_const = false;
    let text = &mut app.canvas.const_text;
    resp.context_menu(|ui| {
        if let (Some(s), true) = (&mut sim, output) {
            crate::simview::port_menu(ui, s, port);
            ui.separator();
        }
        if let (Some(d), Some(index)) = (&mut debug, debug_index) {
            debugview::value_menu(ui, d, index);
            if !output {
                ui.weak(crate::i18n::tr(
                    "Форсируется источник входа (у всех потребителей)",
                ));
            }
            ui.separator();
        }
        if !output {
            let title = if constant.is_some() {
                crate::i18n::tr("Изменить константу")
            } else {
                crate::i18n::tr("Вставить константу")
            };
            // A click in the input field must not close the submenu.
            let config =
                MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            SubMenuButton::new(title).config(config).ui(ui, |ui| {
                if ty == SignalType::Bool {
                    for b in [true, false] {
                        if ui.button(b.to_string()).clicked() {
                            set_const = Some(Some(ParamValue::Bool(b)));
                            ui.close();
                        }
                    }
                    return;
                }
                // The menu style drops the vertical padding: give the field its own, and
                // centre the text, or it sticks to the top of a taller field.
                let edit = ui.add(
                    egui::TextEdit::singleline(text)
                        .margin(egui::Margin::symmetric(4, 3))
                        .align(egui::Align2::LEFT_CENTER),
                );
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(crate::i18n::tr("Применить")).clicked() || enter {
                    match parse_const(ty, text) {
                        Some(v) => {
                            set_const = Some(Some(v));
                            ui.close();
                        }
                        None => bad_const = true,
                    }
                }
            });
            if constant.is_some() && ui.button(crate::i18n::tr("Удалить константу")).clicked()
            {
                set_const = Some(None);
            }
        }
        if ty == SignalType::Bool {
            let mut on = inverted;
            if ui
                .checkbox(&mut on, crate::i18n::tr("Инвертировать"))
                .clicked()
            {
                invert = Some(on);
                ui.close();
            }
        }
    });
    app.debug = debug;
    app.sim = sim;
    if bad_const {
        let text = app.canvas.const_text.clone();
        app.info(
            LogLevel::Error,
            crate::i18n::msg(
                "«{0}» не подходит ко входу {1}: нужно число",
                &[&text, &port],
            ),
        );
    }
    let lib = app.lib.clone();
    let Some(doc) = &mut app.doc else { return };
    let result = match (set_const, invert) {
        (Some(value), _) => doc.edit(&lib, None, |p| p.set_input_const(&lib, port, value)),
        (_, Some(on)) => doc.edit(&lib, None, |p| p.set_port_inverted(&lib, port, output, on)),
        _ => Ok(()),
    };
    if let Err(e) = result {
        app.info(LogLevel::Error, e.to_string());
    }
}

pub(super) fn block_context_menu(app: &mut EditorApp, id: &str, resp: &egui::Response) {
    let sheets = app.sheets();
    let current = app
        .doc
        .as_ref()
        .and_then(|d| d.project.layout.blocks.get(id))
        .map_or(1, |p| p.sheet);
    let var = app
        .doc
        .as_ref()
        .and_then(|d| d.project.logic.block(id))
        .and_then(|b| b.var_name().map(str::to_owned));
    let mut move_to = None;
    let mut delete = false;
    let mut props = false;
    let mut edit_var = false;
    let mut edit_type = false;
    let mut replace_with: Option<String> = None;
    let library_kind = app
        .doc
        .as_ref()
        .and_then(|d| d.project.logic.block(id))
        .filter(|b| b.var_name().is_none())
        .and_then(|b| app.lib.get(&b.kind))
        .cloned();
    let debug_index = var
        .as_deref()
        .and_then(|v| app.debug.as_ref().and_then(|d| d.table.var_index(v)));
    // The menu of a selected block acts on the whole selection.
    let group: Vec<String> = match &app.selection {
        Some(s) if s.has_block(id) => s.blocks().to_vec(),
        _ => vec![id.to_owned()],
    };
    let mut debug = app.debug.take();
    resp.context_menu(|ui| {
        if let (Some(d), Some(index)) = (&mut debug, debug_index) {
            ui.menu_button(crate::i18n::tr("Отладка"), |ui| {
                debugview::value_menu(ui, d, index)
            });
            ui.separator();
        }
        clipboard_items(app, ui);
        ui.separator();
        if ui.button(crate::i18n::tr("Свойства...")).clicked() {
            props = true;
        }
        if var.is_some()
            && ui
                .button(crate::i18n::tr("Изменить переменную..."))
                .clicked()
        {
            edit_var = true;
        }
        if let Some(block) = &library_kind {
            let text = if app.lib.is_user(&block.def.info.kind) {
                crate::i18n::tr("Редактировать блок...")
            } else {
                crate::i18n::tr("Посмотреть тип блока...")
            };
            if ui.button(text).clicked() {
                edit_type = true;
            }
        }
        if let Some(block) = &library_kind {
            let current = block.def.info.kind.clone();
            ui.menu_button(crate::i18n::tr("Заменить блок на"), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        let mut blocks: Vec<_> = app
                            .lib
                            .iter()
                            .map(|b| &b.def)
                            .filter(|d| !d.info.hidden && d.info.kind != current)
                            .filter(|d| d.info.kind != VAR_GET && d.info.kind != VAR_SET)
                            .collect();
                        blocks.sort_by(|a, b| {
                            (&a.info.group, &a.info.name).cmp(&(&b.info.group, &b.info.name))
                        });
                        let mut group = None;
                        for d in blocks {
                            if group != Some(&d.info.group) {
                                group = Some(&d.info.group);
                                ui.weak(crate::i18n::tr(&d.info.group));
                            }
                            if ui.button(crate::i18n::tr(&d.info.name)).clicked() {
                                replace_with = Some(d.info.kind.clone());
                                ui.close();
                            }
                        }
                    });
            });
        }
        ui.menu_button(crate::i18n::tr("Перенести на плату"), |ui| {
            let new = sheets.last().copied().unwrap_or(1) + 1;
            for s in sheets.iter().copied().chain([new]) {
                if s != current && ui.button(crate::i18n::fmt("Плата {0}", &[&s])).clicked() {
                    move_to = Some(s);
                }
            }
        });
        ui.separator();
        if ui.button(crate::i18n::tr("Удалить")).clicked() {
            delete = true;
        }
    });
    app.debug = debug;
    if props {
        app.dialog = Dialog::for_block(app, id);
    }
    if edit_var {
        if let Some(name) = &var {
            app.dialog = Dialog::for_var(app, Some(name));
        }
    }
    if let (true, Some(block)) = (edit_type, &library_kind) {
        app.block_editor = Some(crate::blockedit::BlockEditor::edit(app, block));
    }
    if let Some(kind) = replace_with {
        let lib = app.lib.clone();
        let result = app
            .doc
            .as_mut()
            .map(|doc| doc.edit(&lib, None, |p| p.replace_block(&lib, id, &kind)));
        match result {
            Some(Ok(dropped)) if !dropped.is_empty() => app.info(
                LogLevel::Warning,
                crate::i18n::fmt("Блок заменён; не перенесено: {0}", &[&dropped.join(", ")]),
            ),
            Some(Err(e)) => app.info(LogLevel::Error, e.to_string()),
            _ => {}
        }
    }
    if let Some(sheet) = move_to {
        let lib = app.lib.clone();
        if let Some(doc) = &mut app.doc {
            let _ = doc.edit(&lib, None, |p| {
                for b in &group {
                    if let Some(&pos) = p.layout.blocks.get(b) {
                        p.move_block(b, BlockPos { sheet, ..pos })?;
                    }
                }
                Ok::<_, crab_core::EditError>(())
            });
        }
        app.extra_sheets.insert(sheet);
    }
    if delete {
        if group.len() == 1 {
            app.select(Some(Selection::Block(id.to_owned())));
        }
        app.delete_selection();
    }
}
