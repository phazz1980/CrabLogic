use super::*;

/// Web widgets (everything `web_*` but the service block) have a place on the page set in
/// the web designer, so their position and size are not edited here.
pub(super) fn is_widget_kind(kind: &str) -> bool {
    crab_core::project::is_widget_kind(kind)
}

pub(super) fn block_body(app: &EditorApp, ui: &mut Ui, d: &mut BlockDialog) {
    let Some(doc) = &app.doc else { return };
    let Some(block) = doc.project.logic.block(&d.id) else {
        ui.weak(crate::i18n::tr("Блок удалён"));
        return;
    };
    let Some(lib_block) = app.lib.resolve(&doc.project.logic, block) else {
        ui.colored_label(
            Color32::from_rgb(186, 74, 66),
            crate::i18n::fmt("Неизвестный тип «{0}»", &[&block.kind]),
        );
        return;
    };
    let docs = lib_block
        .docs_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .filter(|text| !text.trim().is_empty());
    let Some(docs) = docs else {
        block_settings(app, ui, d, doc, lib_block);
        return;
    };
    // Settings on the left, the block's docs on the right.
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(SETTINGS_WIDTH);
            block_settings(app, ui, d, doc, lib_block);
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_min_width(DOCS_MIN_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt("block_docs")
                .max_height(ui.ctx().content_rect().height() * 0.7)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let images = crate::markdown::dir_images(&lib_block.dir);
                    crate::markdown::show_with_images(ui, &docs, None, Some(&images));
                });
        });
    });
}

/// Width of the settings column when the docs are shown beside it.
pub(super) const SETTINGS_WIDTH: f32 = 360.0;
pub(super) const DOCS_MIN_WIDTH: f32 = 320.0;

pub(super) fn block_settings(
    app: &EditorApp,
    ui: &mut Ui,
    d: &mut BlockDialog,
    doc: &crate::doc::Doc,
    lib_block: &crab_core::LibraryBlock,
) {
    let def = &lib_block.def;
    ui.label(RichText::new(crate::i18n::tr(&def.info.name)).strong());
    ui.add(egui::Label::new(RichText::new(&def.info.description).weak()).wrap());
    ui.add_space(6.0);
    // Parameters and ports switched off by `when` follow the values being edited.
    let current: BTreeMap<String, ParamValue> = d
        .values
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.clone()?)))
        .collect();
    let params: Vec<&ParamDef> = def
        .active_params(&current)
        .filter(|p| {
            !p.fixed
                && !(is_widget_kind(&d.kind)
                    && (matches!(
                        p.name.as_str(),
                        "col" | "row" | "w" | "h" | "page" | "color" | "back"
                    ) || (d.kind == "web_text_input"
                        && matches!(p.name.as_str(), "initial" | "secret"))))
                && !(d.kind == "web_ui" && matches!(p.name.as_str(), "pages" | "enabled"))
        })
        .collect();
    let inputs: Vec<&PortDef> = def.active_inputs(&current).collect();
    let outputs: Vec<&PortDef> = def.active_outputs(&current).collect();
    let can_expander = def.info.expander && pin_param(def).is_some();
    let pins = Pins {
        layout: BoardLayout::of_board(&doc.project.config.board, app.lib.boards()),
        others: pin_uses(&doc.project.logic, &app.lib)
            .into_iter()
            .filter(|u| u.block != d.id)
            .collect(),
        expanders: if can_expander {
            doc.project
                .logic
                .expanders
                .iter()
                .map(|e| ExpanderPins {
                    expander: e.clone(),
                    taken: taken_pins(&doc.project.logic, &app.lib, &e.id, Some(&d.id)),
                })
                .collect()
        } else {
            Vec::new()
        },
        can_expander,
        free_id: free_expander_id(&doc.project.logic),
    };
    egui::Grid::new("block_props")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Надпись (id)"));
            ui.text_edit_singleline(&mut d.new_id);
            ui.label(RichText::new(crate::i18n::tr("Вход")).weak());
            ui.end_row();
            for &p in &params {
                let hint = match &p.label {
                    Some(_) => format!("{} ({})", p.description, p.name),
                    None => p.description.clone(),
                };
                ui.label(crate::i18n::tr(p.title()))
                    .on_hover_text(crate::i18n::tr(hint.trim()));
                let as_input = d.param_in.contains(&p.name);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!as_input, |ui| {
                        param_editor(ui, p, d, def, &app.local, &pins);
                    });
                });
                if p.name == "wifi_mode" {
                    // The mode comes from the «Mode» input when the last choice is set.
                    let mut on = matches!(d.values.get(&p.name), Some(Some(ParamValue::Int(3))));
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text(crate::i18n::tr(
                            "Режим приходит на вход «Режим»: 0 клиент, 1 точка доступа, 2 клиент и точка доступа",
                        ))
                        .changed()
                    {
                        d.values
                            .insert(p.name.clone(), Some(ParamValue::Int(if on { 3 } else { 0 })));
                    }
                } else if as_input
                    || (param_input_type(p).is_some() && def.input(&p.name).is_none())
                {
                    let mut on = as_input;
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text(crate::i18n::tr(
                            "Значение приходит на вход блока и читается на каждом скане; число в поле — значение входа, пока он не подключён",
                        ))
                        .changed()
                    {
                        if on {
                            d.param_in.insert(p.name.clone());
                        } else {
                            d.param_in.remove(&p.name);
                        }
                    }
                }
                else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    if matches!(
        d.kind.as_str(),
        "modbus_master_read" | "modbus_master_write"
    ) {
        external_tag_picker(ui, d, &doc.project.logic);
    }
    let from_board = pins.layout.is_some() && pending_board_index(d, &pins).is_some();
    if pins.can_expander && d.source == PinSource::New && !from_board {
        ui.add_space(6.0);
        new_expander_form(ui, &mut d.pending, &doc.project.logic);
    }
    if !inputs.is_empty() || !outputs.is_empty() {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(crate::i18n::tr("Входы и выходы"))
            .default_open(true)
            .show(ui, |ui| {
                for input in &inputs {
                    let source =
                        doc.project
                            .logic
                            .links
                            .iter()
                            .find(|l| l.to.block == d.id && l.to.port == input.name)
                            .map(|l| format!("← {}", l.from))
                            .or_else(|| {
                                input.default.as_ref().map(|v| {
                                    crate::i18n::fmt("не подключён, {0}", &[&value_text(v)])
                                })
                            })
                            .unwrap_or_else(|| "не подключён".to_owned());
                    ui.label(crate::i18n::fmt(
                        "вход {0} · {1}: {2}",
                        &[&input.name, &type_name(input.ty), &source],
                    ))
                    .on_hover_text(&input.description);
                }
                for output in &outputs {
                    ui.label(crate::i18n::fmt(
                        "выход {0} · {1}",
                        &[&output.name, &type_name(output.ty)],
                    ))
                    .on_hover_text(&output.description);
                }
            });
    }
}

