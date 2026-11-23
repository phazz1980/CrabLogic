//! Mode «Щит»: the electrical panel of the project (`specs/electrical-schematics`, task 9).
//!
//! Left the tree of the devices, in the middle the preview of the sheets of the documentation,
//! right the properties of the selected device and the problems of the panel. The sheets are the
//! same drawing that goes into the PDF (`crab_schematic::Op`), shown without making a PDF.

use std::path::PathBuf;

use crab_core::{
    bus_key, check_panel, panel_binding, panel_tags, symbol_key, ContactForm, Device, DeviceKind,
    Report, Severity, SheetId,
};
use crab_schematic::{
    build_document_in, device_kind_name, Align, Document, Format, Item, ItemKind, Lang, Op, Rgb,
    Sheet,
};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};

use crate::app::EditorApp;
use crate::i18n::tr;
use crate::panels::{group, row_label, Level};

/// Mode of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    /// The schema of the controller.
    #[default]
    Logic,
    /// The electrical panel.
    Panel,
}

/// Groups of the device tree: the title and the kinds in it.
const GROUPS: [(&str, &[DeviceKind]); 6] = [
    ("Питание", &[DeviceKind::Feed, DeviceKind::Psu]),
    (
        "Защита",
        &[
            DeviceKind::Breaker,
            DeviceKind::Rcd,
            DeviceKind::Fuse,
            DeviceKind::Switch,
        ],
    ),
    ("Коммутация", &[DeviceKind::Contactor, DeviceKind::Relay]),
    ("Нагрузки", &[DeviceKind::Motor, DeviceKind::Load]),
    ("Клеммы", &[DeviceKind::Terminal]),
    (
        "Датчики и кнопки",
        &[DeviceKind::Sensor, DeviceKind::Button],
    ),
];

/// What the preview was made for; a change of any part makes it again.
#[derive(PartialEq)]
struct Key {
    path: PathBuf,
    revision: u64,
    panel_revision: u64,
    lang: Lang,
    format: Format,
}

struct Cache {
    key: Key,
    /// The sheets, or why they could not be made.
    document: Result<Document, String>,
    report: Report,
}

/// What the mouse is dragging on a sheet.
#[derive(Debug, Clone, PartialEq)]
enum Drag {
    /// A symbol: its layout key, where the pointer was on the page and the shift it had.
    Symbol {
        key: String,
        start: (f32, f32),
        shift: (f32, f32),
    },
    /// The middle segment of a wire: the wire key.
    Bend { key: String },
    /// A wire being drawn from a terminal: its device and terminal, and where it starts.
    Link {
        from: (String, String),
        at: (f32, f32),
    },
    /// The end of a wire moved to another terminal (with Ctrl): the wire key, which end, where
    /// it was, and the terminals it may be dropped on (`DEV:terminal` and their places).
    Rewire {
        key: String,
        end: crab_core::WireEnd,
        at: (f32, f32),
        targets: Vec<(String, (f32, f32))>,
    },
}

/// A device dragged from the tree: dropped on a control sheet it is put there.
struct DevicePayload(String);

/// A change of the manual layout made by the mouse; applied when the sheet is drawn.
#[derive(Debug, Clone, PartialEq)]
enum LayoutEdit {
    Shift { key: String, dx: f32, dy: f32 },
    Bend { key: String, y: f32 },
    ResetBend { key: String },
    BusTo { index: usize, to: Option<usize> },
    End,
}

/// State of the mode «Щит».
pub struct PanelView {
    /// Designation of the selected device.
    pub selected: Option<String>,
    /// Number of the sheet shown (from 0).
    sheet: usize,
    /// The project the sheet was last chosen for: a project that has just been opened starts at its
    /// first power sheet, not at the title sheet.
    started_for: Option<std::path::PathBuf>,
    /// Language of the sheets; `None`: the language of the editor.
    lang: Option<Lang>,
    format: Format,
    cache: Option<Cache>,
    /// Zone to mark on a sheet: the sheet and the zone, set by «Найти на листе».
    highlight: Option<(usize, String)>,
    /// A point of the page to bring to the middle of the view at the next frame.
    focus: Option<(f32, f32)>,
    /// Magnification of the preview (1 fits the page into the window) and the shift.
    zoom: f32,
    pan: Vec2,
    /// The designation being typed and the one it is for.
    rename: Option<(String, String)>,
    /// Why the last rename was refused.
    problem: Option<String>,
    /// The PDF being made in the background.
    pdf_job: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// The result of the last «Собрать PDF»: the file or the reason.
    pdf_result: Option<Result<std::path::PathBuf, String>>,
    /// What the mouse is dragging on the sheet.
    drag: Option<Drag>,
    /// The end reference of a supply line the menu is open for: the power sheet (from 0) and the
    /// sheet it is carried on to now.
    bus_edit: Option<(usize, usize)>,
    /// The text picked on the sheet: its layout key and the size of its font as drawn.
    label: Option<(String, f32)>,
    /// The properties shown are those of the coil of the selected contactor or relay.
    coil: bool,
    /// The wire (its key) the right-click menu is open for.
    link_menu: Option<String>,
    /// The device put on a sheet by hand that the right-click menu is open for.
    unplace_menu: Option<String>,
    /// Show this sheet at the next frame (a device has just been put on it, or it was just added).
    goto: Option<SheetId>,
    /// The caption of the shown sheet being typed: the sheet and the text.
    caption_edit: Option<(SheetId, String)>,
    /// «Разложить заново всё» was pressed once and waits for the confirmation.
    confirm_reset: bool,
}

impl Default for PanelView {
    fn default() -> Self {
        Self {
            selected: None,
            sheet: 0,
            started_for: None,
            coil: false,
            link_menu: None,
            unplace_menu: None,
            goto: None,
            caption_edit: None,
            confirm_reset: false,
            lang: None,
            format: Format::default(),
            cache: None,
            highlight: None,
            focus: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            rename: None,
            problem: None,
            pdf_job: None,
            pdf_result: None,
            drag: None,
            label: None,
            bus_edit: None,
        }
    }
}

impl PanelView {
    /// The language of the sheets: chosen, or that of the editor (Russian or else English).
    fn language(&self) -> Lang {
        self.lang.unwrap_or_else(editor_lang)
    }
}

/// Language of the documentation that goes with the language of the editor.
fn editor_lang() -> Lang {
    if crate::i18n::current().code() == "ru" {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// Name of a kind of device in the language of the editor.
pub fn kind_title(kind: DeviceKind) -> String {
    tr(device_kind_name(kind, Lang::Ru)).to_owned()
}

/// Makes the preview again if the project, the panel, the language or the format changed.
fn refresh(app: &mut EditorApp) {
    let Some(doc) = &app.doc else {
        app.panel_view.cache = None;
        return;
    };
    let view = &app.panel_view;
    let key = Key {
        path: doc.project.path.clone(),
        revision: doc.revision,
        panel_revision: doc.panel_revision,
        lang: view.language(),
        format: view.format,
    };
    if view.cache.as_ref().is_some_and(|c| c.key == key) {
        return;
    }
    let report = check_panel(&doc.project, &app.lib);
    // The panel is edited step by step, so unfinished panels are drawn too; a drawing that
    // fails must not take the editor with it.
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build_document_in(&doc.project, &app.lib, key.lang, key.format)
    }));
    let document = match built {
        Ok(Ok((document, _))) => Ok(document),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(tr("Не удалось построить листы").to_owned()),
    };
    app.panel_view.cache = Some(Cache {
        key,
        document,
        report,
    });
    let last = app
        .panel_view
        .cache
        .as_ref()
        .and_then(|c| c.document.as_ref().ok())
        .map_or(0, |d| d.sheets.len().saturating_sub(1));
    app.panel_view.sheet = app.panel_view.sheet.min(last);
    if let Some(id) = app.panel_view.goto {
        let found = app
            .panel_view
            .cache
            .as_ref()
            .and_then(|c| c.document.as_ref().ok())
            .and_then(|d| d.sheets.iter().position(|s| s.id == Some(id)));
        if let Some(found) = found {
            app.panel_view.sheet = found;
            app.panel_view.goto = None;
        }
    }
    let path = app.doc.as_ref().map(|d| d.project.path.clone());
    if last >= 1 && app.panel_view.started_for != path {
        app.panel_view.sheet = 1;
        app.panel_view.started_for = path;
    }
}

/// The tabs «Логика» / «Щит» above the work area.
pub fn tabs(app: &mut EditorApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.mode, AppMode::Logic, tr("Логика"));
        ui.selectable_value(&mut app.mode, AppMode::Panel, tr("Щит"))
            .on_hover_text(tr(
                "Электрический щит: устройства, схемы силовой части и цепей управления, перечни",
            ));
    });
}

