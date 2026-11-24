//! Electrical documentation of a project's panel (`specs/electrical-schematics`).
//!
//! The panel (`panel.toml`) lists devices; here the wires between them are worked out and
//! numbered ([`nets`]), laid out on A3 (or A4) sheets ([`power`]) and written as Typst markup ([`doc`]),
//! which `crab-pdf` turns into a PDF.

mod bus;
mod canvas;
pub mod control;
pub mod doc;
mod error;
mod free;
mod lang;
pub mod nets;
pub mod power;
mod preview;
mod route;
pub mod scene;
mod sheet_names;
mod symbols;
pub mod tables;
pub mod title;
pub mod xref;
pub use xref::find_ref;

use crab_core::{Library, Project};

pub use canvas::{Align, Op, Rgb, EARTH, INK, LOW, WEAK};
pub use doc::{Document, Format, Page, Sheet};
pub use error::Error;
pub use lang::Lang;
pub use nets::{build_nets, BoardTerminal, Nets, Pin, Wire, WireClass, BOARD, EXTERNAL};
pub use preview::{preview, PalettePart, Preview};
pub use scene::{Dir, End, Item, ItemKind, Port};
pub use sheet_names::sheet_designations;
pub use tables::{device_kind_name, Table, Tables};
pub use xref::{TerminalXrefs, Xref, Xrefs};

/// Document number of the first drawing sheet: the title sheet is number 1.
const FIRST_DRAWING_SHEET: usize = 2;

/// The documents of a project: the sheets and the wires with their numbers. Run
/// [`crab_core::check_panel`] first. The numbers are not stored in the project: call
/// [`Nets::store`] and save it to keep them.
pub fn build_document(
    project: &Project,
    lib: &Library,
    lang: Lang,
) -> Result<(Document, Nets), Error> {
    build_document_in(project, lib, lang, Format::default())
}

/// [`build_document`] on sheets of the given format.
///
/// The drawing sheets follow the title sheet in this order: the power sheets, the control sheets,
/// the free sheets. A device put by hand keeps its sheet name (`SheetId`) whatever the number of
/// the sheets of the other kinds is.
pub fn build_document_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<(Document, Nets), Error> {
    let page = Page::new(format);
    let nets = build_nets(project, lib);
    let panel = &project.panel;
    let layout = &project.panel_layout;
    // Three steps. The first pass places the devices (the number of sheets and the zones do not
    // depend on the captions or on the coils) and collects the wires the generator draws: the
    // automatic wires that no sheet draws are drawn the general way only in the last pass, so
    // they are left out here (`general` is `None`). The second pass writes the references to the
    // places of the other part; the last one draws the general wires too.
    let none = Xrefs::new();
    let no_terminals = TerminalXrefs::new();
    let (power_first, power_at, hand_power, power_terminals) = power::power_sheets(
        panel,
        layout,
        &nets,
        lang,
        FIRST_DRAWING_SHEET,
        &none,
        &no_terminals,
        None,
        &page,
    )?;
    let first_control = FIRST_DRAWING_SHEET + power_first.len();
    let (control_first, control_coils, control_terminals) = control::control_sheets(
        project,
        &nets,
        lang,
        first_control,
        &power_at,
        &none,
        &no_terminals,
        None,
        &page,
    );
    let first_free = first_control + control_first.len();
    let free_count = free::count(project);
    let (free_first, hand_free, free_terminals) = free::free_sheets(
        project,
        &nets,
        lang,
        first_free,
        free_count,
        &power_at,
        &none,
        &no_terminals,
        None,
        &page,
    );
    let drawn = control::drawn_wires(power_first.iter().chain(&control_first).chain(&free_first));
    let coil_at = merge_coils(control_coils, [hand_power, hand_free]);
    let terminal_at = merge_terminals([power_terminals, control_terminals, free_terminals]);
    let (power, _, _, _) = power::power_sheets(
        panel,
        layout,
        &nets,
        lang,
        FIRST_DRAWING_SHEET,
        &coil_at,
        &terminal_at,
        Some(&drawn),
        &page,
    )?;
    let (control, _, _) = control::control_sheets(
        project,
        &nets,
        lang,
        first_control,
        &power_at,
        &coil_at,
        &terminal_at,
        Some(&drawn),
        &page,
    );
    let (free, _, _) = free::free_sheets(
        project,
        &nets,
        lang,
        first_free,
        free_count,
        &power_at,
        &coil_at,
        &terminal_at,
        Some(&drawn),
        &page,
    );
    let mut sheets = power;
    sheets.extend(control);
    sheets.extend(free);
    // The tables come last; the sheets are only for the panel that has something to draw.
    let tables = tables::build_tables(project, &nets, lang);
    if !sheets.is_empty() {
        for table in [&tables.terminals, &tables.wires, &tables.bom] {
            sheets.extend(tables::table_sheets(table, &page));
        }
    }
    if !sheets.is_empty() {
        let titles: Vec<&str> = sheets.iter().map(|s| s.title.as_str()).collect();
        let title = title::title_sheet(project, &nets, &titles, lang, &page);
        sheets.insert(0, title);
    }
    let doc = Document {
        name: project.display_name(),
        page,
        sheets,
        power_at,
        coil_at,
        terminal_at,
        tables,
    };
    Ok((doc, nets))
}

/// The places of the terminals of all the sheets: the first sheet that draws a terminal keeps it,
/// the sheets are given in the order of the document.
fn merge_terminals(maps: [TerminalXrefs; 3]) -> TerminalXrefs {
    let mut all = TerminalXrefs::new();
    for map in maps {
        for (terminal, at) in map {
            all.entry(terminal).or_insert(at);
        }
    }
    all
}

/// The places of the coils and auxiliary contacts: the ones drawn on the control sheets, then the
/// ones put by hand on the power and the free sheets. A device whose place is known from the
/// control sheets keeps it.
fn merge_coils(mut coils: Xrefs, hands: [Xrefs; 2]) -> Xrefs {
    for hand in hands {
        for (device, at) in hand {
            coils.entry(device).or_insert(at);
        }
    }
    coils
}

/// The Typst markup of the documentation of a project.
pub fn source(project: &Project, lib: &Library, lang: Lang) -> Result<String, Error> {
    source_in(project, lib, lang, Format::default())
}

/// [`source`] on sheets of the given format.
pub fn source_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<String, Error> {
    let (doc, _) = build_document_in(project, lib, lang, format)?;
    Ok(doc::typst_source(&doc, lang))
}

/// The PDF of the documentation of a project (needs the `crablogic-pdf` helper).
pub fn render_pdf(project: &Project, lib: &Library, lang: Lang) -> Result<Vec<u8>, Error> {
    render_pdf_in(project, lib, lang, Format::default())
}

/// [`render_pdf`] on sheets of the given format.
pub fn render_pdf_in(
    project: &Project,
    lib: &Library,
    lang: Lang,
    format: Format,
) -> Result<Vec<u8>, Error> {
    Ok(crab_pdf::compile(source_in(project, lib, lang, format)?)?)
}