/// Fills table, value type and address of a Modbus master read / write block from a tag
/// taken from another project.
pub(super) fn external_tag_picker(ui: &mut Ui, d: &mut BlockDialog, logic: &crab_core::Logic) {
    if logic.externals.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Внешний тег"));
        egui::ComboBox::from_id_salt("external_tag")
            .selected_text(crate::i18n::tr("выбрать..."))
            .show_ui(ui, |ui| {
                for e in &logic.externals {
                    if !ui.selectable_label(false, &e.name).clicked() {
                        continue;
                    }
                    let kind = match e.ty {
                        crab_core::VarType::Bool => "bool",
                        crab_core::VarType::Float | crab_core::VarType::Double => "float",
                        _ => "int",
                    };
                    for (name, text) in [("table", e.table.as_str()), ("kind", kind)] {
                        d.texts.insert(name.into(), text.into());
                        d.values
                            .insert(name.into(), Some(ParamValue::Text(text.into())));
                    }
                    d.values
                        .insert("reg".into(), Some(ParamValue::Int(i64::from(e.address))));
                }
            })
            .response
            .on_hover_text(crate::i18n::tr(
                "Подставляет таблицу, тип и адрес тега, перенесённого из другого проекта",
            ));
    });
}

/// An on / off switch: a rounded track with a knob that slides to the side of the state.
pub(super) fn toggle_switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(34.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let visuals = ui.style().interact_selectable(&response, *on);
    let track = if *on {
        Color32::from_rgb(70, 140, 90)
    } else {
        visuals.bg_fill
    };
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, track);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    ui.painter().circle_filled(
        egui::pos2(x, rect.center().y),
        radius - 2.5,
        Color32::from_gray(235),
    );
    response
}