/// Whether a diagnostic of the panel speaks of the device (`-K1` or «K1»).
fn is_about(message: &str, id: &str) -> bool {
    let dash = format!("-{id}");
    let quoted = format!("«{id}»");
    message.contains(&quoted)
        || message.match_indices(&dash).any(|(at, _)| {
            let before = message[..at].chars().next_back();
            let after = message[at + dash.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
}

enum Action {
    Select(String),
    /// The coil of a contactor or a relay.
    SelectCoil(String),
    Add(DeviceKind),
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
enum Place {
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

    for (title, kinds) in GROUPS {
        let tools = |ui: &mut Ui| {
            ui.menu_button("+", |ui| {
                for &kind in kinds {
                    if ui.button(kind_title(kind)).clicked() {
                        *action.borrow_mut() = Some(Action::Add(kind));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(tr("Добавить устройство"));
        };
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
                    aux_row(ui, panel, contact, &selected, report, &action);
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
        .filter(|d| d.kind == DeviceKind::Aux && panel.device(&d.of).is_none())
        .collect();
    if !orphans.is_empty() {
        group(
            ui,
            ("panel_group", "orphans"),
            tr("Доп. контакты без аппарата"),
            Level::Sub,
            |_| {},
            |ui| {
                for contact in &orphans {
                    aux_row(ui, panel, contact, &selected, report, &action);
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

/// Whether `id` is a designation with a sheet number, `K17.1`: a letter, the sheet, a point, the
/// number.
fn is_sheet_designation(id: &str) -> bool {
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

/// The row of the coil of a contactor or a relay in the tree: the output of the board it is on.
fn coil_row(
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
            ui.label("A1");
            ui.label(a1);
            ui.end_row();
            ui.label("A2");
            ui.label(a2);
            ui.end_row();
        });
    ui.weak(tr(
        "Выводы A1 и A2 подключены автоматически. Чтобы подключить иначе, протяните провод от вывода к выводу на листе цепей управления.",
    ));
    ui.add_space(6.0);
    if ui.button(tr("Найти катушку на листе")).clicked() {
        apply(app, Some(Action::Find(id.to_owned(), Place::Control)));
    }
    apply_edit(app, id, edit);
}

/// Puts an auxiliary contact in series with the coil `series` (empty: on an input of the board).
/// The circuit is one thing, so the input is let go when a coil is chosen.
fn set_series(contact: &mut Device, series: &str) {
    contact.series = series.to_owned();
    if !series.is_empty() {
        contact.bind.clear();
    }
}

/// Kinds that have auxiliary contacts.
fn can_have_contacts(kind: DeviceKind) -> bool {
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
    }
    .to_owned()
}

/// The row of an auxiliary contact in the tree: the device, the number, the form and where it
/// is switched.
fn aux_row(
    ui: &mut Ui,
    panel: &crab_core::Panel,
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
    let circuit = if !contact.series.is_empty() {
        format!("→ -{}", contact.series)
    } else if !contact.bind.is_empty() {
        format!("→ {}", contact.bind)
    } else {
        String::new()
    };
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
        let resp = ui.selectable_label(
            selected.as_deref() == Some(contact.id.as_str()),
            row_label(ui, caption, warn, None),
        );
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

fn supply_name(supply: crab_core::Supply) -> &'static str {
    match supply {
        crab_core::Supply::Dc24 => "24 В постоянного тока",
        crab_core::Supply::Ac230 => "230 В, 1 фаза",
        crab_core::Supply::Ac400 => "400 В, 3 фазы",
    }
}

fn apply(app: &mut EditorApp, action: Option<Action>) {
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
        Action::Add(kind) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            let id = doc
                .edit_panel::<_, ()>(None, |p| {
                    let id = p.next_id(kind);
                    p.devices.push(Device {
                        id: id.clone(),
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
                            DeviceKind::Feed
                                | DeviceKind::Aux
                                | DeviceKind::Sensor
                                | DeviceKind::Button
                        ),
                        series: String::new(),
                    });
                    Ok(id)
                })
                .unwrap_or_default();
            app.panel_view.coil = false;
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
fn find(app: &mut EditorApp, id: &str, place: Place) {
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
    let others: Vec<String> = doc
        .project
        .panel
        .devices
        .iter()
        .filter(|d| d.id != id)
        .map(|d| d.id.clone())
        .collect();
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
            ) {
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

            if is_aux {
                ui.label(tr("Аппарат"));
                ui.label(format!("-{}", device.of));
                ui.end_row();

                ui.label(tr("Тип контакта"));
                let mut form = device.form;
                egui::ComboBox::from_id_salt("panel_form")
                    .selected_text(form_title(form))
                    .show_ui(ui, |ui| {
                        for f in [ContactForm::No, ContactForm::Nc] {
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
                let (near, far) = crab_schematic::nets::aux_terminals(&doc.project.panel, &device);
                ui.add(
                    egui::TextEdit::singleline(&mut pins)
                        .hint_text(format!("{near}-{far}"))
                        .desired_width(80.0),
                )
                .on_hover_text(tr(
                    "Номера выводов через дефис, например 21-22; пусто — по номеру контакта",
                ));
                if pins != device.pins {
                    edit.pins = Some(pins);
                }
                ui.end_row();

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
            if !is_aux {
                let feeder = doc
                    .project
                    .panel
                    .devices
                    .iter()
                    .find(|d| d.load == id)
                    .map(|d| d.id.clone())
                    .unwrap_or_default();
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
                                && matches!(
                                    d.kind,
                                    DeviceKind::Breaker
                                        | DeviceKind::Rcd
                                        | DeviceKind::Fuse
                                        | DeviceKind::Switch
                                        | DeviceKind::Psu
                                        | DeviceKind::Contactor
                                        | DeviceKind::Relay
                                        | DeviceKind::Terminal
                                )
                        }) {
                            ui.selectable_value(&mut chosen, d.id.clone(), format!("-{}", d.id));
                        }
                    });
                if chosen != shown {
                    edit.feeder = Some(chosen);
                }
                ui.end_row();

                // A second source: the device hangs on a branch of another device's line, which
                // keeps what it feeds; the junction is drawn on the sheet by itself.
                ui.label(tr("Ответвление от"));
                let mut branch = device.from.clone();
                egui::ComboBox::from_id_salt("panel_branch")
                    .selected_text(if branch.is_empty() {
                        tr("нет").to_owned()
                    } else {
                        format!("-{branch}")
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut branch, String::new(), tr("нет"));
                        for d in doc.project.panel.devices.iter().filter(|d| {
                            d.id != id
                                && matches!(
                                    d.kind,
                                    DeviceKind::Breaker
                                        | DeviceKind::Rcd
                                        | DeviceKind::Fuse
                                        | DeviceKind::Switch
                                        | DeviceKind::Psu
                                        | DeviceKind::Contactor
                                        | DeviceKind::Relay
                                        | DeviceKind::Terminal
                                )
                        }) {
                            ui.selectable_value(&mut branch, d.id.clone(), format!("-{}", d.id));
                        }
                    });
                if branch != device.from {
                    edit.branch = Some(branch);
                }
                ui.end_row();

                // A motor and a load other than a device have nothing behind them.
                if !matches!(device.kind, DeviceKind::Motor | DeviceKind::Load) {
                    ui.label(tr("Нагрузка (питает или коммутирует)"));
                    let mut load = device.load.clone();
                    egui::ComboBox::from_id_salt("panel_load")
                        .selected_text(if load.is_empty() {
                            tr("ничего").to_owned()
                        } else {
                            format!("-{load}")
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut load, String::new(), tr("ничего"));
                            for other in &others {
                                ui.selectable_value(&mut load, other.clone(), format!("-{other}"));
                            }
                        });
                    if load != device.load {
                        edit.load = Some(load);
                    }
                    ui.end_row();
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
    load: Option<String>,
    /// The device that feeds this one (empty: straight from the feed).
    feeder: Option<String>,
    /// The device whose line this one branches from (empty: none).
    branch: Option<String>,
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
    remove: bool,
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
        field(doc, "poles", &|d| d.poles = poles);
    }
    if let Some(part) = edit.part {
        field(doc, "part", &|d| d.part = part.clone());
    }
    if let Some(bind) = edit.bind {
        field(doc, "bind", &|d| d.bind = bind.clone());
    }
    if let Some(load) = edit.load {
        field(doc, "load", &|d| d.load = load.clone());
    }
    if let Some(parent) = edit.feeder {
        let parent = (!parent.is_empty()).then_some(parent);
        match doc.edit_panel(None, |p| p.reconnect_power(id, parent.as_deref())) {
            Ok(()) => {
                // The device goes under its new place by itself.
                doc.edit_panel_layout(None, |l| l.clear_symbol(&symbol_key("power", id)));
                app.panel_view.problem = None;
            }
            Err(message) => app.panel_view.problem = Some(message),
        }
    }
    if let Some(source) = edit.branch {
        let source = (!source.is_empty()).then_some(source);
        match doc.edit_panel(None, |p| p.branch_power(id, source.as_deref())) {
            Ok(()) => {
                doc.edit_panel_layout(None, |l| l.clear_symbol(&symbol_key("power", id)));
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

fn color(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.0, rgb.1, rgb.2)
}

/// Where the page is on the screen: the corner and the scale (points of the screen per unit).
struct View {
    origin: Pos2,
    scale: f32,
}

impl View {
    fn at(&self, (x, y): (f32, f32)) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.scale
    }
}

/// The rectangle an operation covers, in the units of the page. Text is measured roughly: the
/// width of a character is half its size.
fn op_bounds(op: &Op) -> Option<Rect> {
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    };
    match op {
        Op::Line { points, .. } => {
            let pts: Vec<Pos2> = points.iter().map(|&(x, y)| Pos2::new(x, y)).collect();
            (!pts.is_empty()).then(|| Rect::from_points(&pts))
        }
        Op::Rect { x, y, w, h, .. } => Some(rect(*x, *y, x + w, y + h)),
        Op::Circle { x, y, r, .. } | Op::Dot { x, y, r, .. } => {
            Some(rect(x - r, y - r, x + r, y + r))
        }
        Op::Text {
            x,
            y,
            text,
            size,
            align,
            vertical,
            ..
        } => {
            let w = text.chars().count() as f32 * size * 0.5;
            let (a, b) = match align {
                Align::Left => (0.0, w),
                Align::Center => (-w / 2.0, w / 2.0),
                Align::Right => (-w, 0.0),
            };
            Some(if *vertical {
                // Turned counter-clockwise: it runs up from `y`.
                rect(x - size / 2.0, y - b, x + size / 2.0, y - a)
            } else {
                rect(x + a, y - size / 2.0, x + b, y + size / 2.0)
            })
        }
    }
}

/// The reference (`/3.C4`) written at the point `p` of the sheet: the sheet it names (`None`: the
/// one it is on) and the zone.
fn ref_at(sheet: &Sheet, p: Pos2) -> Option<(Option<usize>, String)> {
    sheet.ops.iter().rev().find_map(|op| match op {
        Op::Text { text, .. } if op_bounds(op).is_some_and(|r| r.contains(p)) => {
            crab_schematic::find_ref(text)
        }
        _ => None,
    })
}

/// The rectangle of all the operations of an item.
fn item_bounds(sheet: &Sheet, item: &Item) -> Option<Rect> {
    item.ops
        .iter()
        .filter_map(|&i| sheet.ops.get(i).and_then(op_bounds))
        .reduce(Rect::union)
}

/// The symbol under a point of the page, the topmost one.
fn symbol_at(sheet: &Sheet, p: Pos2) -> Option<&Item> {
    sheet.items.iter().rev().find(|item| {
        matches!(item.kind, ItemKind::Symbol { .. })
            && item_bounds(sheet, item).is_some_and(|r| r.expand(4.0).contains(p))
    })
}

/// The numbers of the first and the last power sheet of the document (from 1).
fn power_span(document: &Document) -> (usize, usize) {
    let power: Vec<usize> = document
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.id, Some(crab_core::SheetId::Power(_))))
        .map(|(i, _)| i + 1)
        .collect();
    (
        power.first().copied().unwrap_or(1),
        power.last().copied().unwrap_or(1),
    )
}

/// The end reference of a supply line under a point: the power sheet it is on and the sheet it
/// goes on to (the reference right of the line, not the one at its start).
fn bus_end_at(sheet: &Sheet, p: Pos2, page_w: f32) -> Option<(usize, usize)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Bus { index, .. } = &item.kind else {
            return None;
        };
        item.ops.iter().find_map(|&o| {
            let op = sheet.ops.get(o)?;
            let Op::Text { text, x, .. } = op else {
                return None;
            };
            let (Some(target), _) = crab_schematic::find_ref(text)? else {
                return None;
            };
            (*x > page_w / 2.0 && op_bounds(op)?.expand(3.0).contains(p))
                .then_some((*index, target))
        })
    })
}

/// The terminal of a control sheet under a point of the page: the device, the terminal and where
/// it is.
fn port_at(
    sheet: &Sheet,
    panel: &crab_core::Panel,
    p: Pos2,
    tolerance: f32,
) -> Option<(String, String, (f32, f32))> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Symbol {
            view,
            device,
            ports,
            ..
        } = &item.kind
        else {
            return None;
        };
        if *view != "control" && *view != "board" {
            return None;
        }
        ports
            .iter()
            .find(|pt| (pt.x - p.x).hypot(pt.y - p.y) <= tolerance)
            .map(|pt| {
                let (owner, terminal) = port_terminal(panel, view, device, &pt.name);
                (owner, terminal, (pt.x, pt.y))
            })
    })
}

/// The device and the terminal a port of a symbol is wired as. A part of a device (`K1/coil`,
/// the channel `A1/IO5` of the board) is wired as the device: the ends of a wire are `K1:A1`,
/// `A1:IO5`; a port of a control symbol is named by the terminal of the device.
fn port_terminal(
    panel: &crab_core::Panel,
    view: &str,
    device: &str,
    port: &str,
) -> (String, String) {
    let owner = crab_core::split_part(device).0;
    let terminal = match (view, panel.device(owner)) {
        ("control", Some(d)) => crab_schematic::control::terminal_of_port(panel, d, port),
        _ => port.to_owned(),
    };
    (owner.to_owned(), terminal)
}

/// The end of a wire under a point of the page, the nearest one: the wire key, which end it is
/// and where. Which end a point of the wire is comes from the symbol whose port it touches (a
/// signal wire is drawn from the device to the board, though its key starts at the board); a
/// point on no port counts by the drawing order: the first point is the `from` end.
fn wire_end_at(
    sheet: &Sheet,
    p: Pos2,
    tolerance: f32,
) -> Option<(String, crab_core::WireEnd, (f32, f32))> {
    // A wire ends a dot (2.5) away from its port: the nearest port within a little more.
    let owner_at = |q: (f32, f32)| {
        sheet
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::Symbol { device, ports, .. } => Some((device, ports)),
                _ => None,
            })
            .flat_map(|(device, ports)| {
                ports
                    .iter()
                    .map(move |pt| ((pt.x - q.0).hypot(pt.y - q.1), device))
            })
            .filter(|(d, _)| *d <= 3.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, device)| crab_core::split_part(device).0)
    };
    let mut best: Option<(f32, String, crab_core::WireEnd, (f32, f32))> = None;
    for item in &sheet.items {
        let ItemKind::Wire { key, from, to } = &item.kind else {
            continue;
        };
        let lines: Vec<&Vec<(f32, f32)>> = item
            .ops
            .iter()
            .filter_map(|&o| match sheet.ops.get(o) {
                Some(Op::Line { points, .. }) => Some(points),
                _ => None,
            })
            .collect();
        let (Some(&first), Some(&last)) = (
            lines.first().and_then(|l| l.first()),
            lines.last().and_then(|l| l.last()),
        ) else {
            continue;
        };
        for (q, by_order) in [
            (first, crab_core::WireEnd::From),
            (last, crab_core::WireEnd::To),
        ] {
            let d = (q.0 - p.x).hypot(q.1 - p.y);
            if d > tolerance || best.as_ref().is_some_and(|b| b.0 <= d) {
                continue;
            }
            let end = match owner_at(q) {
                Some(owner) if owner == to.device && owner != from.device => crab_core::WireEnd::To,
                Some(owner) if owner == from.device && owner != to.device => {
                    crab_core::WireEnd::From
                }
                _ => by_order,
            };
            best = Some((d, key.clone(), end, q));
        }
    }
    best.map(|(_, key, end, q)| (key, end, q))
}

/// The terminal (`DEV:terminal`) of any symbol of the sheet under a point of the page.
fn terminal_at(sheet: &Sheet, panel: &crab_core::Panel, p: Pos2, tolerance: f32) -> Option<String> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Symbol {
            view,
            device,
            ports,
            ..
        } = &item.kind
        else {
            return None;
        };
        ports
            .iter()
            .find(|pt| (pt.x - p.x).hypot(pt.y - p.y) <= tolerance)
            .map(|pt| {
                let (owner, terminal) = port_terminal(panel, view, device, &pt.name);
                format!("{owner}:{terminal}")
            })
    })
}

