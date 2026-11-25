//! Property inspector and problem list for the «Щит» mode.

use crab_core::{
    panel_binding, panel_tags, symbol_key, ContactForm, Device, DeviceKind, Report, Severity,
};
use crab_schematic::{Document, Rgb};
use eframe::egui::{self, Color32, RichText, Ui};

use super::tree::{apply, Action, Place};
use super::{is_about, refresh, DevicePayload};
use crate::app::EditorApp;
use crate::i18n::tr;
use crate::panels::row_label;

/// The row of the coil of a contactor or a relay in the tree: the output of the board it is on.
pub(super) fn coil_row(
    ui: &mut Ui,
    device: &Device,
    selected: bool,
    action: &std::cell::RefCell<Option<Action>>,
) {
    let caption = if device.bind.is_empty() {
        format!("{} -{}  A1 —", tr("Катушка"), device.id)
    } else {
        format!("{} -{}  A1 {}", tr("Катушка"), device.id, device.bind)
    };
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        // The coil can be dragged onto a sheet on its own, apart from its contactor or relay.
        let key = crab_core::part_key(&device.id, crab_core::COIL);
        let resp = ui
            .dnd_drag_source(
                ui.make_persistent_id(("panel_coil", &device.id)),
                DevicePayload(key),
                |ui| ui.selectable_label(selected, row_label(ui, caption, None, None)),
            )
            .inner;
        if resp.clicked() || resp.double_clicked() {
            *action.borrow_mut() = Some(Action::SelectCoil(device.id.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button(tr("Найти катушку на листе")).clicked() {
                *action.borrow_mut() = Some(Action::Find(device.id.clone(), Place::Control));
                ui.close();
            }
            if ui.button(tr("Свойства…")).clicked() {
                *action.borrow_mut() = Some(Action::SelectCoil(device.id.clone()));
                ui.close();
            }
        });
    });
}

/// The properties of a coil: the output of the board and where `A1` and `A2` go.
fn coil_properties(app: &mut EditorApp, ui: &mut Ui, id: &str) {
    let Some(doc) = &app.doc else {
        return;
    };
    let panel = &doc.project.panel;
    let Some(device) = panel.device(id).cloned() else {
        return;
    };
    let tags: Vec<String> = panel_tags(&doc.project, &app.lib)
        .into_iter()
        .filter(|t| !t.input && t.digital)
        .map(|t| t.name)
        .collect();
    // The circuit as the control sheet draws it: the output of the board, through the contacts in
    // series with the coil, to `A1`; `A2` to the 0 V line.
    let mut a1 = if device.bind.is_empty() {
        tr("не подключён").to_owned()
    } else {
        format!("{} {}", tr("выход платы"), device.bind)
    };
    for contact in panel
        .devices
        .iter()
        .filter(|c| c.kind == DeviceKind::Aux && c.series == id && panel.device(&c.of).is_some())
    {
        let (near, far) = crab_schematic::nets::aux_terminals(panel, contact);
        a1 = format!("{a1} → -{}:{near}-{far}", contact.of);
    }
    let a2 = if device.bind.is_empty() {
        tr("не подключён").to_owned()
    } else {
        tr("линия 0V").to_owned()
    };
    // A wire drawn by hand to a terminal of the coil replaces the automatic one.
    let by_hand = |terminal: &str| {
        let own = format!("{id}:{terminal}");
        panel.links.iter().find_map(|l| {
            let other = if l.from == own {
                &l.to
            } else if l.to == own {
                &l.from
            } else {
                return None;
            };
            Some(format!("{other} ({})", tr("вручную")))
        })
    };
    let a1 = by_hand("A1").unwrap_or(a1);
    let a2 = by_hand("A2").unwrap_or(a2);
    ui.heading(format!("{} -{id}", tr("Свойства катушки")));
    ui.separator();
    let mut edit = Edit::default();
    let document = app
        .panel_view
        .cache
        .as_ref()
        .and_then(|c| c.document.as_ref().ok());
    // The terminal of the coil and what it is connected to by hand (`None`: automatically).
    let mut connect: Option<(String, Option<String>)> = None;
    egui::Grid::new("panel_coil_props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("Выход платы (A1)"));
            let mut bind = device.bind.clone();
            let name = bind.split('.').next().unwrap_or_default().to_owned();
            egui::ComboBox::from_id_salt("panel_coil_bind")
                .selected_text(if bind.is_empty() {
                    tr("не привязано").to_owned()
                } else {
                    bind.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut bind, String::new(), tr("не привязано"));
                    for tag in &tags {
                        ui.selectable_value(&mut bind, tag.clone(), tag);
                    }
                    if !name.is_empty() && !tags.contains(&name) {
                        ui.selectable_value(&mut bind, device.bind.clone(), &device.bind);
                    }
                });
            if bind != device.bind {
                edit.bind = Some(bind);
            }
            ui.end_row();
            for (terminal, now) in [("A1", &a1), ("A2", &a2)] {
                ui.label(terminal);
                ui.vertical(|ui| {
                    ui.label(now);
                    if let Some(document) = document {
                        if let Some(choice) = connect_choice(ui, document, panel, id, terminal) {
                            connect = Some(choice);
                        }
                    }
                });
                ui.end_row();
            }
        });
    ui.weak(tr(
        "A1 и A2 подключаются к линии щита (+24V, 0V, L1, N…) или к выводу любого устройства: сначала выберите лист, потом вывод. «Автоматически» — как рисует генератор.",
    ));
    ui.add_space(6.0);
    if ui.button(tr("Найти катушку на листе")).clicked() {
        apply(app, Some(Action::Find(id.to_owned(), Place::Control)));
    }
    if let (Some((terminal, target)), Some(doc)) = (connect, app.doc.as_mut()) {
        let end = format!("{id}:{terminal}");
        if let Err(message) = doc.edit_panel(None, |p| {
            crab_core::connect_terminal(p, &end, target.as_deref())
        }) {
            app.panel_view.problem = Some(message);
        }
    }
    apply_edit(app, id, edit);
}

