//! Mode «Щит»: the electrical panel of the project (`specs/electrical-schematics`, task 9).
//!
//! Left the tree of the devices, in the middle the preview of the sheets of the documentation,
//! right the properties of the selected device and the problems of the panel. The sheets are the
//! same drawing that goes into the PDF (`crab_schematic::Op`), shown without making a PDF.

use std::path::PathBuf;

use crab_core::{check_panel, panel_binding, panel_tags, Device, DeviceKind, Report, Severity};
use crab_schematic::{build_document_in, device_kind_name, Align, Document, Format, Lang, Op, Rgb};
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
    ("Защита", &[DeviceKind::Breaker, DeviceKind::Rcd]),
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

/// State of the mode «Щит».
pub struct PanelView {
    /// Designation of the selected device.
    pub selected: Option<String>,
    /// Number of the sheet shown (from 0).
    sheet: usize,
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
}

impl Default for PanelView {
    fn default() -> Self {
        Self {
            selected: None,
            sheet: 0,
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
    Add(DeviceKind),
    Remove(String),
    Find(String, Place),
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
                let resp = ui.selectable_label(
                    selected.as_deref() == Some(device.id.as_str()),
                    row_label(ui, caption, warn, None),
                );
                if resp.clicked() {
                    *action.borrow_mut() = Some(Action::Select(device.id.clone()));
                }
                if resp.double_clicked() {
                    *action.borrow_mut() = Some(Action::Select(device.id.clone()));
                }
                resp.context_menu(|ui| {
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
        Action::Select(id) => {
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

/// The properties of the selected device and the problems of the panel (right panel).
pub fn properties(app: &mut EditorApp, ui: &mut Ui) {
    refresh(app);
    let Some(doc) = &app.doc else {
        ui.weak(tr("Проект не открыт"));
        return;
    };
    let selected = app
        .panel_view
        .selected
        .clone()
        .filter(|id| doc.project.panel.device(id).is_some());
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

            ui.label(tr("Вид"));
            let mut kind = device.kind;
            egui::ComboBox::from_id_salt("panel_kind")
                .selected_text(kind_title(kind))
                .show_ui(ui, |ui| {
                    for k in DeviceKind::ALL {
                        ui.selectable_value(&mut kind, k, kind_title(k));
                    }
                });
            if kind != device.kind {
                edit.kind = Some(kind);
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

            ui.label(tr("Артикул"));
            let mut part = device.part.clone();
            ui.add(egui::TextEdit::singleline(&mut part).desired_width(160.0));
            if part != device.part {
                edit.part = Some(part);
            }
            ui.end_row();

            if panel_binding(device.kind).is_some() {
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

            ui.label(tr("Питает или коммутирует"));
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
        });
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

/// Changes asked in the property fields during one frame.
#[derive(Default)]
struct Edit {
    rename: Option<String>,
    kind: Option<DeviceKind>,
    rating: Option<String>,
    part: Option<String>,
    bind: Option<String>,
    load: Option<String>,
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
    if let Some(kind) = edit.kind {
        field(doc, "kind", &|d| d.kind = kind);
    }
    if let Some(rating) = edit.rating {
        field(doc, "rating", &|d| d.rating = rating.clone());
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
    if let Some(new) = edit.rename {
        match doc.edit_panel(None, |p| p.rename(id, &new)) {
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
    if resp.dragged() {
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
    if resp.double_clicked() {
        view_state.zoom = 1.0;
        view_state.pan = Vec2::ZERO;
        view_state.highlight = None;
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

/// Above the sheet: the sheets, the language and the format.
fn toolbar(app: &mut EditorApp, ui: &mut Ui) {
    let Some(cache) = &app.panel_view.cache else {
        return;
    };
    let titles: Vec<String> = cache.document.as_ref().map_or_else(
        |_| Vec::new(),
        |d| d.sheets.iter().map(|s| s.title.clone()).collect(),
    );
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
}