/// Where the end `end` of the wire `key` can be dropped: each terminal of the sheet the panel
/// accepts it on (`DEV:terminal`), with the place to mark. Each is tried on a copy of the project.
fn rewire_targets(
    sheet: &Sheet,
    project: &crab_core::Project,
    lib: &crab_core::Library,
    key: &str,
    end: crab_core::WireEnd,
) -> Vec<(String, (f32, f32))> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in &sheet.items {
        let ItemKind::Symbol {
            view,
            device,
            ports,
            ..
        } = &item.kind
        else {
            continue;
        };
        for pt in ports {
            let (owner, terminal) = port_terminal(&project.panel, view, device, &pt.name);
            let target = format!("{owner}:{terminal}");
            if !seen.insert(target.clone()) {
                continue;
            }
            let mut copy = project.clone();
            if crab_core::rewire(&mut copy, lib, key, end, &target).is_ok() {
                out.push((target, (pt.x, pt.y)));
            }
        }
    }
    out
}

/// Why a device (or a part of one, `K1/coil`) dragged from the tree cannot be put on a sheet: the
/// generator already draws its coil there (the device is bound to an output of the board), unless
/// the coil was put on a sheet by hand and is drawn as a symbol of its own. `None`: it can.
fn drop_refusal(
    sheets: &[Sheet],
    coil_at: &crab_schematic::Xrefs,
    key: &str,
) -> Option<&'static str> {
    let (owner, part) = crab_core::split_part(key);
    let hand_coil = crab_core::part_key(owner, crab_core::COIL);
    let coil_by_hand = sheets.iter().any(|s| {
        s.items
            .iter()
            .any(|i| i.device() == Some(hand_coil.as_str()))
    });
    if !coil_at.contains_key(owner) || coil_by_hand {
        return None;
    }
    match part {
        Some(crab_core::COIL) => {
            Some("Катушка уже нарисована на листе управления: устройство привязано к выходу платы")
        }
        None => Some("Это устройство уже нарисовано на листе управления"),
        Some(_) => None,
    }
}