/// Edits one buffered parameter of the block dialog.
pub(super) fn param_editor(
    ui: &mut Ui,
    p: &ParamDef,
    d: &mut BlockDialog,
    def: &BlockDef,
    local: &LocalValues,
    pins: &Pins,
) {
    let current = d
        .values
        .get(&p.name)
        .cloned()
        .flatten()
        .or_else(|| p.default.clone());
    if p.name == "page" && !d.pages.is_empty() {
        let page = match current {
            Some(ParamValue::Int(i)) if i >= 1 => i as usize,
            _ => 1,
        };
        let shown = d
            .pages
            .get(page - 1)
            .cloned()
            .unwrap_or_else(|| crate::i18n::fmt("Страница {0}", &[&page]));
        egui::ComboBox::from_id_salt("widget_page")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (i, name) in d.pages.iter().enumerate() {
                    if ui.selectable_label(i + 1 == page, name).clicked() {
                        d.values
                            .insert(p.name.clone(), Some(ParamValue::Int(i as i64 + 1)));
                    }
                }
            });
        return;
    }
    let expander_pin = pins.can_expander && pin_param(def).is_some_and(|q| q.name == p.name);
    if expander_pin && pins.layout.is_none() {
        // A board without a pin table: the source and the pin are chosen separately.
        pin_source_combo(ui, p, d, pins);
        if d.source != PinSource::Gpio {
            expander_pin_combo(ui, p, d, pins);
            return;
        }
    }
    if p.kind == ParamKind::Pin && pins.layout.is_some() {
        let current = match current {
            Some(ParamValue::Int(i)) => Some(i),
            _ => None,
        };
        pin_list_combo(ui, p, d, def, pins, current);
        return;
    }
    if !p.choices.is_empty() {
        let selected = current.as_ref().map_or_else(
            || crate::i18n::tr("не задан").to_owned(),
            |v| crate::i18n::tr(&p.value_label(v)).to_owned(),
        );
        egui::ComboBox::from_id_salt(("param_choice", &p.name))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for c in &p.choices {
                    let on = current.as_ref() == Some(&c.value);
                    if ui.selectable_label(on, crate::i18n::tr(&c.label)).clicked() {
                        if let ParamValue::Text(s) = &c.value {
                            d.texts.insert(p.name.clone(), s.clone());
                        }
                        d.values.insert(p.name.clone(), Some(c.value.clone()));
                    }
                }
            });
        return;
    }
    let mut set = |v: ParamValue| {
        d.values.insert(p.name.clone(), Some(v));
    };
    match p.kind {
        ParamKind::Bool => {
            let mut v = matches!(current, Some(ParamValue::Bool(true)));
            if toggle_switch(ui, &mut v).changed() {
                set(ParamValue::Bool(v));
            }
        }
        ParamKind::Int | ParamKind::Pin => {
            let missing = current.is_none();
            let mut v = match current {
                Some(ParamValue::Int(i)) => i,
                _ => 0,
            };
            let mut drag = egui::DragValue::new(&mut v);
            if p.kind == ParamKind::Pin {
                drag = drag.range(0..=48).prefix("GPIO");
            }
            if let (Some(min), Some(max)) = (p.min, p.max) {
                drag = drag.range(min as i64..=max as i64);
            }
            if ui.add(drag).changed()
                || (missing && ui.small_button(crate::i18n::tr("задать")).clicked())
            {
                set(ParamValue::Int(v));
            }
            if missing {
                ui.colored_label(Color32::from_rgb(186, 74, 66), crate::i18n::tr("не задан"));
            }
        }
        ParamKind::Float => {
            let mut v = match current {
                Some(ParamValue::Float(f)) => f,
                Some(ParamValue::Int(i)) => i as f64,
                _ => 0.0,
            };
            let mut drag = egui::DragValue::new(&mut v).speed(0.1);
            if let (Some(min), Some(max)) = (p.min, p.max) {
                drag = drag.range(min..=max);
            }
            if ui.add(drag).changed() {
                set(ParamValue::Float(v));
            }
        }
        ParamKind::Duration => {
            let text = d.texts.entry(p.name.clone()).or_default();
            if duration_editor(ui, &p.name, text) {
                let text = text.clone();
                d.values
                    .insert(p.name.clone(), Some(ParamValue::Text(text)));
            }
        }
        ParamKind::Text => {
            // Parameters kept by the program under this one's value (SSID -> password).
            let dependents: Vec<&ParamDef> = def
                .params
                .iter()
                .filter(|q| q.local.as_deref() == Some(p.name.as_str()))
                .collect();
            let secret = p.show == ParamShow::Secret && !d.revealed.contains(&p.name);
            let text = d.texts.entry(p.name.clone()).or_default();
            let valid = p.kind != ParamKind::Duration || parse_duration_ms(text).is_some();
            let mut edit = egui::TextEdit::singleline(text)
                .desired_width(110.0)
                .password(secret);
            if p.kind == ParamKind::Duration {
                edit = edit.hint_text("5s, 500ms, 2m");
            }
            if !valid {
                edit = edit.text_color(Color32::from_rgb(186, 74, 66));
            }
            let changed = if dependents.is_empty() {
                ui.add(edit).changed()
            } else {
                // Remembered keys (entered SSIDs) are suggested while typing.
                let known: BTreeSet<&str> = dependents
                    .iter()
                    .flat_map(|q| local.keys(&d.kind, &q.name))
                    .collect();
                let id = ui.make_persistent_id(("suggest", &p.name));
                let out = egui::CompletionPopup::new(id)
                    .word_boundary(|_| false)
                    .show(ui, edit, |query| {
                        let typed = query.text.to_lowercase();
                        known
                            .iter()
                            .filter(|k| **k != query.text && k.to_lowercase().contains(&typed))
                            .map(|k| egui::Suggestion::new(*k))
                            .collect()
                    });
                out.accepted.is_some() || out.text_edit.response.changed()
            };
            if p.show == ParamShow::Secret {
                let shown = d.revealed.contains(&p.name);
                if ui
                    .selectable_label(shown, "\u{1f441}")
                    .on_hover_text(crate::i18n::tr("Показать или скрыть"))
                    .clicked()
                {
                    if shown {
                        d.revealed.remove(&p.name);
                    } else {
                        d.revealed.insert(p.name.clone());
                    }
                }
            }
            if changed {
                let text = d.texts.get(&p.name).cloned().unwrap_or_default();
                let ok = p.kind != ParamKind::Duration || parse_duration_ms(&text).is_some();
                if ok {
                    // A new key brings its stored values (the password of that SSID).
                    for q in &dependents {
                        let value = local.get(&d.kind, &q.name, &text).unwrap_or_default();
                        d.texts.insert(q.name.clone(), value.to_owned());
                        d.values
                            .insert(q.name.clone(), Some(ParamValue::Text(value.to_owned())));
                    }
                    d.values
                        .insert(p.name.clone(), Some(ParamValue::Text(text)));
                }
            }
        }
    }
}

