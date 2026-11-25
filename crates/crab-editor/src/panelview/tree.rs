//! Device tree and element palette for the «Щит» mode.

use crab_core::{ContactForm, Device, DeviceKind, SheetId};
use eframe::egui::{self, Color32, Ui, Vec2};

use super::canvas::{draw_ops, View};
use super::properties::{aux_row, can_have_contacts, coil_row, supply_name};
use super::{
    is_about, kind_title, palette_entries, refresh, DevicePayload, PalettePayload, GROUPS,
};
use crate::app::EditorApp;
use crate::i18n::tr;
use crate::panels::{group, row_label, Level};

pub(super) enum Action {
    Select(String),
    /// The coil of a contactor or a relay.
    SelectCoil(String),
    /// A palette entry clicked: its device is made without a place on a sheet.
    Add(PalettePayload),
    /// An auxiliary contact for the device.
    AddAux(String),
    /// A temperature sensor of a motor.
    AddThermistor(String),
    Remove(String),
    Find(String, Place),
    /// Designations with the number of the sheet for all devices but the terminal blocks.
    SheetNames,
}

/// Which drawing of a device to look for.
#[derive(Clone, Copy)]
pub(super) enum Place {
    /// The contacts or the symbol of the power sheets.
    Power,
    /// The coil, the sensor or the button of the control sheets.
    Control,
}

/// The tree of the devices (left panel).
pub fn tree(app: &mut EditorApp, ui: &mut Ui) {
    refresh(app);
    let Some(doc) = &app.doc else {
        ui.weak(tr("Проект не открыт"));
        return;
    };
    let panel = &doc.project.panel;
    let layout = &doc.project.panel_layout;
    let report = app.panel_view.cache.as_ref().map(|c| &c.report);
    let selected = app.panel_view.selected.clone();
    let coil_selected = app.panel_view.coil;
    let action = std::cell::RefCell::new(None::<Action>);
    let mut supply = panel.supply;

    ui.horizontal(|ui| {
        ui.label(tr("Питание щита"));
        egui::ComboBox::from_id_salt("panel_supply")
            .selected_text(supply_name(supply))
            .show_ui(ui, |ui| {
                for s in [
                    crab_core::Supply::Dc24,
                    crab_core::Supply::Ac230,
                    crab_core::Supply::Ac400,
                ] {
                    ui.selectable_value(&mut supply, s, supply_name(s));
                }
            });
    });
    // Designations of this kind (`K17.1`) that no longer match the sheet their device is on.
    let stale = app
        .panel_view
        .cache
        .as_ref()
        .and_then(|c| c.document.as_ref().ok())
        .map_or(0, |d| {
            crab_schematic::sheet_designations(d, panel)
                .iter()
                .filter(|(old, new)| old != new && is_sheet_designation(old))
                .count()
        });
    if stale > 0 {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("{} {stale}", tr("Обозначения не совпадают с листами:")),
        );
    }
    if ui
        .button(tr("Обозначения по листам"))
        .on_hover_text(tr(
            "Дать устройствам (кроме клеммников) обозначения с номером листа: K17.1, Q2.1...",
        ))
        .clicked()
    {
        *action.borrow_mut() = Some(Action::SheetNames);
    }
    ui.separator();

    palette(ui, panel.supply, &action);
    ui.separator();

    for (title, kinds) in GROUPS {
        // New devices come from the palette above; the groups list what the panel has.
        let tools = |_: &mut Ui| {};
        let body = |ui: &mut Ui| {
            for device in panel.devices.iter().filter(|d| kinds.contains(&d.kind)) {
                let problem = report.is_some_and(|r| {
                    r.diagnostics
                        .iter()
                        .any(|d| is_about(&d.message, &device.id))
                });
                let warn = problem.then(|| ui.visuals().warn_fg_color);
                let caption = if device.rating.is_empty() {
                    device.designation()
                } else {
                    format!("{}  {}", device.designation(), device.rating)
                };
                // The row can be dragged onto a control sheet.
                let resp = ui
                    .dnd_drag_source(
                        ui.make_persistent_id(("panel_device", &device.id)),
                        DevicePayload(device.id.clone()),
                        |ui| {
                            ui.selectable_label(
                                selected.as_deref() == Some(device.id.as_str()),
                                row_label(ui, caption, warn, None),
                            )
                        },
                    )
                    .inner;
                if resp.clicked() {
                    *action.borrow_mut() = Some(Action::Select(device.id.clone()));
                }
                if resp.double_clicked() {
                    *action.borrow_mut() = Some(Action::Select(device.id.clone()));
                }
                resp.context_menu(|ui| {
                    if device.kind == DeviceKind::Motor {
                        if ui.button(tr("Добавить датчик температуры")).clicked()
                        {
                            *action.borrow_mut() = Some(Action::AddThermistor(device.id.clone()));
                            ui.close();
                        }
                        ui.separator();
                    }
                    if can_have_contacts(device.kind) {
                        if ui.button(tr("Добавить доп. контакт")).clicked() {
                            *action.borrow_mut() = Some(Action::AddAux(device.id.clone()));
                            ui.close();
                        }
                        ui.separator();
                    }
                    let both = matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay);
                    if both {
                        if ui.button(tr("Найти контакты на листе")).clicked() {
                            *action.borrow_mut() =
                                Some(Action::Find(device.id.clone(), Place::Power));
                            ui.close();
                        }
                        if ui.button(tr("Найти катушку на листе")).clicked() {
                            *action.borrow_mut() =
                                Some(Action::Find(device.id.clone(), Place::Control));
                            ui.close();
                        }
                    } else if ui.button(tr("Найти на листе")).clicked() {
                        let place = match device.kind {
                            DeviceKind::Sensor | DeviceKind::Button => Place::Control,
                            _ => Place::Power,
                        };
                        *action.borrow_mut() = Some(Action::Find(device.id.clone(), place));
                        ui.close();
                    }
                    if ui.button(tr("Свойства…")).clicked() {
                        *action.borrow_mut() = Some(Action::Select(device.id.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(tr("Удалить из проекта")).clicked() {
                        *action.borrow_mut() = Some(Action::Remove(device.id.clone()));
                        ui.close();
                    }
                });
                // The coil of a contactor or a relay stands under it, then the auxiliary contacts.
                if matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay) {
                    coil_row(
                        ui,
                        device,
                        selected.as_deref() == Some(device.id.as_str()) && coil_selected,
                        &action,
                    );
                }
                for contact in panel.aux_of(&device.id) {
                    aux_row(ui, panel, layout, contact, &selected, report, &action);
                }
            }
        };
        group(
            ui,
            ("panel_group", title),
            tr(title),
            Level::Sub,
            tools,
            body,
        );
    }
    // Contacts whose device is gone are still listed, so that they can be removed.
    let orphans: Vec<&Device> = panel
        .devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Aux && (d.of.is_empty() || panel.device(&d.of).is_none()))
        .collect();
    if !orphans.is_empty() {
        group(
            ui,
            ("panel_group", "orphans"),
            tr("Контакты без катушки"),
            Level::Sub,
            |_| {},
            |ui| {
                for contact in &orphans {
                    aux_row(ui, panel, layout, contact, &selected, report, &action);
                }
            },
        );
    }

    if supply != panel.supply {
        if let Some(doc) = &mut app.doc {
            let _ = doc.edit_panel::<(), ()>(Some("supply".into()), |p| {
                p.supply = supply;
                Ok(())
            });
        }
    }
    apply(app, action.into_inner());
}

