//! Hardware UARTs in the left panel: one flat row per UART of the chip with a switch; a double click
//! opens the settings (speed, format, pins, buffer). A UART set up by a settings block on a sheet has
//! no switch: it is changed in that block. USB (flashing, debugging, the monitor) needs no setup.

use crab_core::pins::BoardLayout;
use crab_core::uart::{block_configs, block_uarts, chip_uarts, UartConfig, USB_SPEED};
use eframe::egui::{self, Ui};

use crate::app::EditorApp;
use crate::dialogs::Dialog;

pub enum UartAction {
    Toggle { number: u8, on: bool },
    Settings(u8),
}

/// The settings of one UART being edited.
pub struct UartDialog {
    pub cfg: UartConfig,
}

impl UartDialog {
    pub fn body(&mut self, ui: &mut Ui) {
        grid(ui, &mut self.cfg);
    }
}

/// «UART» folder of the left panel.
pub fn show(app: &EditorApp, ui: &mut Ui) -> Vec<UartAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let chip = BoardLayout::of_board(&doc.project.config.board, app.lib.boards()).map(|l| l.chip());
    let mut numbers: Vec<u8> = chip.map_or(vec![1, 2], |c| chip_uarts(c).to_vec());
    // A UART set up for another board stays visible, so it can be switched off.
    for u in &doc.project.config.uart {
        if !numbers.contains(&u.number) {
            numbers.push(u.number);
        }
    }
    numbers.sort_unstable();
    let from_blocks: Vec<(u8, String)> = block_configs(&doc.project.logic, &app.lib)
        .into_iter()
        .map(|(id, u)| (u.number, id))
        .collect();
    let users: Vec<(u8, String)> = block_uarts(&doc.project.logic, &app.lib)
        .into_iter()
        .map(|u| (u.number, u.block))
        .collect();
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new("UART").strong())
        .id_salt("uart_panel")
        .default_open(!doc.project.config.uart.is_empty() || !users.is_empty())
        .show(ui, |ui| {
            ui.weak(crate::i18n::fmt("USB — заливка, отладка и монитор порта, {0} бод, без настроек. UART0 у ESP32 занят USB-мостом, у ESP32-C3/S3 — свободный порт.", &[&USB_SPEED]));
            for number in numbers {
                row(app, ui, number, chip, &from_blocks, &users, &mut actions);
            }
        });
    ui.separator();
    actions
}

fn row(
    app: &EditorApp,
    ui: &mut Ui,
    number: u8,
    chip: Option<crab_core::pins::Chip>,
    from_blocks: &[(u8, String)],
    users: &[(u8, String)],
    actions: &mut Vec<UartAction>,
) {
    let Some(doc) = &app.doc else { return };
    let cfg = doc.project.config.uart.iter().find(|u| u.number == number);
    let by_block = from_blocks.iter().find(|(n, _)| *n == number);
    let who: Vec<&str> = users
        .iter()
        .filter(|(n, id)| *n == number && !from_blocks.iter().any(|(_, b)| b == id))
        .map(|(_, id)| id.as_str())
        .collect();
    let mut caption = format!("UART{number}");
    if let Some((_, id)) = by_block {
        caption.push_str(&crate::i18n::fmt(" · настроен блоком «{0}»", &[&id]));
    } else if let Some(u) = cfg {
        caption.push_str(&format!(" · {} {}", u.speed, u.format));
    }
    if !who.is_empty() {
        caption.push_str(&crate::i18n::fmt(" · используют: {0}", &[&who.join(", ")]));
    }
    let missing = chip.is_some_and(|c| !chip_uarts(c).contains(&number));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let text = if missing {
            egui::RichText::new(format!(
                "{caption} · {}",
                crate::i18n::tr("нет на этой плате")
            ))
            .color(ui.visuals().error_fg_color)
        } else {
            egui::RichText::new(caption)
        };
        let resp = ui.selectable_label(false, text);
        if by_block.is_none() {
            let resp = resp.on_hover_text(crate::i18n::tr("Двойной щелчок — настройки"));
            if resp.double_clicked() && cfg.is_some() {
                actions.push(UartAction::Settings(number));
            }
            resp.context_menu(|ui| {
                if cfg.is_some() && ui.button(crate::i18n::tr("Настройки...")).clicked() {
                    actions.push(UartAction::Settings(number));
                    ui.close();
                }
                if cfg.is_some() && ui.button(crate::i18n::tr("Выключить")).clicked() {
                    actions.push(UartAction::Toggle { number, on: false });
                    ui.close();
                }
            });
            let mut on = cfg.is_some();
            if crate::dialogs::toggle_switch(ui, &mut on).changed() {
                actions.push(UartAction::Toggle { number, on });
            }
        }
    });
}

