use super::*;

/// Declaration offered in the "new expander" form: the first free id, the first chip.
pub(super) fn new_expander(logic: Option<&Logic>) -> Expander {
    blank_expander(&logic.map_or_else(|| "pcf1".to_owned(), free_expander_id))
}

/// An expander with the default chip and address under the given id.
pub(super) fn blank_expander(id: &str) -> Expander {
    let chip = ExpanderChip::default();
    Expander::new(id, chip, Expander::default_address(chip))
}

/// The declaration a project gets for an expander soldered on the board: the board's
/// chip, address and bus pins under a free id.
pub(super) fn board_expander(board: &crab_core::pins::BoardExpander, id: &str) -> Expander {
    let mut e = board.expander.clone();
    e.id = id.to_owned();
    e
}

pub(super) fn expander_title(e: &Expander) -> String {
    format!("{} · {} 0x{:02X}", e.id, e.chip.name(), e.address)
}

/// Declaration of an expander that is created together with the block.
pub(super) fn new_expander_form(ui: &mut Ui, e: &mut Expander, logic: &Logic) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new(crate::i18n::tr("Новый расширитель")).strong());
        expander_fields(ui, e, logic, None);
    });
}

/// Window of an expander in the project tree.
pub(super) fn expander_body(app: &EditorApp, ui: &mut Ui, d: &mut ExpanderDialog) {
    let Some(doc) = &app.doc else { return };
    expander_fields(ui, &mut d.form, &doc.project.logic, d.original.as_deref());
    if let Some(id) = &d.original {
        let blocks: Vec<&str> = doc
            .project
            .logic
            .blocks
            .iter()
            .filter(|b| b.expander.as_deref() == Some(id.as_str()))
            .map(|b| b.id.as_str())
            .collect();
        ui.add_space(6.0);
        if blocks.is_empty() {
            ui.weak(crate::i18n::tr(
                "На расширителе нет блоков: его можно удалить.",
            ));
        } else {
            ui.weak(crate::i18n::fmt(
                "На расширителе стоят: {0}.",
                &[&blocks.join(", ")],
            ));
        }
    }
}

/// Fields of an expander declaration; `original` is its id while it is being changed.
pub(super) fn expander_fields(
    ui: &mut Ui,
    e: &mut Expander,
    logic: &Logic,
    original: Option<&str>,
) {
    let red = Color32::from_rgb(186, 74, 66);
    {
        egui::Grid::new("new_expander")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label(crate::i18n::tr("Имя"));
                ui.text_edit_singleline(&mut e.id);
                ui.end_row();
                ui.label(crate::i18n::tr("Микросхема"));
                egui::ComboBox::from_id_salt("new_expander_chip")
                    .selected_text(e.chip.name())
                    .show_ui(ui, |ui| {
                        for chip in ExpanderChip::ALL {
                            let text = crate::i18n::fmt(
                                "{0} · {1} выводов",
                                &[&chip.name(), &chip.pins()],
                            );
                            if ui.selectable_label(e.chip == chip, text).clicked() {
                                e.chip = chip;
                                if !chip.addresses().contains(&e.address) {
                                    e.address = Expander::default_address(chip);
                                }
                            }
                        }
                    });
                ui.end_row();
                ui.label(crate::i18n::tr("Адрес I²C"));
                ui.add(
                    egui::DragValue::new(&mut e.address)
                        .range(e.chip.addresses())
                        .hexadecimal(2, true, false)
                        .prefix("0x"),
                )
                .on_hover_text(crate::i18n::tr(
                    "Задаётся перемычками A0–A2 на плате расширителя",
                ));
                ui.end_row();
                ui.label(crate::i18n::tr("Шина"));
                ui.horizontal(|ui| {
                    let mut own = e.sda >= 0;
                    if ui
                        .checkbox(&mut own, crate::i18n::tr("свои выводы"))
                        .on_hover_text(crate::i18n::tr(
                            "Иначе шина по умолчанию (GPIO21 и GPIO22 у ESP32)",
                        ))
                        .changed()
                    {
                        (e.sda, e.scl) = if own { (21, 22) } else { (-1, -1) };
                    }
                    if own {
                        ui.add(
                            egui::DragValue::new(&mut e.sda)
                                .range(0..=48)
                                .prefix("SDA "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut e.scl)
                                .range(0..=48)
                                .prefix("SCL "),
                        );
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::tr("Период опроса"));
                let period_ok = e.period_ms().is_some();
                let mut edit = egui::TextEdit::singleline(&mut e.period).desired_width(80.0);
                if !period_ok {
                    edit = edit.text_color(red);
                }
                ui.add(edit)
                    .on_hover_text(crate::i18n::tr("Как часто читаются входы: 20ms, 100ms…"));
                ui.end_row();
            });
        let others = || {
            logic
                .expanders
                .iter()
                .filter(|o| Some(o.id.as_str()) != original)
        };
        if others().any(|o| o.id == e.id) {
            ui.colored_label(red, crate::i18n::fmt("имя «{0}» уже занято", &[&e.id]));
        } else if let Some(problem) = e.problem() {
            ui.colored_label(red, problem);
        } else if let Some(other) =
            others().find(|o| o.address == e.address && o.sda == e.sda && o.scl == e.scl)
        {
            ui.colored_label(
                red,
                crate::i18n::fmt(
                    "адрес 0x{0} занят расширителем «{1}»",
                    &[&format!("{:02X}", e.address), &other.id],
                ),
            );
        }
    }
}