/// The places a terminal can be connected to by hand: the lines of the panel (`bus:+24V`,
/// `bus:L1`...) and every terminal drawn on the drawing sheet `sheet` (document number) but the
/// device's own, in order.
pub(super) fn connect_targets(
    document: &Document,
    panel: &crab_core::Panel,
    sheet: usize,
    own: &str,
) -> Vec<String> {
    let mut lines: Vec<String> = ["+24V", "0V"]
        .into_iter()
        .chain(panel.supply.lines().iter().copied())
        .map(|l| format!("bus:{l}"))
        .collect();
    lines.dedup();
    let mut terminals: Vec<String> = document
        .terminal_at
        .iter()
        .filter(|((device, _), at)| at.sheet == sheet && device != own)
        .map(|((device, terminal), _)| format!("{device}:{terminal}"))
        .collect();
    terminals.sort();
    terminals.dedup();
    lines.extend(terminals);
    lines
}

/// How a connection target is shown: a line by its name, a terminal as `-K2_1:14`.
pub(super) fn target_title(target: &str) -> String {
    match crab_core::link_bus(target) {
        Some(line) => format!("{} {line}", tr("линия")),
        None => format!("-{target}"),
    }
}

/// The two lists that connect the terminal `terminal` of `id` by hand: the sheet (the coil's own
/// at first), then the line or the terminal on it. Returns the choice made in this frame.
fn connect_choice(
    ui: &mut Ui,
    document: &Document,
    panel: &crab_core::Panel,
    id: &str,
    terminal: &str,
) -> Option<(String, Option<String>)> {
    let sheets: Vec<(usize, String)> = document
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.id.is_some())
        .map(|(i, s)| (i + 1, format!("{} {}", i + 1, s.title)))
        .collect();
    let own_sheet = document
        .terminal_at
        .get(&(id.to_owned(), terminal.to_owned()))
        .map(|at| at.sheet)
        .or_else(|| sheets.first().map(|(n, _)| *n))?;
    let key = ui.make_persistent_id(("panel_coil_sheet", id, terminal));
    let mut sheet = ui.data(|d| d.get_temp::<usize>(key)).unwrap_or(own_sheet);
    let mut choice = None;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("panel_coil_sheet_box", id, terminal))
            .selected_text(format!("{} {sheet}", tr("лист")))
            .show_ui(ui, |ui| {
                for (n, title) in &sheets {
                    ui.selectable_value(&mut sheet, *n, title);
                }
            });
        let now = format!("{id}:{terminal}");
        let current = panel.links.iter().find_map(|l| {
            if l.from == now {
                Some(l.to.clone())
            } else if l.to == now {
                Some(l.from.clone())
            } else {
                None
            }
        });
        egui::ComboBox::from_id_salt(("panel_coil_target", id, terminal))
            .selected_text(
                current
                    .as_deref()
                    .map_or_else(|| tr("Автоматически").to_owned(), target_title),
            )
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current.is_none(), tr("Автоматически"))
                    .clicked()
                {
                    choice = Some((terminal.to_owned(), None));
                }
                for target in connect_targets(document, panel, sheet, id) {
                    let on = current.as_deref() == Some(target.as_str());
                    if ui.selectable_label(on, target_title(&target)).clicked() && !on {
                        choice = Some((terminal.to_owned(), Some(target)));
                    }
                }
            });
    });
    ui.data_mut(|d| d.insert_temp(key, sheet));
    choice
}

/// Puts an auxiliary contact in series with the coil `series` (empty: on an input of the board).
/// The circuit is one thing, so the input is let go when a coil is chosen.
pub(super) fn set_series(contact: &mut Device, series: &str) {
    contact.series = series.to_owned();
    if !series.is_empty() {
        contact.bind.clear();
    }
}

/// Kinds that have auxiliary contacts.
pub(super) fn can_have_contacts(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Contactor | DeviceKind::Relay
    )
}