pub fn apply(app: &mut EditorApp, actions: Vec<UartAction>) {
    for action in actions {
        match action {
            UartAction::Settings(number) => {
                let Some(doc) = &app.doc else { return };
                if let Some(cfg) = doc.project.config.uart.iter().find(|u| u.number == number) {
                    app.dialog = Some(Dialog::Uart(UartDialog { cfg: cfg.clone() }));
                }
            }
            UartAction::Toggle { number, on } => {
                let lib = app.lib.clone();
                let Some(doc) = &mut app.doc else { return };
                let uarts = &mut doc.project.config.uart;
                let index = uarts.iter().position(|u| u.number == number);
                match (on, index) {
                    (true, None) => uarts.push(UartConfig::new(number)),
                    (false, Some(i)) => {
                        uarts.remove(i);
                    }
                    _ => continue,
                }
                uarts.sort_by_key(|u| u.number);
                changed(doc, &lib);
            }
        }
    }
}

/// Stores the settings of a UART from its dialog.
pub fn commit(app: &mut EditorApp, cfg: &UartConfig) {
    let lib = app.lib.clone();
    let Some(doc) = &mut app.doc else { return };
    if let Some(u) = doc
        .project
        .config
        .uart
        .iter_mut()
        .find(|u| u.number == cfg.number)
    {
        if u != cfg {
            *u = cfg.clone();
            changed(doc, &lib);
        }
    }
}

/// Pins and UARTs are checked against the project again.
fn changed(doc: &mut crate::doc::Doc, lib: &crab_core::Library) {
    doc.dirty = true;
    doc.revision += 1;
    doc.recheck(lib);
}

/// Speeds offered for a UART; any other can be typed.
const SPEEDS: [u32; 10] = [
    1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800,
];
const FORMATS: [&str; 8] = ["8N1", "8E1", "8O1", "8N2", "8E2", "8O2", "7E1", "7O1"];

/// Speed, format, pins and buffer of one UART.
fn grid(ui: &mut Ui, u: &mut UartConfig) {
    let id = u.number;
    egui::Grid::new(("uart_grid", id))
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Скорость, бод"));
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("uart_speed", id))
                    .selected_text(u.speed.to_string())
                    .show_ui(ui, |ui| {
                        for s in SPEEDS {
                            ui.selectable_value(&mut u.speed, s, s.to_string());
                        }
                    });
                ui.add(egui::DragValue::new(&mut u.speed).range(300..=5_000_000));
            });
            ui.end_row();

            ui.label(crate::i18n::tr("Формат"));
            egui::ComboBox::from_id_salt(("uart_format", id))
                .selected_text(u.format.as_str())
                .show_ui(ui, |ui| {
                    for f in FORMATS {
                        ui.selectable_value(&mut u.format, f.to_owned(), f);
                    }
                })
                .response
                .on_hover_text(crate::i18n::tr(
                    "биты данных, чётность (N — нет, E — чёт, O — нечет), стоп-биты",
                ));
            ui.end_row();

            for (label, pin, hint) in [
                (
                    "RX",
                    &mut u.rx,
                    crate::i18n::tr("-1 — вывод чипа по умолчанию"),
                ),
                (
                    "TX",
                    &mut u.tx,
                    crate::i18n::tr("-1 — вывод чипа по умолчанию"),
                ),
                (
                    "DE/RE (RS-485)",
                    &mut u.de,
                    crate::i18n::tr("-1 — без RS-485"),
                ),
            ] {
                ui.label(label);
                ui.add(egui::DragValue::new(pin).range(-1..=48).prefix("GPIO "))
                    .on_hover_text(hint);
                ui.end_row();
            }

            ui.label(crate::i18n::tr("Буфер приёма, байт"));
            ui.add(egui::DragValue::new(&mut u.rx_buffer).range(64..=8192));
            ui.end_row();
        });
}
