//! "MCU settings" dialog: board and upload port.

use std::collections::BTreeMap;

use crab_build::SerialPort;
use crab_core::pins::{BoardModel, BOARD_MODELS};
use crab_core::{BoardConfig, Boards};
use eframe::egui::{self, RichText, Ui};

/// Common ESP32 boards: FQBN and name.
pub(crate) const BOARDS: &[(&str, &str)] = &[
    ("esp32:esp32:esp32", "ESP32 Dev Module"),
    ("esp32:esp32:esp32s2", "ESP32-S2 Dev Module"),
    ("esp32:esp32:esp32s3", "ESP32-S3 Dev Module"),
    ("esp32:esp32:esp32c3", "ESP32-C3 Dev Module"),
    ("esp32:esp32:esp32doit-devkit-v1", "DOIT ESP32 DEVKIT V1"),
    ("esp32:esp32:nodemcu-32s", "NodeMCU-32S"),
    ("esp32:esp32:lolin32", "WEMOS LOLIN32"),
];

/// Human-readable board name: its model (built-in or the user's), the list of common
/// boards, otherwise the last FQBN part.
pub fn board_name<'a>(board: &'a BoardConfig, boards: &'a Boards) -> &'a str {
    let fqbn = board.fqbn.as_str();
    if let Some(m) = board.model.as_deref().and_then(BoardModel::find) {
        if m.fqbn == fqbn {
            return m.name;
        }
    }
    if let Some(b) = board
        .model
        .as_deref()
        .and_then(|id| boards.get(id))
        .filter(|b| b.def.info.fqbn == fqbn)
    {
        return &b.def.info.name;
    }
    BOARDS
        .iter()
        .find(|(f, _)| *f == fqbn)
        .map_or_else(|| fqbn.rsplit(':').next().unwrap_or(fqbn), |(_, name)| name)
}

pub struct BoardDialog {
    fqbn: String,
    /// Id of the exact board picked from the list (built-in model or user board);
    /// dropped when the FQBN no longer matches it.
    model: Option<String>,
    /// User boards: their pin tables, FQBNs and presets.
    boards: Boards,
    port: String,
    /// Network address of the board: set from the toolbar, kept as it is.
    host: Option<String>,
    /// The user asked for the board editor: `Some(None)` — a new board, `Some(Some(id))` —
    /// the board `id`. The caller takes it with [`BoardDialog::take_editor_request`].
    editor_request: Option<Option<String>>,
}

impl BoardDialog {
    pub fn new(board: &BoardConfig, boards: &Boards) -> Self {
        Self {
            editor_request: None,
            fqbn: board.fqbn.clone(),
            model: board.model.clone(),
            boards: boards.clone(),
            port: board.port.clone().unwrap_or_default(),
            host: board.host.clone(),
        }
    }

    /// Settings to write to the project file.
    pub fn config(&self) -> Result<BoardConfig, String> {
        let fqbn = self.fqbn.trim();
        if fqbn.split(':').count() < 3 {
            return Err(crate::i18n::fmt(
                "неверное имя платы «{0}»: ожидается вид пакет:архитектура:плата",
                &[&fqbn],
            ));
        }
        let port = self.port.trim();
        let model = self
            .model
            .clone()
            .filter(|id| self.model_fqbn(id) == Some(fqbn));
        Ok(BoardConfig {
            fqbn: fqbn.to_owned(),
            model,
            port: (!port.is_empty()).then(|| port.to_owned()),
            host: self.host.clone(),
            options: BTreeMap::new(),
        })
    }

    /// FQBN of a board model: built-in or user.
    fn model_fqbn(&self, id: &str) -> Option<&str> {
        BoardModel::find(id)
            .map(|m| m.fqbn)
            .or_else(|| self.boards.get(id).map(|b| b.def.info.fqbn.as_str()))
    }

    /// Name of the picked board model, while the FQBN still belongs to it.
    fn model_title(&self) -> Option<&str> {
        let id = self.model.as_deref()?;
        if self.model_fqbn(id)? != self.fqbn.trim() {
            return None;
        }
        BoardModel::find(id)
            .map(|m| m.name)
            .or_else(|| self.boards.get(id).map(|b| b.def.info.name.as_str()))
    }

    /// What the user asked of the board editor since the last call (see the field).
    pub fn take_editor_request(&mut self) -> Option<Option<String>> {
        self.editor_request.take()
    }