/// The wire drawn by hand under a point of the page: its key.
fn link_at(sheet: &Sheet, panel: &crab_core::Panel, p: Pos2, tolerance: f32) -> Option<String> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Wire { key, .. } = &item.kind else {
            return None;
        };
        panel.links.iter().find(|l| l.key() == *key)?;
        let near = item.ops.iter().any(|&o| {
            let Some(Op::Line { points, .. }) = sheet.ops.get(o) else {
                return false;
            };
            points.windows(2).any(|w| {
                let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                p.x >= x0.min(x1) - tolerance
                    && p.x <= x0.max(x1) + tolerance
                    && p.y >= y0.min(y1) - tolerance
                    && p.y <= y0.max(y1) + tolerance
            })
        });
        near.then(|| key.clone())
    })
}

/// The supply line whose horizontal part is under a point of the page: its layout key.
fn bus_at(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<String> {
    bus_hit(sheet, p, tolerance).map(|(index, k)| bus_key(index, k))
}

/// The supply line whose horizontal part is under a point of the page: the power sheet (from 0)
/// and the number of the line.
fn bus_hit(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<(usize, usize)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Bus { index, k } = &item.kind else {
            return None;
        };
        let hit = item.ops.iter().any(|&o| {
            let Some(Op::Line { points, .. }) = sheet.ops.get(o) else {
                return false;
            };
            points.windows(2).any(|w| {
                let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                y0 == y1
                    && (p.y - y0).abs() <= tolerance
                    && p.x >= x0.min(x1) - tolerance
                    && p.x <= x0.max(x1) + tolerance
            })
        });
        hit.then_some((*index, *k))
    })
}

/// The text that can be moved under a point of the page: its layout key.
fn label_at(sheet: &Sheet, p: Pos2) -> Option<&str> {
    sheet.items.iter().rev().find_map(|item| {
        let ItemKind::Label { key } = &item.kind else {
            return None;
        };
        item_bounds(sheet, item)
            .is_some_and(|r| r.expand(3.0).contains(p))
            .then_some(key.as_str())
    })
}

/// The device and the layout key of a symbol that can be moved by hand (not the board).
fn movable(item: &Item) -> Option<(&str, String)> {
    match &item.kind {
        ItemKind::Symbol { view, device, .. } if *view != "board" => {
            Some((device, symbol_key(view, device)))
        }
        _ => None,
    }
}

/// The wire whose middle segment is under a point of the page: its key and the height of the
/// segment. Only a wire routed in three segments has one.
fn bend_at(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<(&str, f32)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Wire { key, .. } = &item.kind else {
            return None;
        };
        let points = item.ops.iter().find_map(|&i| match sheet.ops.get(i)? {
            Op::Line { points, .. } if points.len() == 4 => Some(points),
            _ => None,
        })?;
        let (a, b) = (points[1], points[2]);
        let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
        ((p.y - a.1).abs() <= tolerance && p.x >= lo - tolerance && p.x <= hi + tolerance)
            .then_some((key.as_str(), a.1))
    })
}

/// Draws the operations of a sheet.
fn draw_ops(painter: &egui::Painter, view: &View, ops: &[Op]) {
    // The font of the editor is a little taller than the drawing font for the same size.
    const FONT: f32 = 0.85;
    let width = |w: f32| (w * view.scale).max(0.6);
    for op in ops {
        match op {
            Op::Line {
                points,
                width: w,
                dashed,
                rgb,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|&p| view.at(p)).collect();
                let stroke = Stroke::new(width(*w), color(*rgb));
                if *dashed {
                    painter.extend(egui::Shape::dashed_line(
                        &pts,
                        stroke,
                        8.0 * view.scale,
                        5.0 * view.scale,
                    ));
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Op::Rect {
                x,
                y,
                w,
                h,
                width: lw,
                rgb,
            } => {
                let rect = Rect::from_min_size(view.at((*x, *y)), Vec2::new(*w, *h) * view.scale);
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(width(*lw), color(*rgb)),
                    egui::StrokeKind::Middle,
                );
            }
            Op::Circle {
                x,
                y,
                r,
                width: lw,
                rgb,
            } => {
                painter.circle_stroke(
                    view.at((*x, *y)),
                    r * view.scale,
                    Stroke::new(width(*lw), color(*rgb)),
                );
            }
            Op::Dot { x, y, r, rgb } => {
                painter.circle_filled(view.at((*x, *y)), r * view.scale, color(*rgb));
            }
            Op::Text {
                x,
                y,
                text,
                size,
                align,
                bold,
                vertical,
                rgb,
            } => {
                let font = egui::FontId::proportional(size * view.scale * FONT);
                if font.size < 3.0 {
                    continue;
                }
                let at = view.at((*x, *y));
                let ink = color(*rgb);
                // A bold line is the same text printed twice, a hair apart.
                let copies: &[f32] = if *bold { &[0.0, 0.5] } else { &[0.0] };
                if *vertical {
                    let galley = painter.layout_no_wrap(text.clone(), font, ink);
                    let (w, h) = (galley.size().x, galley.size().y);
                    // Turned by 90° counter-clockwise: the text runs up from `pos`.
                    let along = match align {
                        Align::Left => 0.0,
                        Align::Center => w / 2.0,
                        Align::Right => w,
                    };
                    for dx in copies {
                        let pos = Pos2::new(at.x - h / 2.0 + dx, at.y + along);
                        painter.add(
                            egui::epaint::TextShape::new(pos, galley.clone(), ink)
                                .with_angle(-std::f32::consts::FRAC_PI_2),
                        );
                    }
                } else {
                    let anchor = match align {
                        Align::Left => egui::Align2::LEFT_CENTER,
                        Align::Center => egui::Align2::CENTER_CENTER,
                        Align::Right => egui::Align2::RIGHT_CENTER,
                    };
                    for dx in copies {
                        painter.text(at + Vec2::new(*dx, 0.0), anchor, text, font.clone(), ink);
                    }
                }
            }
        }
    }
}

/// The preview of the sheets (the middle of the window).
pub fn sheet(app: &mut EditorApp, ui: &mut Ui) {
    refresh(app);
    if app.doc.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(tr("Откройте проект: Файл → Открыть, или создайте новый."));
        });
        return;
    }
    toolbar(app, ui);
    ui.separator();

    // The cache is taken out while the sheet is drawn: the view state changes meanwhile.
    let Some(cache) = app.panel_view.cache.take() else {
        return;
    };
    draw_sheet(app, ui, &cache);
    app.panel_view.cache = Some(cache);
}

