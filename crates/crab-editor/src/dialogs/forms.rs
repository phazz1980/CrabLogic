use super::*;

pub(super) fn var_body(ui: &mut Ui, d: &mut VarDialog) {
    egui::Grid::new("var_props")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Имя"));
            ui.text_edit_singleline(&mut d.name);
            ui.end_row();
            ui.label(crate::i18n::tr("Тип"));
            egui::ComboBox::from_id_salt("var_type")
                .selected_text(d.ty.name())
                .height(400.0)
                .show_ui(ui, |ui| {
                    for ty in VarType::ALL {
                        ui.selectable_value(&mut d.ty, ty, ty.name());
                    }
                });
            ui.end_row();
            ui.label(crate::i18n::tr("Начальное значение"));
            let hint = match d.ty.signal() {
                SignalType::Bool => "false",
                SignalType::Int => "0",
                SignalType::Float => "0.0",
                SignalType::String | SignalType::Any | SignalType::Number | SignalType::Same => crate::i18n::tr("текст"),
            };
            let valid = parse_init(d.ty, &d.init).is_ok();
            let mut edit = egui::TextEdit::singleline(&mut d.init).hint_text(hint);
            if !valid {
                edit = edit.text_color(Color32::from_rgb(186, 74, 66));
            }
            ui.add(edit);
            ui.end_row();
            ui.label(crate::i18n::tr("Описание"));
            ui.text_edit_singleline(&mut d.description);
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut d.retain, crate::i18n::tr("Сохранять в EEPROM")).on_hover_text(
                crate::i18n::tr("Значение хранится во флеш-памяти (NVS) и восстанавливается после перезагрузки.                  Запись — только когда значение изменилось, не чаще раза в секунду."),
            );
            ui.end_row();
            if let Some((_, en)) = &mut d.block {
                ui.label("");
                ui.checkbox(en, crate::i18n::tr("Вход разрешения записи (En)")).on_hover_text(
                    crate::i18n::tr("Только для этого блока: у него появляется логический вход En,                      значение записывается в переменную, только пока En = true."),
                );
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    ui.weak(crate::i18n::tr("Переменную можно читать на любой плате сколько угодно раз, а записывать — в одном месте. Если её никто не записывает, она работает как уставка."));
}

pub(super) fn sheet_body(ui: &mut Ui, d: &mut SheetDialog) {
    egui::Grid::new("sheet_props")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Наименование"));
            ui.text_edit_singleline(&mut d.name);
            ui.end_row();
            ui.label(crate::i18n::tr("Комментарии"));
            ui.add(egui::TextEdit::multiline(&mut d.comment).desired_rows(3));
            ui.end_row();
            ui.label(crate::i18n::tr("Отключение"));
            ui.checkbox(&mut d.disabled, "")
                .on_hover_text(
                    crate::i18n::tr("Блоки отключённой платы не входят в сборку, а ошибки в них не проверяются.                      Блоки других плат, которые брали от них значения, останутся без источника."),
                );
            ui.end_row();
            ui.label(crate::i18n::tr("Фоновая задача"));
            ui.checkbox(&mut d.background, "")
                .on_hover_text(
                    crate::i18n::tr("Блоки этой платы выполняются в отдельной задаче FreeRTOS на ядре 0 (на ESP32 с двумя ядрами), \
                     не замедляя основной цикл: медленные датчики, запись во флеш. Сетевые блоки (Wi-Fi, MQTT, веб, Modbus) \
                     и так уходят на ядро 0 сами, на какой бы плате ни стояли. \
                     Связи с блоками других плат передают значения снимками в начале и конце прохода."),
                );
            ui.end_row();
            if d.background {
                ui.label(crate::i18n::tr("Период, мс"));
                ui.add(egui::DragValue::new(&mut d.period).range(1..=1000).suffix(crate::i18n::tr(" мс")))
                    .on_hover_text(
                        crate::i18n::tr("Только для одноядерных МК (ESP32-C3, S2…): второго ядра нет, поэтому плата \
                         остаётся в loop() и выполняется раз в этот период, не больше одной платы за проход; \
                         фазы фоновых плат разнесены по периоду."),
                    );
                ui.end_row();
            }
        });
}

/// Parses the initial value field; empty means "no initial value".
pub(super) fn parse_init(ty: VarType, text: &str) -> Result<Option<ParamValue>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let bad = || crate::i18n::fmt("«{0}» не подходит к типу {1}", &[&t, &ty.name()]);
    let value = match ty.signal() {
        SignalType::Bool => match t {
            "true" | "1" | "да" => ParamValue::Bool(true),
            "false" | "0" | "нет" => ParamValue::Bool(false),
            _ => return Err(bad()),
        },
        SignalType::Int => ParamValue::Int(t.parse().map_err(|_| bad())?),
        SignalType::Float => ParamValue::Float(t.replace(',', ".").parse().map_err(|_| bad())?),
        // Text keeps its spaces.
        SignalType::String | SignalType::Any | SignalType::Number | SignalType::Same => {
            ParamValue::Text(text.to_owned())
        }
    };
    if !ty.accepts(&value) {
        return Err(bad());
    }
    Ok(Some(value))
}
