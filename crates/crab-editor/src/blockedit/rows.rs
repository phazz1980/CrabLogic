use super::*;

/// Move, delete buttons of a table row; returns the action chosen.
pub(super) enum RowOp {
    Up(usize),
    Down(usize),
    Remove(usize),
}

pub(super) fn row_ops(ui: &mut Ui, i: usize, len: usize, op: &mut Option<RowOp>) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(i > 0, egui::Button::new("↑").small())
            .clicked()
        {
            *op = Some(RowOp::Up(i));
        }
        if ui
            .add_enabled(i + 1 < len, egui::Button::new("↓").small())
            .clicked()
        {
            *op = Some(RowOp::Down(i));
        }
        if ui
            .small_button("✕")
            .on_hover_text(crate::i18n::tr("Удалить"))
            .clicked()
        {
            *op = Some(RowOp::Remove(i));
        }
    });
}

pub(super) fn apply_op<T>(rows: &mut Vec<T>, op: Option<RowOp>) {
    match op {
        Some(RowOp::Up(i)) => rows.swap(i, i - 1),
        Some(RowOp::Down(i)) => rows.swap(i, i + 1),
        Some(RowOp::Remove(i)) => {
            rows.remove(i);
        }
        None => {}
    }
}

pub(super) fn type_combo(ui: &mut Ui, id: (&str, usize), ty: &mut SignalType) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(type_name(*ty))
        .width(70.0)
        .show_ui(ui, |ui| {
            // `any` fits inputs only; the block check says so for an output.
            for t in SignalType::ALL.into_iter().chain([
                SignalType::Any,
                SignalType::Number,
                SignalType::Same,
            ]) {
                ui.selectable_value(ty, t, type_name(t));
            }
        });
}

/// Choice of the value of an unconnected `bool` input: `true`, `false` or required (empty).
pub(super) fn bool_default_combo(ui: &mut Ui, id: (&str, usize), default: &mut String) {
    // Text left from another type or typed by hand shows as the nearest choice.
    *default = match parse_bool(default.trim()) {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    }
    .to_owned();
    let shown = if default.is_empty() {
        crate::i18n::tr("обязателен")
    } else {
        default.as_str()
    };
    egui::ComboBox::from_id_salt(("bool_default", id))
        .selected_text(shown)
        .width(90.0)
        .show_ui(ui, |ui| {
            for (value, text) in [
                ("", crate::i18n::tr("обязателен")),
                ("true", "true"),
                ("false", "false"),
            ] {
                ui.selectable_value(default, value.to_owned(), text);
            }
        })
        .response
        .on_hover_text(crate::i18n::tr(
            "Значение неподключённого входа; «обязателен» — вход надо подключить",
        ));
}

/// A name not used by any port or parameter of the form.
pub(super) fn free_name(f: &Form, base: &str) -> String {
    let taken = |n: &str| {
        f.inputs.iter().chain(&f.outputs).any(|p| p.name == n)
            || f.params.iter().any(|p| p.name == n)
    };
    (1..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken(n))
        .expect("some name is free")
}