/// Name of the form of a contact.
fn form_title(form: ContactForm) -> String {
    match form {
        ContactForm::No => tr("Замыкающий"),
        ContactForm::Nc => tr("Размыкающий"),
        ContactForm::Changeover => tr("Переключающий"),
    }
    .to_owned()
}

/// The row of an auxiliary contact in the tree: the device, the number, the form and where it
/// is switched.
pub(super) fn aux_row(
    ui: &mut Ui,
    panel: &crab_core::Panel,
    layout: &crab_core::PanelLayout,
    contact: &Device,
    selected: &Option<String>,
    report: Option<&Report>,
    action: &std::cell::RefCell<Option<Action>>,
) {
    let number = panel
        .aux_of(&contact.of)
        .position(|d| d.id == contact.id)
        .map_or(1, |i| i + 1);
    let (near, far) = crab_schematic::nets::aux_terminals(panel, contact);
    let switched: Vec<&str> = panel
        .devices
        .iter()
        .filter(|d| d.from == contact.id)
        .map(|d| d.id.as_str())
        .chain((!contact.load.is_empty()).then_some(contact.load.as_str()))
        .collect();
    let circuit = if !contact.series.is_empty() {
        format!("→ -{}", contact.series)
    } else if !contact.bind.is_empty() {
        format!("→ {}", contact.bind)
    } else if !switched.is_empty() {
        format!("→ -{}", switched.join(", -"))
    } else {
        String::new()
    };
    // A contact of a relay is put on a sheet by itself (`specs/panel-relay-contacts`): dragged from
    // its row; the orange cross says it is on no sheet yet. The generator draws it where it stands
    // in a circuit: in the chain of a load, or in series with a coil, or on an input of the board.
    let relay_contact = panel.is_relay_contact(contact);
    let fed = !contact.from.is_empty() || panel.devices.iter().any(|d| d.load == contact.id);
    let in_circuit =
        !switched.is_empty() || fed || !contact.series.is_empty() || !contact.bind.is_empty();
    let off_sheet = relay_contact && !in_circuit && !layout.placed.contains_key(&contact.id);
    let cross = off_sheet.then(|| ui.visuals().warn_fg_color);
    let caption = format!(
        "-{}:{number}  {near}-{far}  {} {circuit}",
        contact.of,
        form_title(contact.form)
    );
    let problem = report.is_some_and(|r| {
        r.diagnostics
            .iter()
            .any(|d| is_about(&d.message, &contact.id))
    });
    let warn = problem.then(|| ui.visuals().warn_fg_color);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let chosen = selected.as_deref() == Some(contact.id.as_str());
        let resp = if relay_contact {
            ui.dnd_drag_source(
                ui.make_persistent_id(("panel_contact", &contact.id)),
                DevicePayload(contact.id.clone()),
                |ui| ui.selectable_label(chosen, row_label(ui, caption, warn, cross)),
            )
            .inner
        } else {
            ui.selectable_label(chosen, row_label(ui, caption, warn, None))
        };
        if resp.clicked() || resp.double_clicked() {
            *action.borrow_mut() = Some(Action::Select(contact.id.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button(tr("Найти на листе")).clicked() {
                *action.borrow_mut() = Some(Action::Find(contact.id.clone(), Place::Control));
                ui.close();
            }
            if ui.button(tr("Свойства…")).clicked() {
                *action.borrow_mut() = Some(Action::Select(contact.id.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(tr("Удалить из проекта")).clicked() {
                *action.borrow_mut() = Some(Action::Remove(contact.id.clone()));
                ui.close();
            }
        });
    });
}

pub(super) fn supply_name(supply: crab_core::Supply) -> &'static str {
    match supply {
        crab_core::Supply::Dc24 => "24 В постоянного тока",
        crab_core::Supply::Ac230 => "230 В, 1 фаза",
        crab_core::Supply::Ac400 => "400 В, 3 фазы",
    }
}

/// The properties of a text picked on the sheet: turned by 90°, the size of its font.
fn label_properties(app: &mut EditorApp, ui: &mut Ui, key: &str, drawn: f32) {
    let Some(doc) = &mut app.doc else {
        return;
    };
    let (mut vertical, size) = doc.project.panel_layout.label_style(key);
    let mut size = if size > 0.0 { size } else { drawn };
    ui.heading(tr("Надпись"));
    let (shift_x, shift_y) = doc.project.panel_layout.shift(key);
    let mut changed = false;
    egui::Grid::new("panel_label")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(tr("Повернуть на 90°"));
            changed |= ui.checkbox(&mut vertical, "").changed();
            ui.end_row();
            ui.label(tr("Размер шрифта"));
            changed |= ui
                .add(egui::DragValue::new(&mut size).range(4.0..=80.0).speed(0.5))
                .changed();
            ui.end_row();
        });
    if changed {
        let size = if (size - drawn).abs() < 0.5 {
            0.0
        } else {
            size
        };
        doc.edit_panel_layout(Some(format!("style:{key}")), |l| {
            l.set_label_style(key, vertical, size);
        });
        if let Some((_, d)) = &mut app.panel_view.label {
            *d = drawn;
        }
    }
    ui.add_space(6.0);
    let moved = shift_x != 0.0 || shift_y != 0.0 || vertical || size != drawn;
    if ui
        .add_enabled(moved, egui::Button::new(tr("Вернуть положение и вид")))
        .clicked()
    {
        doc.edit_panel_layout(None, |l| l.clear_symbol(key));
    }
    ui.add_space(6.0);
    if ui.button(tr("Закрыть")).clicked() {
        app.panel_view.label = None;
    }
}

