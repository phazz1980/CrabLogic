//! Mode «Щит»: the electrical panel of the project (`specs/electrical-schematics`, task 9).
//!
//! Left the tree of the devices, in the middle the preview of the sheets of the documentation,
//! right the properties of the selected device and the problems of the panel. The sheets are the
//! same drawing that goes into the PDF (`crab_schematic::Op`), shown without making a PDF.

use std::path::PathBuf;

use crab_core::{check_panel, DeviceKind, Report, SheetId};
use crab_schematic::{build_document_in, device_kind_name, Document, Format, Lang};
use eframe::egui::{Ui, Vec2};

use crate::app::EditorApp;
use crate::i18n::tr;

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
    ("Реле", &[DeviceKind::Relay, DeviceKind::Contactor]),
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

/// An entry of the palette dragged onto a sheet: a new device of this kind (or its coil, or a
/// contact of a relay) is made where it is dropped.
#[derive(Clone, Copy)]
struct PalettePayload {
    kind: DeviceKind,
    part: crab_schematic::PalettePart,
}

/// The entries of the palette of a kind: the part and its title. A contactor is the device and its
/// coil; a relay is a coil and its contacts (`specs/panel-palette`).
fn palette_entries(kind: DeviceKind) -> Vec<(crab_schematic::PalettePart, String)> {
    use crab_schematic::PalettePart::{Coil, Contact, Whole};
    let t = |s: &str| tr(s).to_owned();
    match kind {
        DeviceKind::Contactor => vec![(Whole, t("Контактор"))],
        DeviceKind::Relay => vec![(Coil, t("Катушка")), (Contact, t("Контакт"))],
        _ => vec![(Whole, kind_title(kind))],
    }
}

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
    /// A coil moved to another sheet: its designation and the one the sheet gives it. Offered in
    /// the sheet controls; the rename happens only when the user asks for it.
    rename_offer: Option<(String, String, u64)>,
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
            rename_offer: None,
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

mod canvas;
mod properties;
mod tree;

#[cfg(test)]
mod tests;

pub use canvas::sheet;
pub use properties::properties;
pub use tree::tree;