pub(super) fn ports_table(ui: &mut Ui, id: &str, rows: &mut Vec<PortRow>, inputs: bool) {
    if rows.is_empty() {
        ui.weak(if inputs {
            crate::i18n::tr("Входов нет")
        } else {
            crate::i18n::tr("Выходов нет")
        });
        return;
    }
    let mut op = None;
    let len = rows.len();
    egui::Grid::new(id)
        .num_columns(if inputs { 6 } else { 7 })
        .striped(true)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new(crate::i18n::tr("Имя")).strong());
            ui.label(RichText::new(crate::i18n::tr("Тип")).strong());
            ui.label(
                RichText::new(if inputs {
                    crate::i18n::tr("Если не подключён")
                } else {
                    crate::i18n::tr("В симуляции")
                })
                .strong(),
            );
            ui.label(RichText::new(crate::i18n::tr("Условие when")).strong());
            ui.label(RichText::new(crate::i18n::tr("Описание")).strong());
            if !inputs {
                ui.label(RichText::new(crate::i18n::tr("Часто")).strong())
                    .on_hover_text(crate::i18n::tr(
                        "Выход меняется часто (счётчик, измерение): проверка предупредит, если им кормят переменную с сохранением во флеш",
                    ));
            }
            ui.label("");
            ui.end_row();
            for (i, row) in rows.iter_mut().enumerate() {
                ui.add_sized([90.0, 20.0], TextEdit::singleline(&mut row.name));
                type_combo(ui, (id, i), &mut row.ty);
                if inputs && row.ty == SignalType::Bool {
                    bool_default_combo(ui, (id, i), &mut row.default);
                } else if inputs {
                    ui.add_sized(
                        [90.0, 20.0],
                        TextEdit::singleline(&mut row.default)
                            .hint_text(crate::i18n::tr("обязателен")),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Значение неподключённого входа; пусто — вход надо подключить",
                    ));
                } else {
                    ui.add_sized(
                        [90.0, 20.0],
                        TextEdit::singleline(&mut row.sim).hint_text(crate::i18n::tr("нет")),
                    )
                    .on_hover_text(crate::i18n::tr(
                        "Значение выхода в симуляции на компьютере, если код блока работает                          только на плате (драйвер железа); пусто — 0",
                    ));
                }
                ui.add_sized(
                    [130.0, 20.0],
                    TextEdit::singleline(&mut row.when).hint_text(crate::i18n::tr("всегда")),
                )
                .on_hover_text(crate::i18n::tr(
                    "Выражение по параметрам, например mode == \"input\": \
                     порт есть, только пока оно выполняется",
                ));
                ui.add_sized([220.0, 20.0], TextEdit::singleline(&mut row.description));
                if !inputs {
                    ui.checkbox(&mut row.fast, "");
                }
                row_ops(ui, i, len, &mut op);
                ui.end_row();
            }
        });
    apply_op(rows, op);
}

pub(super) fn ports_tab(ui: &mut Ui, f: &mut Form) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::i18n::tr("Входы")).strong());
        if ui
            .small_button(crate::i18n::tr("+ Добавить вход"))
            .clicked()
        {
            let name = free_name(f, "in");
            f.inputs.push(PortRow::new(&name));
        }
    });
    ports_table(ui, "block_inputs", &mut f.inputs, true);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(crate::i18n::tr("Выходы")).strong());
        if ui
            .small_button(crate::i18n::tr("+ Добавить выход"))
            .clicked()
        {
            let name = free_name(f, "out");
            f.outputs.push(PortRow::new(&name));
        }
    });
    ports_table(ui, "block_outputs", &mut f.outputs, false);
    ui.add_space(8.0);
    ui.weak(crate::i18n::tr(
        "Имена — буквы (можно русские), цифры и «_», без пробелов: по ним блок связан в схеме \
         и в коде; Входы, выходы и параметры не могут называться \
         одинаково, имена id и type заняты. Неявно приводится только int → float.",
    ));
}