/// The properties of the selected device and the problems of the panel (right panel).
pub fn properties(app: &mut EditorApp, ui: &mut Ui) {
    refresh(app);
    if let Some((key, drawn)) = app.panel_view.label.clone() {
        label_properties(app, ui, &key, drawn);
        return;
    }
    let Some(doc) = &app.doc else {
        ui.weak(tr("Проект не открыт"));
        return;
    };
    let selected = app
        .panel_view
        .selected
        .clone()
        .filter(|id| doc.project.panel.device(id).is_some());
    if let (true, Some(id)) = (app.panel_view.coil, &selected) {
        let coil = doc
            .project
            .panel
            .device(id)
            .is_some_and(|d| matches!(d.kind, DeviceKind::Contactor | DeviceKind::Relay));
        if coil {
            let id = id.clone();
            coil_properties(app, ui, &id);
            return;
        }
    }
    ui.heading(tr("Свойства устройства"));
    ui.separator();
    let Some(id) = selected else {
        ui.weak(tr(
            "Выберите устройство в дереве слева или добавьте новое кнопкой «+» у группы.",
        ));
        problems(app, ui);
        return;
    };
    let Some(device) = doc.project.panel.device(&id).cloned() else {
        return;
    };
    let tags = panel_binding(device.kind)
        .map(|(input, digital_only)| {
            panel_tags(&doc.project, &app.lib)
                .into_iter()
                .filter(|t| t.input == input && (t.digital || !digital_only))
                .map(|t| t.name)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut edit = Edit::default();
    let is_aux = device.kind == DeviceKind::Aux;
    // A contact of a relay switches a load of its own: it is fed like a one-pole device.
    let relay_contact = doc.project.panel.is_relay_contact(&device);
    // A contact either switches a load (it is fed) or stands in a control circuit: only the fields
    // of the one it does are shown (the check forbids both).
    let powered_contact = relay_contact
        && (!device.from.is_empty()
            || !device.load.is_empty()
            || doc
                .project
                .panel
                .devices
                .iter()
                .any(|d| d.load == id || d.from == id));
    let controlled_contact = is_aux && (!device.series.is_empty() || !device.bind.is_empty());
    // A new relay is a coil with contacts; the poles and the load of the relay itself stay only on
    // relays of old projects that have them (`specs/panel-relay-contacts`).
    let old_relay = device.kind == DeviceKind::Relay;
    let relay_poles = !old_relay || device.poles.is_some();
    egui::Grid::new("panel_props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            // The designation: applied when the field is left, so that a half-typed name never
            // renames the device.
            ui.label(tr("Обозначение"));
            let typed = app
                .panel_view
                .rename
                .as_ref()
                .filter(|(of, _)| *of == id)
                .map_or_else(|| id.clone(), |(_, text)| text.clone());
            let mut text = typed.clone();
            let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
            if text != typed {
                app.panel_view.rename = Some((id.clone(), text.clone()));
            }
            if resp.lost_focus() && text != id {
                edit.rename = Some(text);
            }
            ui.end_row();

            ui.label(tr("Параметры"));
            let mut rating = device.rating.clone();
            ui.add(
                egui::TextEdit::singleline(&mut rating)
                    .hint_text("230 В, 9 А")
                    .desired_width(160.0),
            );
            if rating != device.rating {
                edit.rating = Some(rating);
            }
            ui.end_row();

            if matches!(
                device.kind,
                DeviceKind::Breaker
                    | DeviceKind::Rcd
                    | DeviceKind::Fuse
                    | DeviceKind::Switch
                    | DeviceKind::Contactor
                    | DeviceKind::Relay
            ) && relay_poles
            {
                // 1 or 2 poles on 230 V, 3 or 4 on 400 V; the most is the default.
                let (lo, hi) = doc.project.panel.supply.pole_range();
                ui.label(tr("Полюсов"));
                let current = device.poles.map_or(hi, |p| p.clamp(lo, hi));
                let mut chosen = current;
                egui::ComboBox::from_id_salt("panel_poles")
                    .selected_text(chosen.to_string())
                    .show_ui(ui, |ui| {
                        for p in lo..=hi {
                            ui.selectable_value(&mut chosen, p, p.to_string());
                        }
                    });
                if chosen != current {
                    edit.poles = Some((chosen != hi).then_some(chosen));
                }
                ui.end_row();
            }

            ui.label(tr("Артикул"));
            let mut part = device.part.clone();
            ui.add(egui::TextEdit::singleline(&mut part).desired_width(160.0));
            if part != device.part {
                edit.part = Some(part);
            }
            ui.end_row();

            if device.kind == DeviceKind::Sensor {
                ui.label(tr("Через клеммник"));
                let mut via = device.via.clone();
                let shown = if via.is_empty() {
                    tr("напрямую").to_owned()
                } else {
                    format!("-{via}")
                };
                egui::ComboBox::from_id_salt("panel_via")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut via, String::new(), tr("напрямую"));
                        for block in doc
                            .project
                            .panel
                            .devices
                            .iter()
                            .filter(|d| d.kind == DeviceKind::Terminal)
                        {
                            ui.selectable_value(
                                &mut via,
                                block.id.clone(),
                                format!("-{}", block.id),
                            );
                        }
                    });
                if via != device.via {
                    edit.via = Some(via);
                }
                ui.end_row();
            }

            if device.kind == DeviceKind::Contactor {
                // Power contacts: their own coil, or the coil of another device.
                ui.label(tr("Катушка"));
                let mut of = device.of.clone();
                let shown = if of.is_empty() {
                    tr("своя").to_owned()
                } else {
                    format!("-{of}")
                };
                egui::ComboBox::from_id_salt("panel_power_coil")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut of, String::new(), tr("своя"));
                        for coil in doc
                            .project
                            .panel
                            .devices
                            .iter()
                            .filter(|d| d.id != device.id && doc.project.panel.has_own_coil(d))
                        {
                            ui.selectable_value(&mut of, coil.id.clone(), format!("-{}", coil.id));
                        }
                    });
                if of != device.of {
                    edit.power_coil = Some(of);
                }
                ui.end_row();
            }

            if is_aux {
                ui.label(tr("Аппарат"));
                let owner = doc.project.panel.device(&device.of).map(|d| d.kind);
                // The contacts of a breaker or an RCD stay with it. A contact of a relay or a
                // contactor may go to another coil, and a relay contact may be left free.
                if matches!(owner, Some(DeviceKind::Breaker | DeviceKind::Rcd)) {
                    ui.label(format!("-{}", device.of));
                } else {
                    let mut of = device.of.clone();
                    let free_allowed = device.of.is_empty() || owner == Some(DeviceKind::Relay);
                    let shown = if of.is_empty() {
                        tr("нет").to_owned()
                    } else {
                        format!("-{of}")
                    };
                    egui::ComboBox::from_id_salt("panel_of")
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            if free_allowed {
                                ui.selectable_value(&mut of, String::new(), tr("нет"));
                            }
                            for coil in doc.project.panel.devices.iter().filter(|d| {
                                matches!(d.kind, DeviceKind::Contactor | DeviceKind::Relay)
                            }) {
                                ui.selectable_value(
                                    &mut of,
                                    coil.id.clone(),
                                    format!("-{}", coil.id),
                                );
                            }
                        });
                    if of != device.of {
                        edit.of = Some(of);
                    }
                }
                ui.end_row();

                ui.label(tr("Тип контакта"));
                let mut form = device.form;
                egui::ComboBox::from_id_salt("panel_form")
                    .selected_text(form_title(form))
                    .show_ui(ui, |ui| {
                        for f in [ContactForm::No, ContactForm::Nc, ContactForm::Changeover] {
                            ui.selectable_value(&mut form, f, form_title(f));
                        }
                    });
                if form != device.form {
                    edit.form = Some(form);
                }
                ui.end_row();

                // Terminal numbers: by the number of the contact, or by hand (`21-22`).
                ui.label(tr("Выводы"));
                let mut pins = device.pins.clone();
                let all = crab_schematic::nets::aux_all_terminals(&doc.project.panel, &device);
                ui.add(
                    egui::TextEdit::singleline(&mut pins)
                        .hint_text(all.join("-"))
                        .desired_width(80.0),
                )
                .on_hover_text(tr(
                    "Номера выводов через дефис, например 21-22; пусто — по номеру контакта",
                ));
                if pins != device.pins {
                    edit.pins = Some(pins);
                }
                ui.end_row();
            }
            if is_aux && !powered_contact {
                // The circuit: an input of the board, or in series with a coil.
                ui.label(tr("Цепь контакта"));
                let mut series = device.series.clone();
                egui::ComboBox::from_id_salt("panel_series")
                    .selected_text(if series.is_empty() {
                        tr("вход платы").to_owned()
                    } else {
                        format!("{} -{series}", tr("последовательно с катушкой"))
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut series, String::new(), tr("вход платы"));
                        for coil in
                            doc.project.panel.devices.iter().filter(|d| {
                                matches!(d.kind, DeviceKind::Contactor | DeviceKind::Relay)
                            })
                        {
                            ui.selectable_value(
                                &mut series,
                                coil.id.clone(),
                                format!("{} -{}", tr("последовательно с катушкой"), coil.id),
                            );
                        }
                    });
                if series != device.series {
                    edit.series = Some(series);
                }
                ui.end_row();
            }

            let is_coil = matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay);
            // The coil has its own properties (its row in the tree).
            if panel_binding(device.kind).is_some()
                && !is_coil
                && !(is_aux && !device.series.is_empty())
                && !powered_contact
            {
                ui.label(tr("Привязка"));
                let mut bind = device.bind.clone();
                let name = bind.split('.').next().unwrap_or_default().to_owned();
                egui::ComboBox::from_id_salt("panel_bind")
                    .selected_text(if bind.is_empty() {
                        tr("не привязано").to_owned()
                    } else {
                        bind.clone()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut bind, String::new(), tr("не привязано"));
                        for tag in &tags {
                            ui.selectable_value(&mut bind, tag.clone(), tag);
                        }
                        // A binding that is not in the list (a deleted tag) stays visible.
                        if !name.is_empty() && !tags.contains(&name) {
                            ui.selectable_value(&mut bind, device.bind.clone(), &device.bind);
                        }
                    });
                if bind != device.bind {
                    edit.bind = Some(bind);
                }
                ui.end_row();
            }

            // The device this one hangs under: the one whose load it is.
            if !is_aux || (relay_contact && !controlled_contact) {
                let feeder = doc
                    .project
                    .panel
                    .devices
                    .iter()
                    .find(|d| d.load == id)
                    .map(|d| d.id.clone())
                    // Fed on a branch of a line that feeds another device too.
                    .unwrap_or_else(|| device.from.clone());
                // A device just added is connected to nothing until a place is chosen.
                let unplugged = device.free && feeder.is_empty() && device.from.is_empty();
                ui.label(tr("Питается от"));
                let mut chosen = if unplugged {
                    NOT_CONNECTED.to_owned()
                } else {
                    feeder.clone()
                };
                let shown = chosen.clone();
                egui::ComboBox::from_id_salt("panel_feeder")
                    .selected_text(if chosen == NOT_CONNECTED {
                        tr("не подключено").to_owned()
                    } else if chosen.is_empty() {
                        tr("напрямую от ввода").to_owned()
                    } else {
                        format!("-{chosen}")
                    })
                    .show_ui(ui, |ui| {
                        if unplugged {
                            ui.selectable_value(
                                &mut chosen,
                                NOT_CONNECTED.to_owned(),
                                tr("не подключено"),
                            );
                        }
                        ui.selectable_value(&mut chosen, String::new(), tr("напрямую от ввода"));
                        for d in doc.project.panel.devices.iter().filter(|d| {
                            d.id != id
                                && d.id != device.of
                                && (doc.project.panel.is_relay_contact(d)
                                    || matches!(
                                        d.kind,
                                        DeviceKind::Breaker
                                            | DeviceKind::Rcd
                                            | DeviceKind::Fuse
                                            | DeviceKind::Switch
                                            | DeviceKind::Psu
                                            | DeviceKind::Contactor
                                            | DeviceKind::Relay
                                            | DeviceKind::Terminal
                                    ))
                        }) {
                            ui.selectable_value(&mut chosen, d.id.clone(), format!("-{}", d.id));
                        }
                    });
                if chosen != shown {
                    edit.feeder = Some(chosen);
                }
                ui.end_row();

                // On 400 V a one-phase branch takes the phases it is given (`specs/panel-phase`).
                if doc.project.panel.supply == crab_core::Supply::Ac400 && !is_aux {
                    let nets = crab_schematic::build_nets(&doc.project, &app.lib);
                    let all: Vec<String> = crab_core::Supply::Ac400
                        .lines()
                        .iter()
                        .map(|l| (*l).to_owned())
                        .collect();
                    // Straight from the feed: every line.
                    let passed = nets.conductors.get(&feeder).or(Some(&all)).filter(|_| {
                        doc.project.panel.device(&feeder).is_none_or(|f| {
                            f.kind == DeviceKind::Feed || nets.conductors.contains_key(&feeder)
                        })
                    });
                    let choices = passed.map_or_else(Vec::new, |passed| {
                        phase_choices(device.kind, device.poles, passed)
                    });
                    if !choices.is_empty() {
                        let switching = has_poles(device.kind);
                        ui.label(tr(if switching { "Фаза" } else { "Фазы" }));
                        let default = tr(if switching {
                            "по умолчанию"
                        } else {
                            "как у питающего"
                        });
                        let current = device.conductors.clone();
                        let mut chosen = current.clone();
                        egui::ComboBox::from_id_salt("panel_phases")
                            .selected_text(phase_caption(&chosen, default))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut chosen, Vec::new(), default);
                                for c in choices {
                                    let caption = phase_caption(&c, default);
                                    ui.selectable_value(&mut chosen, c, caption);
                                }
                            });
                        if chosen != current {
                            edit.conductors = Some(chosen);
                        }
                        ui.end_row();
                    }
                }
            }
        });
    // After a terminal block the wires may be numbered by hand: one number to a conductor.
    if device.kind == DeviceKind::Terminal {
        let nets = crab_schematic::build_nets(&doc.project, &app.lib);
        let after: Vec<&crab_schematic::Wire> =
            nets.wires.iter().filter(|w| w.from.device == id).collect();
        if !after.is_empty() {
            ui.add_space(6.0);
            ui.label(tr("Номера проводов после клеммника"));
            egui::Grid::new("panel_wire_numbers")
                .num_columns(3)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for wire in after {
                        ui.label(&wire.from.terminal);
                        let mut number = wire.number;
                        let changed = ui
                            .add(egui::DragValue::new(&mut number).range(1..=u32::MAX))
                            .changed();
                        let keys: Vec<String> = nets
                            .wires
                            .iter()
                            .filter(|w| w.net == wire.net)
                            .map(crab_schematic::Wire::key)
                            .collect();
                        if changed {
                            edit.number = Some((keys.clone(), number));
                        }
                        if ui
                            .small_button(tr("Автоматически"))
                            .on_hover_text(tr(
                                "Снять ручной номер: провод получит следующий свободный",
                            ))
                            .clicked()
                        {
                            edit.reset_numbers = Some(keys);
                        }
                        ui.end_row();
                    }
                });
        }
    }
    if let Some(problem) = &app.panel_view.problem {
        ui.colored_label(app.theme.palette.error, problem);
    }
    ui.add_space(6.0);
    if ui.button(tr("Удалить из проекта")).clicked() {
        edit.remove = true;
    }
    problems(app, ui);
    apply_edit(app, &id, edit);
}