    pub fn body(&mut self, ui: &mut Ui, ports: &[SerialPort]) {
        egui::Grid::new("board_main")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label(crate::i18n::tr("Плата (FQBN)"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.fqbn).desired_width(220.0));
                    // (FQBN, board id)
                    let mut picked: Option<(String, Option<String>)> = None;
                    ui.menu_button("▾", |ui| {
                        for (fqbn, name) in BOARDS {
                            if board_button(ui, name, fqbn).clicked() {
                                picked = Some(((*fqbn).to_owned(), None));
                                ui.close();
                            }
                        }
                        ui.separator();
                        for m in BOARD_MODELS {
                            if board_button(ui, m.name, m.fqbn).clicked() {
                                picked = Some((m.fqbn.to_owned(), Some(m.id.to_owned())));
                                ui.close();
                            }
                        }
                        if !self.boards.is_empty() {
                            ui.separator();
                            for b in self.boards.iter() {
                                let info = &b.def.info;
                                if board_button(ui, &info.name, &info.fqbn).clicked() {
                                    picked = Some((info.fqbn.clone(), Some(info.id.clone())));
                                    ui.close();
                                }
                            }
                        }
                    });
                    if let Some((fqbn, model)) = picked {
                        self.fqbn = fqbn;
                        self.model = model;
                    }
                    if let Some(name) = self.model_title() {
                        ui.label(RichText::new(name).weak());
                    }
                    let user_board = self
                        .model
                        .clone()
                        .filter(|id| self.model_title().is_some() && self.boards.get(id).is_some());
                    if let Some(id) = user_board {
                        if ui
                            .small_button(crate::i18n::tr("Изменить плату..."))
                            .on_hover_text(crate::i18n::tr(
                                "Выводы, периферия и описание этой платы",
                            ))
                            .clicked()
                        {
                            self.editor_request = Some(Some(id));
                        }
                    }
                    if ui
                        .small_button(crate::i18n::tr("Новая плата..."))
                        .on_hover_text(crate::i18n::tr(
                            "Описать свою плату: выводы, периферию, схемы",
                        ))
                        .clicked()
                    {
                        self.editor_request = Some(None);
                    }
                });
                ui.end_row();

                ui.label(crate::i18n::tr("Порт"));
                egui::ComboBox::from_id_salt("board_port")
                    .selected_text(if self.port.is_empty() {
                        crate::i18n::tr("авто")
                    } else {
                        self.port.as_str()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.port, String::new(), crate::i18n::tr("авто"));
                        for p in ports {
                            ui.selectable_value(
                                &mut self.port,
                                p.address.clone(),
                                format!("{} — {}", p.address, p.description),
                            );
                        }
                    });
                ui.end_row();
            });
    }
}

/// A board in the picker.
fn board_button(ui: &mut Ui, name: &str, fqbn: &str) -> egui::Response {
    ui.add(egui::Button::new(name)).on_hover_text(fqbn)
}

#[cfg(test)]
mod tests {
    use crab_core::boards::{BoardDef, BoardInfo};
    use crab_core::UserBoard;

    use super::*;

    fn boards() -> Boards {
        let mut boards = Boards::default();
        boards.insert(UserBoard {
            def: BoardDef {
                info: BoardInfo {
                    id: "kc868".into(),
                    name: "KC868".into(),
                    chip: "esp32".into(),
                    fqbn: "esp32:esp32:esp32".into(),
                    version: 1,
                    options: BTreeMap::new(),
                },
                pins: Vec::new(),
                expander_pins: Vec::new(),
                devices: Vec::new(),
            },
            dir: "kc868".into(),
        });
        boards
    }

    fn project_board(model: Option<&str>) -> BoardConfig {
        BoardConfig {
            fqbn: "esp32:esp32:esp32".into(),
            model: model.map(str::to_owned),
            port: None,
            host: None,
            options: BTreeMap::new(),
        }
    }

    #[test]
    fn a_user_board_has_its_name_and_a_stale_model_is_dropped() {
        let boards = boards();
        assert_eq!(board_name(&project_board(Some("kc868")), &boards), "KC868");
        // Another FQBN than the board's: the generic name.
        let mut other = project_board(Some("kc868"));
        other.fqbn = "esp32:esp32:esp32s3".into();
        assert_eq!(board_name(&other, &boards), "ESP32-S3 Dev Module");
        assert_eq!(
            board_name(&project_board(None), &boards),
            "ESP32 Dev Module"
        );

        let mut empty = project_board(Some("kc868"));
        empty.fqbn.clear();
        let mut dialog = BoardDialog::new(&empty, &boards);
        dialog.fqbn = "esp32:esp32:esp32".into();
        assert_eq!(dialog.model_title(), Some("KC868"));
        assert_eq!(dialog.config().unwrap().model.as_deref(), Some("kc868"));
        dialog.fqbn = "esp32:esp32:esp32s3".into();
        assert_eq!(dialog.model_title(), None);
        assert_eq!(dialog.config().unwrap().model, None);
    }
}