pub(super) fn params_tab(ui: &mut Ui, f: &mut Form) {
    if ui
        .small_button(crate::i18n::tr("+ Добавить параметр"))
        .clicked()
    {
        let name = free_name(f, "p");
        f.params.push(ParamRow::new(&name));
    }
    if f.params.is_empty() {
        ui.weak(crate::i18n::tr("Параметров нет"));
        return;
    }
    let mut op = None;
    let len = f.params.len();
    egui::Grid::new("block_params")
        .num_columns(9)
        .striped(true)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for h in [
                crate::i18n::tr("Имя"),
                crate::i18n::tr("Вид"),
                crate::i18n::tr("По умолчанию"),
                crate::i18n::tr("Мин."),
                crate::i18n::tr("Макс."),
                crate::i18n::tr("Условие when"),
                crate::i18n::tr("Под блоком"),
                crate::i18n::tr("Описание"),
            ] {
                ui.label(RichText::new(h).strong());
            }
            ui.label("");
            ui.end_row();
            for (i, row) in f.params.iter_mut().enumerate() {
                ui.add_sized([90.0, 20.0], TextEdit::singleline(&mut row.name));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("param_kind", i))
                        .selected_text(kind_label(row.kind))
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            for k in PARAM_KINDS {
                                ui.selectable_value(&mut row.kind, k, kind_label(k));
                            }
                        });
                    if row.kind == ParamKind::Pin {
                        pin_mode_edit(ui, i, row);
                    } else if matches!(
                        row.kind,
                        ParamKind::Int | ParamKind::Float | ParamKind::Duration
                    ) {
                        ui.checkbox(&mut row.static_only, crate::i18n::tr("статический"))
                            .on_hover_text(crate::i18n::tr(
                                "Значение нужно при генерации кода (размер массива, начальное значение): параметр нельзя сделать входом",
                            ));
                    }
                });
                let hint = match row.kind {
                    ParamKind::Duration => crate::i18n::tr("обязателен (1s, 500ms)"),
                    _ => crate::i18n::tr("обязателен"),
                };
                ui.add_sized(
                    [110.0, 20.0],
                    TextEdit::singleline(&mut row.default).hint_text(hint),
                )
                .on_hover_text(crate::i18n::tr(
                    "Пусто — параметр надо задать в свойствах блока",
                ));
                let numeric = matches!(
                    row.kind,
                    ParamKind::Int | ParamKind::Float | ParamKind::Pin | ParamKind::Duration
                );
                for bound in [&mut row.min, &mut row.max] {
                    ui.add_enabled_ui(numeric, |ui| {
                        ui.add_sized([50.0, 20.0], TextEdit::singleline(bound))
                    })
                    .inner
                    .on_hover_text(crate::i18n::tr("У duration — в миллисекундах"));
                }
                ui.add_sized(
                    [130.0, 20.0],
                    TextEdit::singleline(&mut row.when).hint_text(crate::i18n::tr("всегда")),
                );
                egui::ComboBox::from_id_salt(("param_show", i))
                    .selected_text(show_label(row.show))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for s in ParamShow::ALL {
                            ui.selectable_value(&mut row.show, s, show_label(s));
                        }
                    })
                    .response
                    .on_hover_text(crate::i18n::tr(
                        "Как значение подписывается под блоком на плате",
                    ));
                ui.add_sized([200.0, 20.0], TextEdit::singleline(&mut row.description));
                row_ops(ui, i, len, &mut op);
                ui.end_row();
            }
        });
    apply_op(&mut f.params, op);
}

/// Direction of a pin parameter and whether blocks of the type may share the pin.
pub(super) fn pin_mode_edit(ui: &mut Ui, i: usize, row: &mut ParamRow) {
    let label = |m: PinMode| match m {
        PinMode::Io => crate::i18n::tr("вход-выход"),
        PinMode::In => crate::i18n::tr("вход"),
        PinMode::Out => crate::i18n::tr("выход"),
        PinMode::Pwm => crate::i18n::tr("ШИМ"),
        PinMode::Adc => crate::i18n::tr("АЦП"),
        PinMode::Dac => crate::i18n::tr("ЦАП"),
    };
    egui::ComboBox::from_id_salt(("pin_mode", i))
        .selected_text(label(row.mode))
        .width(90.0)
        .show_ui(ui, |ui| {
            for m in [
                PinMode::Io,
                PinMode::In,
                PinMode::Out,
                PinMode::Pwm,
                PinMode::Adc,
                PinMode::Dac,
            ] {
                ui.selectable_value(&mut row.mode, m, label(m));
            }
        })
        .response
        .on_hover_text(crate::i18n::tr("Какие выводы платы подходят: выход не ставится на вывод «только вход», ШИМ, АЦП и ЦАП — только на выводы с этой функцией"));
    ui.checkbox(&mut row.shared, crate::i18n::tr("общий"))
        .on_hover_text(crate::i18n::tr(
            "Несколько блоков этого типа могут сидеть на одном выводе (шина 1-Wire)",
        ));
}