/// Draws the sheet of the preview and handles the mouse on it.
fn draw_sheet(app: &mut EditorApp, ui: &mut Ui, cache: &Cache) {
    let document = match &cache.document {
        Ok(document) if !document.sheets.is_empty() => document,
        Ok(_) => {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(tr(
                    "Листов пока нет. Добавьте в дереве слева ввод питания, автомат и нагрузку: схема появится здесь.",
                ));
            });
            return;
        }
        Err(message) => {
            ui.colored_label(app.theme.palette.error, message);
            return;
        }
    };
    let index = app.panel_view.sheet.min(document.sheets.len() - 1);
    let lang = cache.key.lang;
    let (page_w, page_h) = document.page_size();
    let sheet = &document.sheets[index];

    let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let area = resp.rect;
    let view_state = &mut app.panel_view;
    let fit = (area.width() / page_w)
        .min(area.height() / page_h)
        .max(0.01);

    // Zoom around the pointer with the wheel, move with the drag.
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = fit * view_state.zoom;
            let new_zoom = (view_state.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 12.0);
            if let Some(pointer) = resp.hover_pos() {
                let corner = area.min
                    + (area.size() - Vec2::new(page_w, page_h) * old) / 2.0
                    + view_state.pan;
                let on_page = (pointer - corner) / old;
                let new = fit * new_zoom;
                let new_corner = area.min + (area.size() - Vec2::new(page_w, page_h) * new) / 2.0;
                view_state.pan = pointer - on_page * new - new_corner;
            }
            view_state.zoom = new_zoom;
        }
    }
    // Where the page was at the start of the frame: a press on a symbol or on the middle of a
    // wire takes the drag; anything else moves the view.
    let tolerance = 6.0 / (fit * view_state.zoom);
    // A text is moved with Shift held; without it the drag takes the symbol or the wire.
    let shift_held = ui.input(|i| i.modifiers.shift);
    // With Ctrl held the end of a wire is taken to another terminal.
    let ctrl_held = ui.input(|i| i.modifiers.command);
    if resp.drag_started() {
        let scale0 = fit * view_state.zoom;
        let origin0 =
            area.min + (area.size() - Vec2::new(page_w, page_h) * scale0) / 2.0 + view_state.pan;
        view_state.drag = resp.interact_pointer_pos().and_then(|pos| {
            let p = ((pos - origin0) / scale0).to_pos2();
            // With Shift held a text is picked before the symbol and the wire it stands on.
            if let Some(key) = label_at(sheet, p).filter(|_| shift_held) {
                let shift = app
                    .doc
                    .as_ref()
                    .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(key));
                return Some(Drag::Symbol {
                    key: key.to_owned(),
                    start: (p.x, p.y),
                    shift,
                });
            }
            if let (true, Some(doc)) = (ctrl_held, app.doc.as_ref()) {
                if let Some((key, end, at)) = wire_end_at(sheet, p, tolerance * 1.5) {
                    let targets = rewire_targets(sheet, &doc.project, &app.lib, &key, end);
                    return Some(Drag::Rewire {
                        key,
                        end,
                        at,
                        targets,
                    });
                }
            }
            if let Some((device, terminal, at)) = app
                .doc
                .as_ref()
                .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * 1.5))
            {
                return Some(Drag::Link {
                    from: (device, terminal),
                    at,
                });
            }
            if let Some((key, _)) = bend_at(sheet, p, tolerance) {
                return Some(Drag::Bend {
                    key: key.to_owned(),
                });
            }
            if let Some(key) = bus_at(sheet, p, tolerance) {
                let shift = app
                    .doc
                    .as_ref()
                    .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(&key));
                return Some(Drag::Symbol {
                    key,
                    start: (p.x, p.y),
                    shift,
                });
            }
            let (_, key) = movable(symbol_at(sheet, p)?)?;
            let shift = app
                .doc
                .as_ref()
                .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(&key));
            Some(Drag::Symbol {
                key,
                start: (p.x, p.y),
                shift,
            })
        });
    }
    if resp.dragged() && view_state.drag.is_none() {
        view_state.pan += resp.drag_delta();
    }
    if let Some((x, y)) = view_state.focus.take() {
        view_state.zoom = view_state.zoom.max(2.5);
        let s = fit * view_state.zoom;
        let centred = area.min + (area.size() - Vec2::new(page_w, page_h) * s) / 2.0;
        view_state.pan = area.center() - (centred + Vec2::new(x, y) * s);
    }
    if view_state.zoom <= 1.001 {
        view_state.zoom = 1.0;
        view_state.pan = Vec2::ZERO;
    }
    let scale = fit * view_state.zoom;
    let origin =
        area.min + (area.size() - Vec2::new(page_w, page_h) * scale) / 2.0 + view_state.pan;
    let view = View { origin, scale };

    let painter = painter.with_clip_rect(area);
    painter.rect_filled(area, 0.0, ui.visuals().extreme_bg_color);
    let paper = Rect::from_min_size(origin, Vec2::new(page_w, page_h) * scale);
    painter.rect_filled(paper, 0.0, Color32::WHITE);
    draw_ops(&painter, &view, &document.frame_ops(index, lang));
    draw_ops(&painter, &view, &sheet.ops);

    // The mouse: what is under it, the drag in progress, the selection.
    let page_at = |pos: Pos2| ((pos - origin) / scale).to_pos2();
    let hover = resp.hover_pos().map(page_at);
    let mut layout_edit: Option<LayoutEdit> = None;
    let mut new_link: Option<crab_core::WireLink> = None;
    let mut rewire: Option<(String, crab_core::WireEnd, String)> = None;
    let mut remove_link: Option<String> = None;
    let mut unplace: Option<String> = None;
    let mut select = None;
    if let Some(drag) = view_state.drag.clone() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let (Some(pos), true) = (resp.interact_pointer_pos(), resp.dragged()) {
            let p = page_at(pos);
            match &drag {
                Drag::Symbol { key, start, shift } => {
                    layout_edit = Some(LayoutEdit::Shift {
                        // A supply line moves up and down only.
                        dx: if key.starts_with("bus:") {
                            0.0
                        } else {
                            shift.0 + p.x - start.0
                        },
                        key: key.clone(),
                        dy: shift.1 + p.y - start.1,
                    });
                }
                Drag::Bend { key } => {
                    layout_edit = Some(LayoutEdit::Bend {
                        key: key.clone(),
                        y: p.y,
                    });
                }
                // The wire being drawn follows the pointer.
                Drag::Link { at, .. } => {
                    painter.line_segment(
                        [view.at(*at), pos],
                        egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                    );
                }
                // The end being moved follows the pointer; the terminals it may go to are marked,
                // the one under the pointer brighter.
                Drag::Rewire { at, targets, .. } => {
                    let color = ui.visuals().selection.stroke.color;
                    painter.line_segment([view.at(*at), pos], egui::Stroke::new(1.5, color));
                    for (_, q) in targets {
                        let near = (q.0 - p.x).hypot(q.1 - p.y) <= tolerance * 1.5;
                        let r = if near { 6.0 } else { 4.0 };
                        painter.circle_stroke(view.at(*q), r, egui::Stroke::new(1.5, color));
                    }
                }
            }
        }
        if resp.drag_stopped() {
            view_state.drag = None;
            if let Drag::Rewire { key, end, .. } = &drag {
                // Let go over a terminal: the core decides, a refusal is shown with its reason.
                let target = resp.interact_pointer_pos().map(page_at).and_then(|p| {
                    app.doc
                        .as_ref()
                        .and_then(|d| terminal_at(sheet, &d.project.panel, p, tolerance * 1.5))
                });
                if let Some(target) = target {
                    rewire = Some((key.clone(), *end, target));
                }
            } else if let Drag::Link { from, .. } = &drag {
                // Let go over another terminal: the wire is made.
                let target = resp.interact_pointer_pos().map(page_at).and_then(|p| {
                    app.doc
                        .as_ref()
                        .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * 1.5))
                });
                if let Some((device, terminal, _)) = target {
                    if (device.as_str(), terminal.as_str()) != (from.0.as_str(), from.1.as_str()) {
                        new_link = Some(crab_core::WireLink {
                            from: format!("{}:{}", from.0, from.1),
                            to: format!("{device}:{terminal}"),
                        });
                    }
                }
            } else {
                layout_edit = Some(LayoutEdit::End);
            }
        }
        ui.ctx().request_repaint();
    } else if let Some(p) = hover {
        if ctrl_held && wire_end_at(sheet, p, tolerance * 1.5).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        } else if app
            .doc
            .as_ref()
            .is_some_and(|d| port_at(sheet, &d.project.panel, p, tolerance * 1.5).is_some())
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if shift_held && label_at(sheet, p).is_some() && ref_at(sheet, p).is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if ref_at(sheet, p).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if bend_at(sheet, p, tolerance).is_some() || bus_at(sheet, p, tolerance).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        } else if symbol_at(sheet, p).is_some_and(|i| movable(i).is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    // A right click on the end reference of a supply line edits where the line is carried on to.
    if resp.secondary_clicked() {
        let click = resp.interact_pointer_pos().map(page_at);
        view_state.link_menu = None;
        view_state.unplace_menu = None;
        if let (Some(p), Some(doc)) = (click, app.doc.as_ref()) {
            view_state.link_menu = link_at(sheet, &doc.project.panel, p, tolerance);
            if view_state.link_menu.is_none() {
                view_state.unplace_menu = symbol_at(sheet, p)
                    .and_then(|i| i.device())
                    .filter(|d| doc.project.panel_layout.placed.contains_key(*d))
                    .map(str::to_owned);
            }
        }
        view_state.bus_edit = resp
            .interact_pointer_pos()
            .map(page_at)
            .and_then(|p| {
                // The reference at the end of a line that goes on, or any place of the line.
                bus_end_at(sheet, p, page_w).or_else(|| {
                    let (first_power, _) = power_span(document);
                    bus_hit(sheet, p, tolerance).map(|(index, _)| (index, index + first_power + 1))
                })
            })
            .map(|(index, shown)| {
                let manual = app
                    .doc
                    .as_ref()
                    .and_then(|d| d.project.panel_layout.bus_to(index));
                (index, manual.unwrap_or(shown))
            });
    }
    if let Some((index, mut to)) = view_state.bus_edit {
        let (first_power, last_power) = power_span(document);
        let here = index + first_power;
        resp.context_menu(|ui| {
            ui.label(tr("Продлить линии питания до листа"));
            let changed = ui
                .add(egui::DragValue::new(&mut to).range(
                    (here + 1)..=(last_power + crab_schematic::power::MAX_EXTRA).max(here + 1),
                ))
                .changed();
            if changed {
                view_state.bus_edit = Some((index, to));
                layout_edit = Some(LayoutEdit::BusTo {
                    index,
                    to: Some(to),
                });
            }
            if ui.button(tr("Автоматически")).clicked() {
                layout_edit = Some(LayoutEdit::BusTo { index, to: None });
                view_state.bus_edit = None;
                ui.close();
            }
        });
    }
    if view_state.bus_edit.is_none() {
        if let Some(key) = view_state.link_menu.clone() {
            resp.context_menu(|ui| {
                if ui.button(tr("Удалить соединение")).clicked() {
                    remove_link = Some(key.clone());
                    view_state.link_menu = None;
                    ui.close();
                }
            });
        } else if let Some(id) = view_state.unplace_menu.clone() {
            resp.context_menu(|ui| {
                if ui.button(tr("Убрать с листа")).clicked() {
                    unplace = Some(id.clone());
                    view_state.unplace_menu = None;
                    ui.close();
                }
            });
        }
    }
    // A device dragged from the tree and dropped here is put on the sheet it is dropped on.
    let mut put: Option<(String, SheetId, f32, f32)> = None;
    if let Some(payload) = resp.dnd_release_payload::<DevicePayload>() {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let at = page_at(pos);
            let id = payload.0.clone();
            if let Some(why) = drop_refusal(&document.sheets, &document.coil_at, &id) {
                view_state.problem = Some(tr(why).to_owned());
            } else if let Some(target) = sheet.id {
                put = Some((id, target, at.x, at.y));
            } else {
                // Dropped on a title or a table sheet: the device goes on the first control sheet,
                // which is shown, so that it can be put in place and wired.
                put = Some((id, SheetId::Control(0), 80.0, 400.0));
                view_state.goto = Some(SheetId::Control(0));
            }
        }
    }
    // A click on a reference goes to the sheet and the zone it names.
    let mut jump: Option<(usize, String)> = None;
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos().map(page_at) {
            if let Some((target, zone)) = ref_at(sheet, p) {
                jump = Some((target.map_or(index, |n| n.saturating_sub(1)), zone));
            } else if let Some(key) = label_at(sheet, p).filter(|_| shift_held) {
                // A text is picked (with Shift held): its properties (turn, size) are shown at the right.
                let size = sheet
                    .items
                    .iter()
                    .find(|i| matches!(&i.kind, ItemKind::Label { key: k } if k == key))
                    .and_then(|i| i.ops.first())
                    .and_then(|&o| match sheet.ops.get(o)? {
                        Op::Text { size, .. } => Some(*size),
                        _ => None,
                    })
                    .unwrap_or(17.0);
                view_state.label = Some((key.to_owned(), size));
                view_state.selected = None;
            } else {
                // A part (`K1/coil`) selects its device.
                select = symbol_at(sheet, p)
                    .and_then(|i| i.device())
                    .map(|d| crab_core::split_part(d).0.to_owned());
            }
        }
    }
    if resp.double_clicked() {
        if let Some((key, _)) = resp
            .interact_pointer_pos()
            .and_then(|pos| bend_at(sheet, page_at(pos), tolerance))
        {
            layout_edit = Some(LayoutEdit::ResetBend {
                key: key.to_owned(),
            });
        }
    }
    // The symbols of the selected device are framed.
    let accent = app.theme.palette.accent;
    if let Some(chosen) = &view_state.selected {
        for item in &sheet.items {
            if item.device().map(|d| crab_core::split_part(d).0) == Some(chosen.as_str()) {
                if let Some(r) = item_bounds(sheet, item) {
                    let r = Rect::from_min_max(
                        view.at((r.min.x, r.min.y)),
                        view.at((r.max.x, r.max.y)),
                    )
                    .expand(3.0);
                    painter.rect_stroke(
                        r,
                        2.0,
                        Stroke::new(1.5, accent),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }
    if let Some((on, zone)) = &view_state.highlight {
        if *on == index {
            if let Some((x, y, w, h)) = document.zone_rect(zone) {
                let rect = Rect::from_min_size(view.at((x, y)), Vec2::new(w, h) * scale);
                let accent = app.theme.palette.accent;
                painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.18));
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(2.0, accent),
                    egui::StrokeKind::Middle,
                );
            }
        }
    }
    if let Some((sheet_index, zone)) = jump {
        if sheet_index < document.sheets.len() {
            view_state.focus = document
                .zone_rect(&zone)
                .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0));
            view_state.sheet = sheet_index;
            view_state.highlight = Some((sheet_index, zone));
        }
    }
    let reset_bend = matches!(layout_edit, Some(LayoutEdit::ResetBend { .. }));
    if resp.double_clicked() && !reset_bend {
        view_state.zoom = 1.0;
        view_state.pan = Vec2::ZERO;
        view_state.highlight = None;
    }
    if let Some(id) = select {
        if app
            .doc
            .as_ref()
            .is_some_and(|d| d.project.panel.device(&id).is_some())
        {
            view_state.selected = Some(id);
            view_state.coil = false;
            view_state.label = None;
        }
    }
    if let (Some(edit), Some(doc)) = (layout_edit, app.doc.as_mut()) {
        apply_layout_edit(doc, edit);
    }
    if let (Some((key, end, target)), Some(doc)) = (rewire, app.doc.as_mut()) {
        let lib = &app.lib;
        match doc.edit_panel_project(None, |p| crab_core::rewire(p, lib, &key, end, &target)) {
            Ok(()) => app.panel_view.problem = None,
            Err(message) => {
                // The device of the moved end is selected so the reason shows in its properties.
                let moved = key.split_once('>').map(|(a, b)| match end {
                    crab_core::WireEnd::From => a,
                    crab_core::WireEnd::To => b,
                });
                if let Some(device) = moved.map(crab_core::link_device) {
                    app.panel_view.selected = Some(device.to_owned());
                    app.panel_view.coil = false;
                }
                app.panel_view.problem = Some(message);
            }
        }
    }
    if let Some(doc) = app.doc.as_mut() {
        if let Some(link) = new_link {
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                if !p.links.contains(&link) {
                    p.links.push(link);
                }
                Ok(())
            });
        }
        if let Some(key) = remove_link {
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                p.links.retain(|l| l.key() != key);
                Ok(())
            });
        }
        if let Some(id) = unplace {
            doc.edit_panel_layout(None, |l| l.unplace(&id));
        }
        if let Some((id, target, x, y)) = put {
            doc.edit_panel_layout(None, |l| l.place(&id, target, x, y));
            app.panel_view.selected = Some(id);
            app.panel_view.coil = false;
        }
    }
}