/// The value of the list «Питается от» for a device that is connected to nothing yet.
const NOT_CONNECTED: &str = "?";

/// Changes asked in the property fields during one frame.
#[derive(Default)]
struct Edit {
    rename: Option<String>,
    rating: Option<String>,
    part: Option<String>,
    /// Number of poles (`Some(None)`: all the conductors of the supply).
    poles: Option<Option<u8>>,
    bind: Option<String>,
    /// The device that feeds this one (empty: straight from the feed).
    feeder: Option<String>,
    /// Form of an auxiliary contact.
    form: Option<ContactForm>,
    /// Terminal numbers of an auxiliary contact.
    pins: Option<String>,
    /// A number given by hand to the wires of a conductor after a terminal block.
    number: Option<(Vec<String>, u32)>,
    /// Wires whose hand-made number is taken back.
    reset_numbers: Option<Vec<String>>,
    /// The coil an auxiliary contact stands in series with (empty: an input of the board).
    series: Option<String>,
    /// The conductors a device takes on 400 V (empty: by default).
    conductors: Option<Vec<String>>,
    /// Another coil the contact belongs to (empty: a free contact).
    of: Option<String>,
    /// The terminal block a sensor is wired through (empty: straight).
    via: Option<String>,
    power_coil: Option<String>,
    remove: bool,
}