/// A new device of `kind`, not connected yet.
pub(super) fn new_device(id: String, kind: DeviceKind) -> Device {
    Device {
        id,
        kind,
        rating: String::new(),
        part: String::new(),
        bind: String::new(),
        load: String::new(),
        poles: None,
        of: String::new(),
        form: ContactForm::default(),
        pins: String::new(),
        from: String::new(),
        // Nothing feeds a new device until the user says where it is connected.
        free: !matches!(
            kind,
            DeviceKind::Feed | DeviceKind::Aux | DeviceKind::Sensor | DeviceKind::Button
        ),
        series: String::new(),
        via: String::new(),
        conductors: Vec::new(),
    }
}

/// Makes the device of a palette entry: a coil is named by the drawing sheet `number` it is put
/// on (`K3.1`, in the order the coils are put; without a sheet like any device), a contact of a
/// relay comes without its coil (`K_1`; the coil is chosen in its properties). Returns the
/// designation and the layout key the entry is put under (`K3.1/coil` for a coil).
fn new_entry(
    panel: &mut crab_core::Panel,
    payload: PalettePayload,
    number: Option<usize>,
) -> (String, String) {
    use crab_schematic::PalettePart::{Coil, Contact, Whole};
    let (id, kind, key) = match payload.part {
        Coil => {
            let id = number.map_or_else(
                || panel.next_id(payload.kind),
                |n| panel.next_sheet_id(payload.kind, n),
            );
            let key = crab_core::part_key(&id, crab_core::COIL);
            (id, payload.kind, key)
        }
        Contact => {
            let id = panel.next_aux_id(&DeviceKind::Relay.letter().to_string());
            (id.clone(), DeviceKind::Aux, id)
        }
        Whole => {
            let id = panel.next_id(payload.kind);
            (id.clone(), payload.kind, id)
        }
    };
    panel.devices.push(new_device(id.clone(), kind));
    (id, key)
}