/// Puts a change of the layout made by the mouse into the document.
fn apply_layout_edit(doc: &mut crate::doc::Doc, edit: LayoutEdit) {
    match edit {
        LayoutEdit::Shift { key, dx, dy } => {
            doc.edit_panel_layout(Some(format!("shift:{key}")), |l| l.set_shift(&key, dx, dy));
        }
        LayoutEdit::Bend { key, y } => {
            doc.edit_panel_layout(Some(format!("bend:{key}")), |l| l.set_bend(&key, Some(y)));
        }
        LayoutEdit::ResetBend { key } => {
            doc.edit_panel_layout(None, |l| l.set_bend(&key, None));
        }
        LayoutEdit::BusTo { index, to } => {
            doc.edit_panel_layout(Some(format!("busto:{index}")), |l| l.set_bus_to(index, to));
        }
        LayoutEdit::End => doc.end_edit(),
    }
}

/// Makes the PDF of the panel next to the project file, `<name>_panel.pdf`, in a thread.
fn start_pdf(
    project: crab_core::Project,
    lib: crab_core::Library,
    lang: Lang,
    format: Format,
) -> std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let dir = project
                .path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            let out = dir.join(format!("{}_panel.pdf", project.display_name()));
            let pdf = crab_schematic::render_pdf_in(&project, &lib, lang, format)
                .map_err(|e| e.to_string())?;
            std::fs::write(&out, pdf).map_err(|e| e.to_string())?;
            Ok(out)
        })();
        let _ = tx.send(result);
    });
    rx
}

/// The buttons of the shown sheet: its caption, a new free sheet, removing a free sheet.
fn sheet_controls(
    view: &mut PanelView,
    doc: &mut Option<crate::doc::Doc>,
    ui: &mut Ui,
    shown: Option<SheetId>,
    title: Option<&String>,
    free_count: usize,
) {
    let Some(doc) = doc else {
        return;
    };
    if let Some(id) = shown {
        let editing = view.caption_edit.as_ref().is_some_and(|(e, _)| *e == id);
        if !editing {
            let text = doc
                .project
                .panel_layout
                .caption(id)
                .unwrap_or("")
                .to_owned();
            view.caption_edit = Some((id, text));
        }
        if let Some((_, text)) = &mut view.caption_edit {
            let hint = title.map_or("", |t| t.as_str());
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .desired_width(160.0),
            );
            response.clone().on_hover_text(tr(
                "Подпись листа: идёт в основную надпись и в содержание; пусто — подпись по умолчанию",
            ));
            if response.lost_focus() {
                let text = text.clone();
                if doc.project.panel_layout.caption(id).unwrap_or("") != text.trim() {
                    doc.edit_panel_layout(None, |l| l.set_caption(id, &text));
                }
            }
        }
    }
    if ui
        .button("+")
        .on_hover_text(tr("Добавить свободный лист"))
        .clicked()
    {
        let id = SheetId::Free(free_count);
        let caption = format!("{} {}", tr("Схема"), free_count + 1);
        doc.edit_panel_layout(None, |l| l.set_caption(id, &caption));
        view.goto = Some(id);
        view.caption_edit = None;
    }
    if let Some(SheetId::Free(k)) = shown {
        if ui
            .button(tr("Удалить лист"))
            .on_hover_text(tr(
                "Удалить этот свободный лист; устройства с него вернутся в дерево, следующие листы сдвинутся",
            ))
            .clicked()
        {
            doc.edit_panel_layout(None, |l| l.remove_free_sheet(k));
            view.caption_edit = None;
        }
    }
}