/// Switching devices with poles `1/2`, `3/4`, ...
fn has_poles(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Fuse
            | DeviceKind::Switch
            | DeviceKind::Contactor
            | DeviceKind::Relay
    )
}

/// The phases a device on 400 V may take from the conductors `passed` to it by its feeder; the
/// default (no `conductors`) is not in the list. A switching device chooses with one or two poles
/// (a phase; a phase and the neutral or two phases); a load, a motor or a power supply under a
/// device that passes on two or three phases chooses a phase with the neutral or two phases. The
/// neutral is always there: it passes by from the bus.
pub(super) fn phase_choices(
    kind: DeviceKind,
    poles: Option<u8>,
    passed: &[String],
) -> Vec<Vec<String>> {
    let phases: Vec<&str> = ["L1", "L2", "L3"]
        .into_iter()
        .filter(|l| passed.iter().any(|p| p == l))
        .collect();
    let with_n = || phases.iter().map(|l| vec![(*l).to_owned(), "N".to_owned()]);
    let pairs = || {
        phases.iter().enumerate().flat_map(|(i, a)| {
            phases[i + 1..]
                .iter()
                .map(|b| vec![(*a).to_owned(), (*b).to_owned()])
        })
    };
    if has_poles(kind) {
        match poles.unwrap_or(4) {
            1 => phases.iter().map(|l| vec![(*l).to_owned()]).collect(),
            2 => with_n().chain(pairs()).collect(),
            _ => Vec::new(),
        }
    } else if matches!(kind, DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu)
        && phases.len() > 1
    {
        with_n().chain(pairs()).collect()
    } else {
        Vec::new()
    }
}