/// Makes the device of a palette entry dropped on the drawing sheet `on` (document number
/// `number`, when known) at `(x, y)` and puts it there. Returns the designation.
pub(super) fn drop_new(
    project: &mut crab_core::Project,
    payload: PalettePayload,
    on: SheetId,
    number: Option<usize>,
    (x, y): (f32, f32),
) -> String {
    let (id, key) = new_entry(&mut project.panel, payload, number);
    project.panel_layout.place(&key, on, x, y);
    id
}

/// The palette: every kind of device with the picture of its symbol, in the groups of the tree.
/// An entry is dragged onto a sheet (the device is made there) or clicked (the device is made and
/// waits in the tree for its place).
fn palette(ui: &mut Ui, supply: crab_core::Supply, action: &std::cell::RefCell<Option<Action>>) {
    egui::CollapsingHeader::new(tr("Элементы"))
        .id_salt("panel_palette")
        .default_open(true)
        .show(ui, |ui| {
            for (title, kinds) in GROUPS {
                ui.weak(tr(title));
                for &kind in kinds {
                    let entries = palette_entries(kind);
                    // A device of parts: its name, the parts under it.
                    let nested = entries.len() > 1 && kind == DeviceKind::Relay;
                    if nested {
                        ui.label(kind_title(kind));
                    }
                    for (part, name) in entries {
                        ui.horizontal(|ui| {
                            if nested {
                                ui.add_space(12.0);
                            }
                            let payload = PalettePayload { kind, part };
                            let resp = ui
                                .dnd_drag_source(
                                    ui.make_persistent_id((
                                        "panel_palette",
                                        format!("{kind:?}"),
                                        part,
                                    )),
                                    payload,
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            preview_image(ui, kind, part, supply);
                                            ui.label(&name);
                                        });
                                    },
                                )
                                .response;
                            if resp.clicked() {
                                *action.borrow_mut() = Some(Action::Add(payload));
                            }
                            resp.on_hover_text(tr("Перетащите на лист"));
                        });
                    }
                }
            }
        });
}

/// The picture of a palette entry: its symbol, fitted into a small box.
fn preview_image(
    ui: &mut Ui,
    kind: DeviceKind,
    part: crab_schematic::PalettePart,
    supply: crab_core::Supply,
) {
    let size = Vec2::new(44.0, 30.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    thread_local! {
        // The pictures do not change while the program runs: made once per entry and supply.
        static CACHE: std::cell::RefCell<std::collections::HashMap<String, crab_schematic::Preview>> =
            Default::default();
    }
    let key = format!("{kind:?}/{part:?}/{supply:?}");
    let pv = CACHE.with(|c| {
        c.borrow_mut()
            .entry(key)
            .or_insert_with(|| crab_schematic::preview(kind, part, supply))
            .clone()
    });
    let (w, h) = (pv.max.0 - pv.min.0, pv.max.1 - pv.min.1);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let scale = ((size.x - 4.0) / w).min((size.y - 4.0) / h);
    let used = Vec2::new(w, h) * scale;
    let origin = rect.min + (size - used) / 2.0 - Vec2::new(pv.min.0, pv.min.1) * scale;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::WHITE);
    draw_ops(&painter, &View { origin, scale }, &pv.ops);
}