/// How a duration is typed.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum DurationUnit {
    Millis,
    Seconds,
    Minutes,
    /// Minutes, seconds and milliseconds in three fields.
    Parts,
}

/// Duration entry: the unit is chosen with a radio button, the value is typed in one
/// field or in three (min : s : ms). `text` keeps the short form (`2m`, `1500ms`);
/// returns true when it changed.
pub(super) fn duration_editor(ui: &mut Ui, salt: &str, text: &mut String) -> bool {
    let id = ui.make_persistent_id(("duration_unit", salt));
    let ms = parse_duration_ms(text).unwrap_or(0);
    let mut unit = ui
        .data(|d| d.get_temp::<DurationUnit>(id))
        .unwrap_or(match ms {
            m if m > 0 && m.is_multiple_of(60_000) => DurationUnit::Minutes,
            m if m.is_multiple_of(1_000) => DurationUnit::Seconds,
            _ => DurationUnit::Millis,
        });
    let mut new_ms = None;
    let max = u64::from(u32::MAX);
    ui.vertical(|ui| {
        let rows = [
            (DurationUnit::Millis, "Миллисекунды"),
            (DurationUnit::Seconds, "Секунды"),
            (DurationUnit::Minutes, "Минуты"),
            (DurationUnit::Parts, "Минуты : секунды : миллисекунды"),
        ];
        for (row, label) in rows {
            ui.horizontal(|ui| {
                if ui.radio(unit == row, crate::i18n::tr(label)).clicked() {
                    unit = row;
                }
                if unit != row {
                    return;
                }
                match row {
                    DurationUnit::Millis => {
                        let mut v = ms;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(0..=max))
                            .changed()
                        {
                            new_ms = Some(v);
                        }
                    }
                    DurationUnit::Seconds | DurationUnit::Minutes => {
                        let factor = if row == DurationUnit::Seconds {
                            1_000.0
                        } else {
                            60_000.0
                        };
                        let mut v = ms as f64 / factor;
                        let drag = egui::DragValue::new(&mut v)
                            .range(0.0..=max as f64 / factor)
                            .speed(if row == DurationUnit::Seconds {
                                0.1
                            } else {
                                0.01
                            })
                            .max_decimals(if row == DurationUnit::Seconds { 3 } else { 4 });
                        if ui.add(drag).changed() {
                            new_ms = Some((v * factor).round() as u64);
                        }
                    }
                    DurationUnit::Parts => {
                        let (mut m, mut s, mut f) = split_min_sec_ms(ms);
                        let mut changed = ui
                            .add(egui::DragValue::new(&mut m).range(0..=71_582))
                            .changed();
                        ui.label(":");
                        changed |= ui.add(egui::DragValue::new(&mut s).range(0..=59)).changed();
                        ui.label(":");
                        changed |= ui
                            .add(egui::DragValue::new(&mut f).range(0..=999))
                            .changed();
                        if changed {
                            new_ms = join_min_sec_ms(m, s, f).or(Some(max));
                        }
                    }
                }
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, unit));
    match new_ms {
        Some(v) if v != ms || parse_duration_ms(text).is_none() => {
            *text = format_duration_ms(v);
            true
        }
        _ => false,
    }
}