/// `L2 + N`; nothing chosen: `default`.
pub(super) fn phase_caption(conductors: &[String], default: &str) -> String {
    if conductors.is_empty() {
        default.to_owned()
    } else {
        conductors.join(" + ")
    }
}

fn apply_edit(app: &mut EditorApp, id: &str, edit: Edit) {
    let Some(doc) = &mut app.doc else {
        return;
    };
    let key = |what: &str| Some(format!("panel:{what}:{id}"));
    let field = |doc: &mut crate::doc::Doc, what: &str, f: &dyn Fn(&mut Device)| {
        let _ = doc.edit_panel::<(), ()>(key(what), |p| {
            if let Some(d) = p.devices.iter_mut().find(|d| d.id == id) {
                f(d);
            }
            Ok(())
        });
    };
    if let Some(rating) = edit.rating {
        field(doc, "rating", &|d| d.rating = rating.clone());
    }
    if let Some(via) = edit.via {
        field(doc, "via", &|d| d.via = via.clone());
    }
    if let Some(of) = edit.power_coil {
        field(doc, "of", &|d| d.of = of.clone());
    }
    if let Some((keys, number)) = edit.number {
        // Two conductors never share a number.
        let nets = crab_schematic::build_nets(&doc.project, &app.lib);
        let net = nets
            .wires
            .iter()
            .find(|w| keys.contains(&w.key()))
            .map(|w| w.net);
        let taken = nets
            .wires
            .iter()
            .find(|w| w.number == number && Some(w.net) != net);
        if let Some(other) = taken {
            app.panel_view.problem = Some(format!(
                "{} {number}: {}",
                tr("Номер"),
                tr("уже у другого провода").to_owned() + &format!(" ({})", other.key())
            ));
        } else {
            let _ = doc.edit_panel::<(), ()>(
                Some(format!("panel:number:{id}:{}", keys.join(","))),
                |p| {
                    for key in &keys {
                        p.wire_numbers.insert(key.clone(), number);
                    }
                    Ok(())
                },
            );
            app.panel_view.problem = None;
        }
    }
    if let Some(keys) = edit.reset_numbers {
        let _ = doc.edit_panel::<(), ()>(None, |p| {
            for key in &keys {
                p.wire_numbers.remove(key);
            }
            Ok(())
        });
        app.panel_view.problem = None;
    }
    if let Some(form) = edit.form {
        field(doc, "form", &|d| d.form = form);
    }
    if let Some(pins) = edit.pins {
        field(doc, "pins", &|d| d.pins = pins.trim().to_owned());
    }
    if let Some(series) = edit.series {
        // The circuit is one thing: the coil or the input of the board.
        field(doc, "series", &|d| set_series(d, &series));
    }
    if let Some(poles) = edit.poles {
        // Phases chosen for another number of poles no longer fit.
        field(doc, "poles", &|d| {
            d.poles = poles;
            let n = usize::from(poles.unwrap_or(4));
            if !d.conductors.is_empty() && d.conductors.len() != n && has_poles(d.kind) {
                d.conductors.clear();
            }
        });
    }
    if let Some(conductors) = edit.conductors {
        field(doc, "conductors", &|d| d.conductors = conductors.clone());
    }
    if let Some(part) = edit.part {
        field(doc, "part", &|d| d.part = part.clone());
    }
    if let Some(bind) = edit.bind {
        field(doc, "bind", &|d| d.bind = bind.clone());
    }
    if let Some(parent) = edit.feeder {
        // A source that already feeds another device feeds this one on a branch of its line.
        let parent = (!parent.is_empty()).then_some(parent);
        match doc.edit_panel(None, |p| p.feed_from(id, parent.as_deref())) {
            Ok(()) => {
                // The device goes under its new place by itself.
                doc.edit_panel_layout(None, |l| l.clear_symbol(&symbol_key("power", id)));
                app.panel_view.problem = None;
            }
            Err(message) => app.panel_view.problem = Some(message),
        }
    }
    if let Some(of) = edit.of {
        // The contact takes the next free designation of its new coil; the layout and the links
        // follow it, and it is one undo step.
        match doc.edit_panel_project(None, |p| crab_core::reassign_contact(p, id, &of)) {
            Ok(new) => {
                app.panel_view.selected = Some(new);
                app.panel_view.rename = None;
                app.panel_view.problem = None;
            }
            Err(message) => app.panel_view.problem = Some(message),
        }
    }
    if let Some(new) = edit.rename {
        match doc.edit_panel_project(None, |p| crab_core::rename_device(p, id, &new)) {
            Ok(()) => {
                app.panel_view.selected = Some(new);
                app.panel_view.rename = None;
                app.panel_view.problem = None;
            }
            Err(message) => {
                app.panel_view.problem = Some(message);
                app.panel_view.rename = None;
            }
        }
    }
    if edit.remove {
        apply(app, Some(Action::Remove(id.to_owned())));
    }
}

/// The problems the check of the panel found.
fn problems(app: &EditorApp, ui: &mut Ui) {
    let Some(cache) = &app.panel_view.cache else {
        return;
    };
    let pal = app.theme.palette;
    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new(tr("Проверка щита")).strong());
    if cache.report.diagnostics.is_empty() {
        ui.colored_label(pal.ok, tr("Ошибок нет"));
        return;
    }
    for d in &cache.report.diagnostics {
        let color = if d.severity == Severity::Error {
            pal.error
        } else {
            pal.warning
        };
        ui.add(egui::Label::new(RichText::new(&d.message).color(color)).wrap());
    }
}

pub(super) fn color(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}