/// Whether `id` is a designation with a sheet number, `K17.1`: a letter, the sheet, a point, the
/// number.
pub(super) fn is_sheet_designation(id: &str) -> bool {
    let mut chars = id.chars();
    let letter = chars.next().is_some_and(|c| c.is_ascii_uppercase());
    let rest = chars.as_str();
    letter
        && rest.split_once('.').is_some_and(|(sheet, n)| {
            [sheet, n]
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// The designation a coil is offered when it is put on the drawing sheet `sheet` (document number):
/// a coil named by its sheet (`K3.1`) that is on another sheet gets the next free one of `sheet`
/// (`K5.1`). `None`: the designation is not named by a sheet (`K_1`, `K3`), or the coil stays on
/// its sheet.
pub(super) fn sheet_name_offer(
    id: &str,
    kind: DeviceKind,
    panel: &crab_core::Panel,
    sheet: usize,
) -> Option<String> {
    if !is_sheet_designation(id) {
        return None;
    }
    let (own, _) = id.get(1..)?.split_once('.')?;
    if own.parse::<usize>().ok()? == sheet {
        return None;
    }
    Some(panel.next_sheet_id(kind, sheet))
}

pub(super) fn apply(app: &mut EditorApp, action: Option<Action>) {
    let Some(action) = action else {
        return;
    };
    match action {
        Action::SelectCoil(id) => {
            app.panel_view.selected = Some(id);
            app.panel_view.coil = true;
            app.panel_view.label = None;
            app.panel_view.rename = None;
            app.panel_view.problem = None;
            app.right_open_for_panel();
        }
        Action::Select(id) => {
            app.panel_view.coil = false;
            app.panel_view.selected = Some(id);
            app.panel_view.rename = None;
            app.panel_view.problem = None;
            app.right_open_for_panel();
        }
        Action::Add(payload) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            let id = doc
                .edit_panel::<_, ()>(None, |p| Ok(new_entry(p, payload, None).0))
                .unwrap_or_default();
            app.panel_view.coil = payload.part == crab_schematic::PalettePart::Coil;
            app.panel_view.selected = Some(id);
            app.panel_view.rename = None;
            app.panel_view.problem = None;
            app.right_open_for_panel();
        }
        Action::AddThermistor(of) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            let id = doc
                .edit_panel::<_, ()>(None, |p| {
                    let id = p.next_id(DeviceKind::Sensor);
                    p.devices.push(Device {
                        id: id.clone(),
                        kind: DeviceKind::Sensor,
                        via: String::new(),
                        rating: String::new(),
                        part: String::new(),
                        bind: String::new(),
                        load: String::new(),
                        poles: None,
                        of,
                        form: ContactForm::default(),
                        pins: String::new(),
                        from: String::new(),
                        free: false,
                        series: String::new(),
                        conductors: Vec::new(),
                    });
                    Ok(id)
                })
                .unwrap_or_default();
            app.panel_view.selected = Some(id);
            app.panel_view.rename = None;
            app.panel_view.problem = None;
            app.right_open_for_panel();
        }
        Action::AddAux(of) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            let id = doc
                .edit_panel::<_, ()>(None, |p| {
                    let id = p.next_aux_id(&of);
                    p.devices.push(Device {
                        id: id.clone(),
                        kind: DeviceKind::Aux,
                        via: String::new(),
                        rating: String::new(),
                        part: String::new(),
                        bind: String::new(),
                        load: String::new(),
                        poles: None,
                        of,
                        form: ContactForm::default(),
                        pins: String::new(),
                        from: String::new(),
                        free: false,
                        series: String::new(),
                        conductors: Vec::new(),
                    });
                    Ok(id)
                })
                .unwrap_or_default();
            app.panel_view.selected = Some(id);
            app.panel_view.rename = None;
            app.panel_view.problem = None;
            app.right_open_for_panel();
        }
        Action::Remove(id) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                p.remove(&id);
                Ok(())
            });
            if app.panel_view.selected.as_deref() == Some(id.as_str()) {
                app.panel_view.selected = None;
            }
        }
        Action::SheetNames => {
            refresh(app);
            let Some(Ok(document)) = app.panel_view.cache.as_ref().map(|c| c.document.as_ref())
            else {
                return;
            };
            let Some(doc) = &mut app.doc else {
                return;
            };
            let names = crab_schematic::sheet_designations(document, &doc.project.panel);
            match doc.edit_panel_project(None, |p| crab_core::rename_devices(p, &names)) {
                Ok(()) => {
                    app.panel_view.selected = None;
                    app.panel_view.problem = None;
                }
                Err(message) => app.panel_view.problem = Some(message),
            }
        }
        Action::Find(id, place) => {
            app.panel_view.selected = Some(id.clone());
            find(app, &id, place);
        }
    }
}

/// Shows the drawing of the device: the sheet, a mark on its zone, the view brought to it.
pub(super) fn find(app: &mut EditorApp, id: &str, place: Place) {
    refresh(app);
    let Some(Ok(document)) = app.panel_view.cache.as_ref().map(|c| c.document.as_ref()) else {
        return;
    };
    let at = match place {
        Place::Power => document.power_at.get(id),
        Place::Control => document.coil_at.get(id),
    };
    let Some(at) = at else {
        return;
    };
    let (sheet, zone) = (at.sheet.saturating_sub(1), at.zone.clone());
    let centre = document
        .zone_rect(&zone)
        .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0));
    app.panel_view.sheet = sheet;
    app.panel_view.highlight = Some((sheet, zone));
    app.panel_view.focus = centre;
}