/// Above the sheet: the sheets, the language and the format.
fn toolbar(app: &mut EditorApp, ui: &mut Ui) {
    let Some(cache) = &app.panel_view.cache else {
        return;
    };
    let titles: Vec<String> = cache.document.as_ref().map_or_else(
        |_| Vec::new(),
        |d| d.sheets.iter().map(|s| s.title.clone()).collect(),
    );
    let ids: Vec<Option<SheetId>> = cache
        .document
        .as_ref()
        .map_or_else(|_| Vec::new(), |d| d.sheets.iter().map(|s| s.id).collect());
    let free_count = ids
        .iter()
        .filter(|id| matches!(id, Some(SheetId::Free(_))))
        .count();
    let errors = cache.report.errors().count();
    let view = &mut app.panel_view;
    ui.horizontal(|ui| {
        let count = titles.len();
        if ui
            .add_enabled(view.sheet > 0, egui::Button::new("◀"))
            .clicked()
        {
            view.sheet -= 1;
            view.highlight = None;
        }
        let shown = titles.get(view.sheet).map_or_else(String::new, |t| {
            format!("{} / {count}  {t}", view.sheet + 1)
        });
        egui::ComboBox::from_id_salt("panel_sheet")
            .selected_text(shown)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (i, title) in titles.iter().enumerate() {
                    if ui
                        .selectable_value(&mut view.sheet, i, format!("{} {title}", i + 1))
                        .clicked()
                    {
                        view.highlight = None;
                    }
                }
            });
        if ui
            .add_enabled(view.sheet + 1 < count, egui::Button::new("▶"))
            .clicked()
        {
            view.sheet += 1;
            view.highlight = None;
        }
        sheet_controls(view, &mut app.doc, ui, ids.get(view.sheet).copied().flatten(), titles.get(view.sheet), free_count);
        ui.separator();
        let mut lang = view.language();
        ui.selectable_value(&mut lang, Lang::Ru, "RU");
        ui.selectable_value(&mut lang, Lang::En, "EN");
        if lang != view.language() {
            view.lang = Some(lang);
        }
        ui.separator();
        ui.selectable_value(&mut view.format, Format::A3, "A3");
        ui.selectable_value(&mut view.format, Format::A4, "A4");
        ui.separator();
        if let Some(doc) = &mut app.doc {
            let mine = view
                .selected
                .as_ref()
                .map(|id| (symbol_key("power", id), symbol_key("control", id)));
            let has_mine = mine.as_ref().is_some_and(|(a, b)| {
                doc.project.panel_layout.symbols.contains_key(a)
                    || doc.project.panel_layout.symbols.contains_key(b)
            });
            if ui
                .add_enabled(has_mine, egui::Button::new(tr("Сбросить положение")))
                .on_hover_text(tr("Вернуть выбранному устройству автоматическое положение"))
                .clicked()
            {
                if let Some((a, b)) = mine {
                    doc.edit_panel_layout(None, |l| {
                        l.clear_symbol(&a);
                        l.clear_symbol(&b);
                    });
                }
            }
            let layout = &doc.project.panel_layout;
            let any = !layout.symbols.is_empty()
                || !layout.wires.is_empty()
                || !layout.bus_to.is_empty()
                || !layout.placed.is_empty();
            if view.confirm_reset {
                ui.label(tr("Убрать всё, что расставлено вручную?"));
                if ui.button(tr("Да, разложить")).clicked() {
                    doc.edit_panel_layout(None, |l| {
                        l.clear_all();
                        l.placed.clear();
                    });
                    view.confirm_reset = false;
                }
                if ui.button(tr("Отмена")).clicked() {
                    view.confirm_reset = false;
                }
            } else if ui
                .add_enabled(any, egui::Button::new(tr("Разложить заново всё")))
                .on_hover_text(tr(
                    "Вернуть все символы и провода на автоматические места и убрать с листов то, что положено вручную (подписи листов остаются)",
                ))
                .clicked()
            {
                view.confirm_reset = true;
            }
            ui.separator();
        }
        let busy = view.pdf_job.is_some();
        if ui
            .add_enabled(!busy && count > 0, egui::Button::new(tr("Собрать PDF")))
            .clicked()
        {
            if let Some(doc) = &app.doc {
                view.pdf_result = None;
                view.pdf_job = Some(start_pdf(
                    doc.project.clone(),
                    app.lib.clone(),
                    view.language(),
                    view.format,
                ));
            }
        }
        if busy {
            ui.spinner();
            use std::sync::mpsc::TryRecvError;
            match view.pdf_job.as_ref().map(|j| j.try_recv()) {
                Some(Ok(done)) => {
                    view.pdf_result = Some(done);
                    view.pdf_job = None;
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    view.pdf_result = Some(Err(tr("PDF не собран").to_owned()));
                    view.pdf_job = None;
                }
                _ => {}
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        match &view.pdf_result {
            Some(Ok(path)) => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ui.label(crate::i18n::fmt(tr("Готово: {0}"), &[&name]))
                    .on_hover_text(path.display().to_string());
            }
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
            None => {}
        }
        if errors > 0 {
            ui.separator();
            let message = crate::i18n::fmt(tr("Ошибок в щите: {0}"), &[&errors]);
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_is_about_a_device_by_its_designation() {
        assert!(is_about(
            "-K1 не привязано к входу или выходу проекта",
            "K1"
        ));
        assert!(is_about("устройство «K1»: недопустимое обозначение", "K1"));
        assert!(is_about(
            "обозначение -K1 занято несколькими устройствами",
            "K1"
        ));
        // Not another device that starts the same way.
        assert!(!is_about("-K12 не привязано", "K1"));
        assert!(!is_about("-K1_a не привязано", "K1"));
        assert!(!is_about("-XK1 не привязано", "K1"));
        assert!(!is_about("всё в порядке", "K1"));
    }

    #[test]
    fn the_language_of_the_sheets_follows_the_editor_until_chosen() {
        let mut view = PanelView::default();
        let editor = editor_lang();
        assert_eq!(view.language(), editor);
        view.lang = Some(if editor == Lang::Ru {
            Lang::En
        } else {
            Lang::Ru
        });
        assert_ne!(view.language(), editor);
    }

    #[test]
    fn every_kind_has_a_title_and_belongs_to_one_group() {
        for kind in DeviceKind::ALL {
            assert!(!kind_title(kind).is_empty());
            let groups = GROUPS
                .iter()
                .filter(|(_, kinds)| kinds.contains(&kind))
                .count();
            assert_eq!(groups, 1, "{kind:?}");
        }
    }

    /// The sheets of a panel are drawn without a panic, with every kind of operation, at the
    /// scales of the fit and of a close look.
    #[test]
    fn drawing_the_operations_of_every_sheet_does_not_panic() {
        let (lib, _) =
            crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"))
                .unwrap();
        let mut project =
            crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
                .unwrap();
        project.panel = toml::from_str(
            r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#,
        )
        .unwrap();
        for lang in [Lang::Ru, Lang::En] {
            let (document, _) = build_document_in(&project, &lib, lang, Format::A3).unwrap();
            let (page_w, _) = document.page_size();
            let ctx = egui::Context::default();
            for scale in [0.4, 2.0] {
                let mut drawn = 0;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let view = View {
                        origin: Pos2::new(10.0, 10.0),
                        scale: scale * 1000.0 / page_w,
                    };
                    let painter = ui.painter().clone();
                    for (i, sheet) in document.sheets.iter().enumerate() {
                        draw_ops(&painter, &view, &document.frame_ops(i, lang));
                        draw_ops(&painter, &view, &sheet.ops);
                        drawn += sheet.ops.len();
                    }
                });
                // The textures of the fonts were made for nobody: say they are not needed.
                output.textures_delta.clear();
                assert!(drawn > 100, "{drawn}");
            }
        }
    }

    fn symbol(view: &'static str, device: &str, ops: Vec<usize>) -> Item {
        Item {
            kind: ItemKind::Symbol {
                view,
                device: device.to_owned(),
                origin: (0.0, 0.0),
                ports: Vec::new(),
            },
            ops,
        }
    }

    fn line(points: &[(f32, f32)]) -> Op {
        Op::Line {
            points: points.to_vec(),
            width: 1.0,
            dashed: false,
            rgb: (0, 0, 0),
        }
    }

    fn sheet_of(ops: Vec<Op>, items: Vec<Item>) -> Sheet {
        Sheet {
            title: String::new(),
            id: None,
            body: String::new(),
            ops,
            items,
        }
    }

    #[test]
    fn a_wire_from_a_part_ends_at_its_device() {
        let part = |view: &'static str, device: &str, name: &str| Item {
            kind: ItemKind::Symbol {
                view,
                device: device.to_owned(),
                origin: (0.0, 0.0),
                ports: vec![crab_schematic::scene::Port {
                    name: name.to_owned(),
                    x: 10.0,
                    y: 10.0,
                    dir: crab_schematic::scene::Dir::Down,
                }],
            },
            ops: Vec::new(),
        };
        let panel = crab_core::Panel::default();
        let channel = sheet_of(Vec::new(), vec![part("board", "A1/IO5", "IO5")]);
        let (device, terminal, _) =
            port_at(&channel, &panel, Pos2::new(10.0, 10.0), 2.0).expect("a port");
        assert_eq!((device.as_str(), terminal.as_str()), ("A1", "IO5"));
        let coil = sheet_of(Vec::new(), vec![part("control", "K1/coil", "A1")]);
        let (device, terminal, _) =
            port_at(&coil, &panel, Pos2::new(10.0, 10.0), 2.0).expect("a port");
        assert_eq!((device.as_str(), terminal.as_str()), ("K1", "A1"));
    }

    /// The rewiring gesture on the pump example: the end of every wire is found under the
    /// pointer, every terminal marked as allowed is accepted by the core, and the panel still
    /// builds after the change.
    #[test]
    fn every_marked_terminal_takes_the_end_of_the_wire() {
        let (lib, _) =
            crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"))
                .unwrap();
        let project =
            crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
                .unwrap();
        let (document, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
        let mut moved = 0;
        for sheet in &document.sheets {
            for item in &sheet.items {
                let ItemKind::Wire { key, .. } = &item.kind else {
                    continue;
                };
                let lines: Vec<&Vec<(f32, f32)>> = item
                    .ops
                    .iter()
                    .filter_map(|&o| match &sheet.ops[o] {
                        Op::Line { points, .. } => Some(points),
                        _ => None,
                    })
                    .collect();
                let (Some(first), Some(last)) = (
                    lines.first().and_then(|l| l.first()),
                    lines.last().and_then(|l| l.last()),
                ) else {
                    continue;
                };
                for (end, at) in [
                    (crab_core::WireEnd::From, *first),
                    (crab_core::WireEnd::To, *last),
                ] {
                    let found = wire_end_at(sheet, Pos2::new(at.0, at.1), 1.0);
                    assert!(found.is_some(), "конец провода {key} не найден");
                    for (target, _) in rewire_targets(sheet, &project, &lib, key, end) {
                        let mut copy = project.clone();
                        crab_core::rewire(&mut copy, &lib, key, end, &target)
                            .unwrap_or_else(|e| panic!("{key} → {target}: {e}"));
                        build_document_in(&copy, &lib, Lang::Ru, Format::A3)
                            .unwrap_or_else(|e| panic!("{key} → {target}: {e:?}"));
                        moved += 1;
                    }
                }
            }
        }
        assert!(moved > 0, "ни один конец провода нельзя перепаять");
    }

    /// A signal wire is drawn from the device to the channel of the board, though its key starts
    /// at the board: the end at the channel is still the `from` end.
    #[test]
    fn the_end_at_the_board_is_the_board_end_of_the_key() {
        let (lib, _) =
            crab_core::Library::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"))
                .unwrap();
        let project =
            crab_core::Project::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/pump"))
                .unwrap();
        let (document, _) = build_document_in(&project, &lib, Lang::Ru, Format::A3).unwrap();
        let mut checked = 0;
        for sheet in &document.sheets {
            let board_ports: Vec<(f32, f32)> = sheet
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    ItemKind::Symbol {
                        view: "board",
                        ports,
                        ..
                    } => Some(ports.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()),
                    _ => None,
                })
                .flatten()
                .collect();
            for item in &sheet.items {
                let ItemKind::Wire { key, from, .. } = &item.kind else {
                    continue;
                };
                if from.device != crab_schematic::BOARD {
                    continue;
                }
                let points: Vec<(f32, f32)> = item
                    .ops
                    .iter()
                    .filter_map(|&o| match &sheet.ops[o] {
                        Op::Line { points, .. } => Some(points.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                let at_board = points.iter().copied().find(|q| {
                    board_ports
                        .iter()
                        .any(|b| (b.0 - q.0).hypot(b.1 - q.1) <= 3.0)
                });
                let Some(q) = at_board else {
                    continue;
                };
                let (k, end, _) = wire_end_at(sheet, Pos2::new(q.0, q.1), 0.5).expect("конец");
                if k == *key {
                    assert_eq!(end, crab_core::WireEnd::From, "{key}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "нет сигнальных проводов к плате");
    }

    #[test]
    fn a_coil_drawn_by_the_generator_cannot_be_dropped_but_a_hand_coil_moves() {
        let mut coil_at = crab_schematic::Xrefs::new();
        coil_at.insert(
            "K1".to_owned(),
            crab_schematic::Xref {
                sheet: 3,
                zone: "2".to_owned(),
            },
        );
        // The generator draws the coil of K1: neither the coil nor the device can be dropped.
        assert!(drop_refusal(&[], &coil_at, "K1/coil").is_some());
        assert!(drop_refusal(&[], &coil_at, "K1").is_some());
        // A coil already put by hand is drawn as its own symbol: it can be moved to another sheet.
        let hand = sheet_of(Vec::new(), vec![symbol("control", "K1/coil", Vec::new())]);
        assert!(drop_refusal(&[hand], &coil_at, "K1/coil").is_none());
        // A device whose coil nobody draws can be dropped, and so can its coil.
        assert!(drop_refusal(&[], &coil_at, "K2/coil").is_none());
        assert!(drop_refusal(&[], &coil_at, "K2").is_none());
    }

    #[test]
    fn the_topmost_symbol_under_a_point_is_found() {
        let sheet = sheet_of(
            vec![
                Op::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 100.0,
                    width: 1.0,
                    rgb: (0, 0, 0),
                },
                Op::Rect {
                    x: 50.0,
                    y: 50.0,
                    w: 100.0,
                    h: 100.0,
                    width: 1.0,
                    rgb: (0, 0, 0),
                },
            ],
            vec![
                symbol("power", "Q1", vec![0]),
                symbol("power", "K1", vec![1]),
            ],
        );
        let at = |x: f32, y: f32| symbol_at(&sheet, Pos2::new(x, y)).and_then(Item::device);
        assert_eq!(at(10.0, 10.0), Some("Q1"));
        // Where they overlap, the one drawn last is on top.
        assert_eq!(at(70.0, 70.0), Some("K1"));
        assert_eq!(at(400.0, 400.0), None);
    }

    #[test]
    fn only_a_symbol_of_a_device_view_can_be_moved() {
        assert_eq!(
            movable(&symbol("power", "K1", vec![])),
            Some(("K1", "power:K1".to_owned()))
        );
        assert_eq!(movable(&symbol("board", "A1", vec![])), None);
    }

    #[test]
    fn the_middle_segment_of_a_routed_wire_is_a_handle_but_a_straight_wire_is_not() {
        let wire = |key: &str, op: usize| Item {
            kind: ItemKind::Wire {
                key: key.to_owned(),
                from: crab_schematic::End::new("A", "out:0"),
                to: crab_schematic::End::new("B", "in:0"),
            },
            ops: vec![op],
        };
        let sheet = sheet_of(
            vec![
                line(&[(0.0, 0.0), (0.0, 50.0), (40.0, 50.0), (40.0, 100.0)]),
                line(&[(200.0, 0.0), (200.0, 100.0)]),
            ],
            vec![wire("routed", 0), wire("straight", 1)],
        );
        let hit = |x: f32, y: f32| bend_at(&sheet, Pos2::new(x, y), 4.0);
        assert_eq!(hit(20.0, 51.0), Some(("routed", 50.0)));
        // Off the segment, and on a vertical part.
        assert_eq!(hit(20.0, 70.0), None);
        assert_eq!(hit(0.0, 20.0), None);
        // A straight wire has nothing to bend.
        assert_eq!(hit(200.0, 50.0), None);
    }

    fn text(x: f32, y: f32, text: &str) -> Op {
        Op::Text {
            x,
            y,
            text: text.to_owned(),
            size: 17.0,
            align: Align::Left,
            bold: false,
            vertical: false,
            rgb: (0, 0, 0),
        }
    }

    #[test]
    fn a_reference_under_a_point_names_its_sheet_and_zone() {
        let sheet = sheet_of(
            vec![
                text(100.0, 50.0, "/3.4"),
                text(100.0, 200.0, "K1"),
                text(300.0, 50.0, "13 /.2"),
            ],
            Vec::new(),
        );
        assert_eq!(
            ref_at(&sheet, Pos2::new(120.0, 50.0)),
            Some((Some(3), "4".to_owned()))
        );
        // The sheet is left out for a column of the same sheet.
        assert_eq!(
            ref_at(&sheet, Pos2::new(330.0, 50.0)),
            Some((None, "2".to_owned()))
        );
        // A caption that is no reference, and the empty places, are not links.
        assert_eq!(ref_at(&sheet, Pos2::new(110.0, 200.0)), None);
        assert_eq!(ref_at(&sheet, Pos2::new(500.0, 500.0)), None);
    }

    #[test]
    fn a_contact_is_in_one_circuit_only() {
        let mut contact = Device {
            id: "Q1_1".into(),
            kind: DeviceKind::Aux,
            rating: String::new(),
            part: String::new(),
            bind: "button".into(),
            load: String::new(),
            poles: None,
            of: "Q1".into(),
            form: ContactForm::No,
            pins: String::new(),
            from: String::new(),
            free: false,
            series: String::new(),
        };
        set_series(&mut contact, "K1");
        assert_eq!((contact.series.as_str(), contact.bind.as_str()), ("K1", ""));
        // Back to an input of the board: the coil is let go, the input is chosen again by hand.
        set_series(&mut contact, "");
        assert_eq!(contact.series, "");
    }

    #[test]
    fn only_breakers_rcds_contactors_and_relays_have_contacts() {
        for kind in DeviceKind::ALL {
            let expected = matches!(
                kind,
                DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Contactor | DeviceKind::Relay
            );
            assert_eq!(can_have_contacts(kind), expected, "{kind:?}");
        }
        assert!(!can_have_contacts(DeviceKind::Aux));
    }
}
